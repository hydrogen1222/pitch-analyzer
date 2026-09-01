// ASS 导出: 歌词 + 字级音高 + karaoke timing，可直接用 ffmpeg/libass 烧录
//
// 布局 (PlayResX=1280, PlayResY=720):
//   E4      F#4      G4            ← Pitch 行 (每个字上方, \pos 定位)
//   我 想 飞                          ← Lyric 行 (整行一句, \k 逐字点亮)
//
// Karaoke: 整行一条 Dialogue, 用 \k 逐字点亮 (唱到哪个字哪个字变色);
//          音高行按字单独定位在对应字上方, x 坐标按等宽估算对齐。
//          不做字幕编辑器，只做最小化 exporter。

use crate::export::{presentation_segments, PresentationSegment};
use crate::models::{LyricLine, LyricToken};
use std::io::Write;
use std::path::Path;

const PLAY_RES_X: u32 = 1280;
const PLAY_RES_Y: u32 = 720;
const LYRIC_MARGIN_V: u32 = 60;
const PITCH_Y: f32 = 510.0;
const READING_Y: f32 = 570.0;

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
    writeln!(f).map_err(|e| e.to_string())?;
    writeln!(f, "[V4+ Styles]").map_err(|e| e.to_string())?;
    writeln!(f, "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")
        .map_err(|e| e.to_string())?;
    // PrimaryColour = 已唱 (白), SecondaryColour = 未唱 (青绿) → \k 点亮效果
    // Alignment: Pitch=5(中部居中锚点, 配合 \pos), Lyric=2(底部居中)
    writeln!(
        f,
        "Style: Pitch,Noto Sans CJK SC,{},&H00FFFFFF,&H00B4FF7D,&H00101010,&H80000000,-1,0,0,0,100,100,0,0,1,2,1,5,40,40,30,1",
        pitch_font
    )
    .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "Style: Lyric,Noto Sans CJK SC,{},&H00FFFFFF,&H00B4FF7D,&H00101010,&H80000000,-1,0,0,0,100,100,0,0,1,2,1,2,40,40,{},1",
        lyric_font, LYRIC_MARGIN_V
    )
    .map_err(|e| e.to_string())?;
    writeln!(
        f,
        "Style: Reading,Noto Sans CJK JP,{},&H00FFFFFF,&H00B4FF7D,&H00101010,&H80000000,-1,0,0,0,100,100,0,0,1,2,1,5,40,40,30,1",
        lyric_font
    )
    .map_err(|e| e.to_string())?;
    writeln!(f).map_err(|e| e.to_string())?;
    writeln!(f, "[Events]").map_err(|e| e.to_string())?;
    writeln!(
        f,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    )
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

        let segments = presentation_segments(line);
        if segments.is_empty() {
            continue;
        }

        // ── 音高行: 每个显示组一条 Dialogue, \pos 到对应组中心上方 ──
        // 多个 Dialogue 仍位于同一条视觉音高行；这样既保持歌词块对齐，
        // 又能让一个组内的转音以 `D4-C4` 作为完整标签显示。
        let centers = estimate_token_centers(&tokens, lyric_font);
        let timed_indices: Vec<usize> = line
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(index, token)| token.start_time.zip(token.end_time).map(|_| index))
            .collect();
        let phonetic_layout = segments.iter().any(|segment| segment.phonetic);
        let phonetic_centers = phonetic_layout.then(|| {
            estimate_text_centers(
                &segments
                    .iter()
                    .map(|segment| segment.display_text.as_str())
                    .collect::<Vec<_>>(),
                lyric_font,
            )
        });
        for (segment_index, segment) in segments.iter().enumerate() {
            if let Some(name) = &segment.note_text {
                let center = phonetic_centers
                    .as_ref()
                    .and_then(|positions| positions.get(segment_index).copied())
                    .unwrap_or_else(|| {
                        estimate_segment_center(
                            segment,
                            &timed_indices,
                            &tokens,
                            &centers,
                            lyric_font,
                        )
                    });
                let pitch_dialog = format!("{{\\pos({:.0},{})}}{}", center, PITCH_Y, name);
                writeln!(
                    f,
                    "Dialogue: 0,{},{},Pitch,,0,0,0,,{}",
                    to_ass_time(segment.start_time),
                    to_ass_time(segment.end_time),
                    pitch_dialog
                )
                .map_err(|e| e.to_string())?;
            }
        }

        // ── 歌词行: 整行一条 Dialogue, \k 逐字点亮 ──
        let line_start = segments
            .iter()
            .map(|segment| segment.start_time)
            .fold(f32::INFINITY, f32::min);
        let line_end = segments
            .iter()
            .map(|segment| segment.end_time)
            .fold(f32::NEG_INFINITY, f32::max);

        // Explicit ruby gets its own kana row. Pitch centers above are based on
        // this row, while the original kanji source remains a separate line.
        if phonetic_layout {
            let mut reading_text = String::new();
            let mut previous_start = line_start;
            for segment in &segments {
                let cs = (((segment.start_time - previous_start) * 100.0).round() as i64).max(0);
                reading_text.push_str(&format!(
                    "{{\\k{}}}{}",
                    cs,
                    escape_ass(&segment.display_text)
                ));
                previous_start = segment.start_time;
            }
            writeln!(
                f,
                "Dialogue: 0,{},{},Reading,,0,0,0,,{{\\pos({:.0},{:.0})}}{}",
                to_ass_time(line_start),
                to_ass_time(line_end),
                PLAY_RES_X as f32 / 2.0,
                READING_Y,
                reading_text
            )
            .map_err(|e| e.to_string())?;
        }

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

fn estimate_text_centers(texts: &[&str], font_size: u32) -> Vec<f32> {
    let mut widths: Vec<f32> = texts
        .iter()
        .map(|text| token_width(text, font_size).max(font_size as f32 * 0.9))
        .collect();
    let max_width = PLAY_RES_X as f32 - 80.0;
    let total: f32 = widths.iter().sum();
    if total > max_width {
        let scale = max_width / total;
        for width in &mut widths {
            *width *= scale;
        }
    }
    let fitted_total: f32 = widths.iter().sum();
    let mut x = ((PLAY_RES_X as f32 - fitted_total) / 2.0).max(40.0);
    widths
        .into_iter()
        .map(|width| {
            let center = x + width / 2.0;
            x += width;
            center
        })
        .collect()
}

fn estimate_segment_center(
    segment: &PresentationSegment,
    timed_indices: &[usize],
    timed_tokens: &[&LyricToken],
    centers: &[f32],
    lyric_font: u32,
) -> f32 {
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for &token_index in &segment.token_indices {
        let Some(position) = timed_indices.iter().position(|&index| index == token_index) else {
            continue;
        };
        let width = token_width(
            timed_tokens[position]
                .text
                .split('|')
                .next()
                .unwrap_or(&timed_tokens[position].text),
            lyric_font,
        );
        left = left.min(centers[position] - width / 2.0);
        right = right.max(centers[position] + width / 2.0);
    }
    if left.is_finite() && right.is_finite() {
        (left + right) / 2.0
    } else {
        PLAY_RES_X as f32 / 2.0
    }
}

/// 估算每个 token 在底部居中行内的 x 中心坐标。
/// CJK/全角字符按 1.0 倍字号, 其他 (拉丁/数字/半角标点) 按 0.55 倍估算。
fn estimate_token_centers(tokens: &[&LyricToken], lyric_font: u32) -> Vec<f32> {
    let total: f32 = tokens
        .iter()
        .map(|t| token_width(t.text.split('|').next().unwrap_or(&t.text), lyric_font))
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
            let w = token_width(text, lyric_font);
            let center = x + w / 2.0;
            x += w;
            center
        })
        .collect()
}

fn token_width(text: &str, lyric_font: u32) -> f32 {
    text.chars()
        .map(|ch| {
            let is_wide = (ch as u32) >= 0x2E80 || ('\u{FF00}'..='\u{FFEF}').contains(&ch);
            if is_wide {
                lyric_font as f32
            } else {
                lyric_font as f32 * 0.55
            }
        })
        .sum()
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
