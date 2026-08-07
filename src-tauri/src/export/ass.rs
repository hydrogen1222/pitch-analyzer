// ASS 导出: 歌词 + 字级音高 + karaoke timing，可直接用 ffmpeg/libass 烧录
//
// 布局 (PlayResX=1280, PlayResY=720):
//       E4      F#4      G4        ← Pitch 行 (每个字上方居中)
//       我       想       飞         ← Lyric 行 (底部)
//
// 不做字幕编辑器，只做最小化 exporter。

use crate::lyrics::select_primary_note;
use crate::models::{midi_to_note_name, LyricLine};
use std::io::Write;
use std::path::Path;

const PLAY_RES_X: u32 = 1280;
const PLAY_RES_Y: u32 = 720;
const LYRIC_Y: f32 = 560.0;
const PITCH_Y: f32 = 495.0;

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
    // Alignment: Pitch=8(上方居中), Lyric=5(中间居中)
    writeln!(
        f,
        "Style: Pitch,Noto Sans CJK SC,{},{},{},{},{},-1,0,0,0,100,100,0,0,1,2,1,8,40,40,30,1",
        pitch_font, "&H00FFFFFF", "&H00B4FF7D", "&H00101010", "&H80000000"
    )
    .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "Style: Lyric,Noto Sans CJK SC,{},{},{},{},{},-1,0,0,0,100,100,0,0,1,2,1,5,40,40,30,1",
        lyric_font, "&H00FFFFFF", "&H00B4FF7D", "&H00101010", "&H80000000"
    )
    .map_err(|e| e.to_string())?;
    writeln!(f, "").map_err(|e| e.to_string())?;
    writeln!(f, "[Events]").map_err(|e| e.to_string())?;
    writeln!(f, "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text")
        .map_err(|e| e.to_string())?;

    for line in lyrics {
        let tokens: Vec<&crate::models::LyricToken> = line
            .tokens
            .iter()
            .filter(|t| t.start_time.is_some() && t.end_time.is_some())
            .collect();
        if tokens.is_empty() {
            continue;
        }
        let n = tokens.len() as f32;
        let slot_w = (PLAY_RES_X as f32 - 80.0) / n;
        for (i, token) in tokens.iter().enumerate() {
            let s = token.start_time.unwrap();
            let e = token.end_time.unwrap();
            let x = 40.0 + slot_w * (i as f32 + 0.5);
            let text = token.text.split('|').next().unwrap_or(&token.text);
            let dur_cs = (((e - s) * 100.0).round() as u32).max(1);

            // 歌词行: karaoke timing (\k<cs> 为单字点亮)
            let lyric_dialog = format!(
                "{{\\pos({:.0},{})}}{{\\k{}}}{}",
                x,
                LYRIC_Y,
                dur_cs,
                escape_ass(text)
            );
            writeln!(
                f,
                "Dialogue: 0,{},{},Lyric,,0,0,0,,{}",
                to_ass_time(s),
                to_ass_time(e),
                lyric_dialog
            )
            .map_err(|e| e.to_string())?;

            // 音高行: 位于对应歌词上方，使用 primary_note
            if let Some(note) = token
                .primary_note
                .clone()
                .or_else(|| select_primary_note(&token.pitch_notes, 0.0, 0.0))
            {
                let name = midi_to_note_name(note.median_midi);
                if name != "---" {
                    let pitch_dialog = format!("{{\\pos({:.0},{})}}{}", x, PITCH_Y, name);
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
    }
    Ok(())
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
