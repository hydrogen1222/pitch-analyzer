// SRT 导出: 每句源歌词一个 cue，音符行在上、歌词行在下。

use crate::export::{phonetic_line_text, presentation_segments, source_line_text};
use crate::models::{midi_to_note_name, LyricLine, PitchTrack};
use std::io::Write;
use std::path::Path;

pub fn export_srt(
    pitch_track: &PitchTrack,
    lyrics: &[LyricLine],
    path: &Path,
) -> Result<(), String> {
    let to_srt_time = |sec: f32| -> String {
        // 四舍五入到毫秒, 避免截断误差累积; 总毫秒数统一进位防止 999.9ms 溢出
        let total_ms = (sec.max(0.0) * 1000.0).round() as u64;
        let hrs = total_ms / 3_600_000;
        let mins = (total_ms % 3_600_000) / 60_000;
        let secs = (total_ms % 60_000) / 1000;
        let ms = total_ms % 1000;
        format!("{:02}:{:02}:{:02},{:03}", hrs, mins, secs, ms)
    };

    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut idx = 1u32;

    if !lyrics.is_empty() {
        let mut timed_lines = 0usize;
        for line in lyrics {
            let segments = presentation_segments(line);
            if segments.is_empty() {
                continue;
            }
            timed_lines += 1;

            let line_start = segments
                .iter()
                .map(|segment| segment.start_time)
                .fold(f32::INFINITY, f32::min);
            let line_end = segments
                .iter()
                .map(|segment| segment.end_time)
                .fold(f32::NEG_INFINITY, f32::max);
            let pitch_row = segments
                .iter()
                .map(|segment| segment.note_text.as_deref().unwrap_or("---"))
                .collect::<Vec<_>>()
                .join(" ");
            let source = source_line_text(line);
            let reading = phonetic_line_text(line, &segments);
            let mut rows = Vec::new();
            if !pitch_row.is_empty() {
                rows.push(pitch_row);
            }
            if let Some(reading) = reading.filter(|reading| !reading.is_empty()) {
                rows.push(reading);
            }
            if !source.is_empty() {
                rows.push(source);
            }
            let text = rows.join("\n");

            writeln!(f, "{}", idx).map_err(|e| e.to_string())?;
            writeln!(
                f,
                "{} --> {}",
                to_srt_time(line_start),
                to_srt_time(line_end)
            )
            .map_err(|e| e.to_string())?;
            writeln!(f, "{}\n", text).map_err(|e| e.to_string())?;
            idx += 1;
        }
        if timed_lines == 0 {
            return Err(
                "歌词没有逐句时间信息 (TXT 歌词需要在导入音频后使用 LRC 才能对齐时间)，无法导出 SRT"
                    .to_string(),
            );
        }
    } else if let Some(&max_time) = pitch_track.times.last() {
        let interval = 0.5f32;
        let mut t = 0.0f32;
        while t < max_time {
            let i = match pitch_track
                .times
                .binary_search_by(|probe| probe.partial_cmp(&t).unwrap())
            {
                Ok(i) => i,
                Err(i) => i.min(pitch_track.midis.len().saturating_sub(1)),
            };
            let midi = pitch_track.midis[i];
            let mut display = midi_to_note_name(midi);
            if !midi.is_nan() {
                display.push_str(&format!(" ({:.2})", midi));
            }
            writeln!(f, "{}", idx).map_err(|e| e.to_string())?;
            writeln!(f, "{} --> {}", to_srt_time(t), to_srt_time(t + interval))
                .map_err(|e| e.to_string())?;
            writeln!(f, "{}\n", display).map_err(|e| e.to_string())?;
            idx += 1;
            t += interval;
        }
    }
    Ok(())
}
