// SRT 导出: 简单输出，每字使用 primary_note (禁止直接取第一个音)

use crate::lyrics::select_primary_note;
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
        let mut timed_tokens = 0usize;
        for line in lyrics {
            for token in &line.tokens {
                let (t_start, t_end) = match (token.start_time, token.end_time) {
                    (Some(s), Some(e)) => (s, e),
                    _ => continue,
                };
                timed_tokens += 1;
                let text = if token.text.contains('|') {
                    token.text.split('|').next().unwrap_or(&token.text)
                } else {
                    &token.text
                };
                // 使用 primary_note；旧工程无 primary_note 时按评分回退
                let note_display = token
                    .primary_note
                    .clone()
                    .or_else(|| select_primary_note(&token.pitch_notes, 0.0, 0.0))
                    .map(|note| format!(" [{}]", midi_to_note_name(note.median_midi)))
                    .unwrap_or_default();
                writeln!(f, "{}", idx).map_err(|e| e.to_string())?;
                writeln!(f, "{} --> {}", to_srt_time(t_start), to_srt_time(t_end))
                    .map_err(|e| e.to_string())?;
                writeln!(f, "{}{}\n", text, note_display).map_err(|e| e.to_string())?;
                idx += 1;
            }
        }
        if timed_tokens == 0 {
            return Err(
                "歌词没有逐字时间信息 (TXT 歌词需要在导入音频后使用 LRC 才能对齐时间)，无法导出 SRT"
                    .to_string(),
            );
        }
    } else if let Some(&max_time) = pitch_track.times.last() {
        let interval = 0.5f32;
        let mut t = 0.0f32;
        while t < max_time {
            let i = match pitch_track.times.binary_search_by(|probe| probe.partial_cmp(&t).unwrap()) {
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
