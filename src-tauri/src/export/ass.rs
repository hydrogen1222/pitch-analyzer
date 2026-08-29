// ASS 导出: 歌词 + 字级音高 + karaoke timing，可直接用 ffmpeg/libass 烧录
//
// 布局 (PlayResX=1280, PlayResY=720):
//   E4      F#4      G4            ← Pitch 行 (每个字上方, \pos 定位)
//   我 想 飞                          ← Lyric 行 (整行一句, \k 逐字点亮)
//
// Karaoke: 整行一条 Dialogue, 用 \k 逐字点亮 (唱到哪个字哪个字变色);
//          音高行按字单独定位在对应字上方, x 坐标按等宽估算对齐。
//          不做字幕编辑器，只做最小化 exporter。

use crate::lyrics::select_primary_note;
use crate::models::{midi_to_note_name, LyricLine, LyricToken};
use std::io::Write;
use std::path::Path;

const PLAY_RES_X: u32 = 1280;
const PLAY_RES_Y: u32 = 720;
const LYRIC_MARGIN_V: u32 = 60;
const PITCH_Y: f32 = 540.0;

/// 导出 ASS 字幕。pitch_font_size / lyric_font_size 为基础字号。
pub fn export_ass(
    lyrics: &[LyricLine],
    path: &Path,
    pitch_font_size: u32,
    lyric_font_size: u32,
) -> Result<(), String> {
    if lyrics.is_empty() {
        return Err("没有歌词，无法导出 ASS 字幕（请先导入 LRC/TXT 歌词）".to_string());
    }
    let pitch_font = pitch_font_size.max(14);
    let lyric_font = lyric_font_size.max(12);

    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;

    writeln!(f, "[Script Info]").map_err(|e| e.to_string())?;
    writeln!(f, "ScriptType: v4.00+").map_err(|e| e.to_string())?;
    writeln!(f, "PlayResX: {}", PLAY_RES_X).map_err(|e| e.to_string())?;
    writeln!(f, "PlayResY: {}", PLAY_RES_Y).map_err(|e| e.to_string())?;
    writeln!(f, "WrapStyle: 2").map_err(|e| e.to_string())?;
    writeln!(f, "ScaledBorderAndShadow: yes").map_err(|e| e.to_string())?;
    writeln!(f, "").map_err(|e| e.to_string())?;
    writeln!(f, "[V4+ Styles]").map_err(|e| e.to_string())?;
    writeln!(f, "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")
        .map_err(|e| e.to_string())?;
    // PrimaryColour = 已唱 (白), SecondaryColour = 未唱 (青绿) → \k 点亮效果
    // Alignment: Pitch=5(中部居中锚点, 配合 \pos), Lyric=2(底部居中)
    writeln!(
        f,
        "Style: Pitch,Noto Sans CJK SC,{},{},{},{},{},-1,0,0,0,100,100,0,0,1,2,1,5,40,40,30,1",
        pitch_font, "&H00FFFFFF", "&H00B4FF7D", "&H00101010", "&H80000000"
    )
    .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "Style: Lyric,Noto Sans CJK SC,{},{},{},{},{},-1,0,0,0,100,100,0,0,1,2,1,2,40,40,{},1",
        lyric_font, "&H00FFFFFF", "&H00B4FF7D", "&H00101010", "&H80000000", LYRIC_MARGIN_V
    )
    .map_err(|e| e.to_string())?;
    writeln!(f, "").map_err(|e| e.to_string())?;
    writeln!(f, "[Events]").map_err(|e| e.to_string())?;
    writeln!(f, "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text")
        .map_err(|e| e.to_string())?;

    for line in lyrics {
        let tokens: Vec<&LyricToken> = line
            .tokens
            .iter()
            .filter(|t| t.start_time.is_some() && t.end_time.is_some())
            .collect();
        if tokens.is_empty() {
            continue;
        }

        // ── 音高行: 每字一条 Dialogue, \pos 到估算的字中心上方 ──
        let centers = estimate_token_centers(&tokens, lyric_font);
        for (i, token) in tokens.iter().enumerate() {
            let s = token.start_time.unwrap();
            let e = token.end_time.unwrap();
            if let Some(note) = token
                .primary_note
                .clone()
                .or_else(|| select_primary_note(&token.pitch_notes, 0.0, 0.0))
            {
                let name = midi_to_note_name(note.median_midi);
                if name != "---" {
                    let pitch_dialog = format!("{{\\pos({:.0},{})}}{}", centers[i], PITCH_Y, name);
                    writeln!(
                        f,
                        "Dialogue: 0,{},{},Pitch,,0,0,0,,{}",
                        to_ass_time(s),
                        to_ass_time(e),
                        pitch_dialog
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }

        // ── 歌词行: 整行一条 Dialogue, \k 逐字点亮 ──
        let line_start = tokens[0].start_time.unwrap();
        let line_end = tokens.last().unwrap().end_time.unwrap();
        let mut karaoke_text = String::new();
        let mut prev_start = line_start;
        for token in &tokens {
            let s = token.start_time.unwrap();
            // \k 为相邻两次点亮之间的时长 → 高亮精确发生在该字开始演唱的时刻
            let cs = (((s - prev_start) * 100.0).round() as i64).max(0);
            let text = token.text.split('|').next().unwrap_or(&token.text);
            karaoke_text.push_str(&format!("{{\\k{}}}{}", cs, escape_ass(text)));
            prev_start = s;
        }
        writeln!(
            f,
            "Dialogue: 0,{},{},Lyric,,0,0,0,,{}",
            to_ass_time(line_start),
            to_ass_time(line_end),
            karaoke_text
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 估算每个 token 在底部居中行内的 x 中心坐标。
/// CJK/全角字符按 1.0 倍字号, 其他 (拉丁/数字/半角标点) 按 0.55 倍估算。
fn estimate_token_centers(tokens: &[&LyricToken], lyric_font: u32) -> Vec<f32> {
    let char_w = |ch: char| -> f32 {
        let is_wide = (ch as u32) >= 0x2E80 || ('\u{FF00}'..='\u{FFEF}').contains(&ch);
        if is_wide {
            lyric_font as f32
        } else {
            lyric_font as f32 * 0.55
        }
    };
    let token_w = |t: &str| -> f32 { t.chars().map(char_w).sum() };

    let total: f32 = tokens
        .iter()
        .map(|t| token_w(t.text.split('|').next().unwrap_or(&t.text)))
        .sum();
    let max_width = (PLAY_RES_X as f32) - 80.0;
    let mut x = if total > max_width {
        40.0
    } else {
        ((PLAY_RES_X as f32) - total) / 2.0
    };

    tokens
        .iter()
        .map(|t| {
            let text = t.text.split('|').next().unwrap_or(&t.text);
            let w = token_w(text);
            let center = x + w / 2.0;
            x += w;
            center
        })
        .collect()
}

/// ASS 时间: H:MM:SS.cc
fn to_ass_time(sec: f32) -> String {
    let cs = ((sec * 100.0).round() as i64).max(0);
    let h = cs / 360000;
    let m = (cs % 360000) / 6000;
    let s = (cs % 6000) / 100;
    let c = cs % 100;
    format!("{}:{:02}:{:02}.{:02}", h, m, s, c)
}

/// 转义 ASS 特殊字符
fn escape_ass(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out
}
