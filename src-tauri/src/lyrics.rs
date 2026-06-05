// 歌词解析: LRC + TXT, tokenizer, aligner, SRT 导出

use crate::models::{LyricLine, LyricToken, PitchNote, PitchTrack};
use regex::Regex;
use std::path::Path;

// ── Tokenizer ──────────────────────────────────────────────

pub fn tokenize(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let re = Regex::new(
        r"(?x)
        [a-zA-Z0-9]+(?:['\-][a-zA-Z0-9]+)*  # English/Latin words
        |[\x{4e00}-\x{9fff}]                 # Chinese Hanzi
        |[\x{3040}-\x{309f}]                 # Japanese Hiragana
        |[\x{30a0}-\x{30ff}]                 # Japanese Katakana
        |[^\s]                               # Fallback
        "
    )
    .expect("Invalid tokenizer regex pattern");

    let non_word_re = Regex::new(r"^[^\w\x{4e00}-\x{9fff}\x{3040}-\x{309f}\x{30a0}-\x{30ff}]+$")
        .expect("Invalid non-word regex pattern");

    let raw: Vec<&str> = re.find_iter(text).map(|m| m.as_str()).collect();
    let mut merged: Vec<String> = Vec::new();
    for token in raw {
        if non_word_re.is_match(token) && !merged.is_empty() {
            let last = merged.last_mut().unwrap();
            last.push_str(token);
        } else {
            merged.push(token.to_string());
        }
    }
    merged
}

// ── TXT Parser ─────────────────────────────────────────────

pub fn parse_txt(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for line_str in text.lines() {
        let trimmed = line_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        let token_strs = tokenize(trimmed);
        let tokens = token_strs
            .iter()
            .map(|t| LyricToken {
                text: t.clone(),
                start_time: None,
                end_time: None,
                pitch_notes: Vec::new(),
            })
            .collect();
        lines.push(LyricLine {
            text: trimmed.to_string(),
            start_time: None,
            end_time: None,
            tokens,
            primary_text: trimmed.to_string(),
            translations: Vec::new(),
        });
    }
    lines
}

// ── LRC Parser ─────────────────────────────────────────────

pub fn parse_lrc(text: &str, audio_duration: Option<f32>) -> Vec<LyricLine> {
    let time_re = Regex::new(r"\[(\d{2}):(\d{2}\.\d{2,3})\]")
        .expect("Invalid LRC time regex pattern");

    #[derive(Debug)]
    struct RawEntry {
        start_time: f32,
        text: String,
    }

    let mut entries: Vec<RawEntry> = Vec::new();
    for line_str in text.lines() {
        let trimmed = line_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        let captures: Vec<_> = time_re.captures_iter(trimmed).collect();
        if captures.is_empty() {
            continue;
        }
        let content = time_re.replace_all(trimmed, "").trim().to_string();
        if content.is_empty() {
            continue;
        }
        for cap in &captures {
            let mins: f32 = cap[1].parse().unwrap_or(0.0);
            let secs: f32 = cap[2].parse().unwrap_or(0.0);
            let time_sec = mins * 60.0 + secs;
            entries.push(RawEntry {
                start_time: time_sec,
                text: content.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());

    // 合并双语（相同时间戳 ±50ms）
    struct MergedEntry {
        start_time: f32,
        text: String,
        translations: Vec<String>,
    }
    let mut merged: Vec<MergedEntry> = Vec::new();
    for entry in entries {
        if let Some(last) = merged.last_mut() {
            if (last.start_time - entry.start_time).abs() < 0.05 {
                last.translations.push(entry.text);
                continue;
            }
        }
        merged.push(MergedEntry {
            start_time: entry.start_time,
            text: entry.text,
            translations: Vec::new(),
        });
    }

    let mut lines = Vec::new();
    for (i, entry) in merged.iter().enumerate() {
        let start_time = entry.start_time;
        let end_time = if i + 1 < merged.len() {
            let next = merged[i + 1].start_time;
            if next <= start_time {
                start_time + 0.1
            } else {
                next
            }
        } else {
            audio_duration.unwrap_or(start_time + 0.1)
        };

        let primary_text = entry.text.clone();
        let token_strs = tokenize(&primary_text);
        let tokens = token_strs
            .iter()
            .map(|t| LyricToken {
                text: t.clone(),
                start_time: None,
                end_time: None,
                pitch_notes: Vec::new(),
            })
            .collect();

        let display_text = if entry.translations.is_empty() {
            primary_text.clone()
        } else {
            format!("{} | {}", primary_text, entry.translations.join(" / "))
        };

        lines.push(LyricLine {
            text: display_text,
            start_time: Some(start_time),
            end_time: Some(end_time),
            tokens,
            primary_text,
            translations: entry.translations.clone(),
        });
    }
    lines
}

// ── Aligner ────────────────────────────────────────────────

const MIN_NOTE_FRAMES: usize = 5;
const DOMINANT_NOTE_RATIO: f32 = 0.65;

/// 均匀分配 token 时间
pub fn distribute_token_times(lines: &mut [LyricLine]) {
    for line in lines.iter_mut() {
        let start = match line.start_time {
            Some(t) => t,
            None => continue,
        };
        let end = match line.end_time {
            Some(t) => t,
            None => continue,
        };
        if line.tokens.is_empty() {
            continue;
        }
        let duration = (end - start).max(0.1);
        let token_dur = duration / line.tokens.len() as f32;
        let mut current = start;
        for token in &mut line.tokens {
            token.start_time = Some(current);
            token.end_time = Some(current + token_dur);
            current += token_dur;
        }
    }
}

/// 绑定 pitch 到每个 token
pub fn bind_pitch_to_tokens(
    lines: &mut [LyricLine],
    pitch_track: &PitchTrack,
    confidence_threshold: f32,
) {
    if pitch_track.times.is_empty() {
        return;
    }
    for line in lines.iter_mut() {
        for token in &mut line.tokens {
            let (t_start, t_end) = match (token.start_time, token.end_time) {
                (Some(s), Some(e)) => (s, e),
                _ => continue,
            };
            // 收集落在 [t_start, t_end) 内的帧
            let mut seg_times = Vec::new();
            let mut seg_midis = Vec::new();
            let mut seg_conf = Vec::new();
            for i in 0..pitch_track.times.len() {
                let t = pitch_track.times[i];
                if t < t_start {
                    continue;
                }
                if t >= t_end {
                    break;
                }
                let m = pitch_track.midis[i];
                let c = pitch_track.confidences[i];
                if c >= confidence_threshold && m.is_finite() && !m.is_nan() {
                    seg_times.push(t);
                    seg_midis.push(m);
                    seg_conf.push(c);
                }
            }
            token.pitch_notes = segment_pitch_notes(&seg_times, &seg_midis, &seg_conf, t_start, t_end);
        }
    }
}

fn segment_pitch_notes(
    times: &[f32],
    midis: &[f32],
    confidences: &[f32],
    token_start: f32,
    token_end: f32,
) -> Vec<PitchNote> {
    if midis.len() < 2 {
        return Vec::new();
    }

    // 检查 dominant note
    let labels: Vec<i32> = midis.iter().map(|&m| m.round() as i32).collect();
    let mut label_counts: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for &l in &labels {
        *label_counts.entry(l).or_insert(0) += 1;
    }
    let (dominant_label, dominant_count) = label_counts
        .iter()
        .max_by_key(|(_, &c)| c)
        .map(|(&l, &c)| (l, c))
        .unwrap();
    let dominant_ratio = dominant_count as f32 / labels.len() as f32;

    if dominant_ratio >= DOMINANT_NOTE_RATIO {
        let core_midis: Vec<f32> = midis
            .iter()
            .filter(|&&m| (m - dominant_label as f32).abs() <= 0.75)
            .copied()
            .collect();
        let core_conf: Vec<f32> = midis
            .iter()
            .zip(confidences.iter())
            .filter(|(m, _)| (**m - dominant_label as f32).abs() <= 0.75)
            .map(|(_, c)| *c)
            .collect();
        let core_midis = if core_midis.is_empty() { midis.to_vec() } else { core_midis };
        let core_conf = if core_conf.is_empty() { confidences.to_vec() } else { core_conf };
        return vec![make_pitch_note(
            token_start,
            token_end,
            &core_midis,
            &core_conf,
            core_midis.len(),
        )];
    }

    // 按 label run 分段
    let cleaned_labels = remove_short_label_runs(&labels);
    let runs = label_runs(&cleaned_labels);
    let hop = infer_hop_seconds(times);
    let mut notes = Vec::new();
    for (start_idx, end_idx, _label) in &runs {
        let run_midis = &midis[*start_idx..*end_idx];
        let run_conf = &confidences[*start_idx..*end_idx];
        if run_midis.len() < 2 {
            continue;
        }
        let note_start = token_start.max(times[*start_idx]);
        let note_end = token_end.min(times[*end_idx - 1] + hop);
        notes.push(make_pitch_note(note_start, note_end, run_midis, run_conf, run_midis.len()));
    }
    merge_adjacent_notes(notes)
}

fn remove_short_label_runs(labels: &[i32]) -> Vec<i32> {
    let mut cleaned = labels.to_vec();
    let runs = label_runs(&cleaned);
    for i in 0..runs.len() {
        let (start, end, _label) = runs[i];
        if end - start >= MIN_NOTE_FRAMES {
            continue;
        }
        let prev_label = if i > 0 { Some(runs[i - 1].2) } else { None };
        let next_label = if i + 1 < runs.len() { Some(runs[i + 1].2) } else { None };
        let fill = match (prev_label, next_label) {
            (Some(p), Some(n)) if p == n => p,
            (Some(p), Some(n)) => {
                let prev_len = runs[i - 1].1 - runs[i - 1].0;
                let next_len = runs[i + 1].1 - runs[i + 1].0;
                if prev_len >= next_len { p } else { n }
            }
            (Some(p), None) => p,
            (None, Some(n)) => n,
            _ => _label,
        };
        for idx in start..end {
            cleaned[idx] = fill;
        }
    }
    cleaned
}

fn label_runs(labels: &[i32]) -> Vec<(usize, usize, i32)> {
    if labels.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=labels.len() {
        if i == labels.len() || labels[i] != labels[start] {
            runs.push((start, i, labels[start]));
            start = i;
        }
    }
    runs
}

fn infer_hop_seconds(times: &[f32]) -> f32 {
    if times.len() < 2 {
        return 0.01;
    }
    let mut diffs: Vec<f32> = times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|&d| d > 0.0)
        .collect();
    if diffs.is_empty() {
        return 0.01;
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    diffs[diffs.len() / 2]
}

fn make_pitch_note(
    start_time: f32,
    end_time: f32,
    midis: &[f32],
    confidences: &[f32],
    point_count: usize,
) -> PitchNote {
    let filtered = iqr_filter(midis);
    let median_m = median(&filtered);
    let mean_m = mean(&filtered);
    PitchNote {
        start_time,
        end_time,
        median_midi: median_m,
        mean_midi: mean_m,
        rounded_midi: median_m.round() as i32,
        confidence_mean: mean(confidences),
        point_count,
    }
}

fn iqr_filter(values: &[f32]) -> Vec<f32> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p25_idx = (sorted.len() as f32 * 0.25) as usize;
    let p75_idx = (sorted.len() as f32 * 0.75) as usize;
    let p25 = sorted[p25_idx];
    let p75 = sorted[p75_idx];
    let iqr = p75 - p25;
    if iqr == 0.0 {
        return values.to_vec();
    }
    let lo = p25 - 0.5 * iqr;
    let hi = p75 + 0.5 * iqr;
    let filtered: Vec<f32> = values.iter().filter(|&&v| v >= lo && v <= hi).copied().collect();
    if filtered.is_empty() { values.to_vec() } else { filtered }
}

fn merge_adjacent_notes(notes: Vec<PitchNote>) -> Vec<PitchNote> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut merged = vec![notes[0].clone()];
    for note in notes.into_iter().skip(1) {
        let prev = merged.last_mut().unwrap();
        if prev.rounded_midi == note.rounded_midi {
            let total = prev.point_count + note.point_count;
            let w1 = prev.point_count as f32 / total as f32;
            let w2 = note.point_count as f32 / total as f32;
            prev.end_time = note.end_time;
            prev.median_midi = prev.median_midi * w1 + note.median_midi * w2;
            prev.mean_midi = prev.mean_midi * w1 + note.mean_midi * w2;
            prev.confidence_mean = prev.confidence_mean * w1 + note.confidence_mean * w2;
            prev.rounded_midi = prev.median_midi.round() as i32;
            prev.point_count = total;
        } else {
            merged.push(note);
        }
    }
    merged
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n % 2 == 0 {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    } else {
        s[n / 2]
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

// ── SRT Export ─────────────────────────────────────────────

pub fn export_srt(
    pitch_track: &PitchTrack,
    lyrics: &[LyricLine],
    path: &Path,
) -> Result<(), String> {
    let note_names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

    let midi_to_note = |midi: f32| -> String {
        if midi.is_nan() {
            return "---".to_string();
        }
        let m = midi.round() as i32;
        format!("{}{}", note_names[(((m % 12) + 12) % 12) as usize], m / 12 - 1)
    };

    let to_srt_time = |sec: f32| -> String {
        let hrs = (sec / 3600.0) as u32;
        let mins = ((sec % 3600.0) / 60.0) as u32;
        let secs = (sec % 60.0) as u32;
        let ms = ((sec % 1.0) * 1000.0) as u32;
        format!("{:02}:{:02}:{:02},{:03}", hrs, mins, secs, ms)
    };

    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut idx = 1u32;

    if !lyrics.is_empty() {
        for line in lyrics {
            for token in &line.tokens {
                let (t_start, t_end) = match (token.start_time, token.end_time) {
                    (Some(s), Some(e)) => (s, e),
                    _ => continue,
                };
                let text = if token.text.contains('|') {
                    token.text.split('|').next().unwrap_or(&token.text)
                } else {
                    &token.text
                };
                let note_display = if let Some(note) = token.pitch_notes.first() {
                    format!(" [{}]", midi_to_note(note.median_midi))
                } else {
                    String::new()
                };
                writeln!(f, "{}", idx).map_err(|e| e.to_string())?;
                writeln!(f, "{} --> {}", to_srt_time(t_start), to_srt_time(t_end))
                    .map_err(|e| e.to_string())?;
                writeln!(f, "{}{}\n", text, note_display).map_err(|e| e.to_string())?;
                idx += 1;
            }
        }
    } else {
        let interval = 0.5f32;
        let mut t = 0.0f32;
        while t < pitch_track.times[pitch_track.times.len() - 1] {
            let i = match pitch_track.times.binary_search_by(|probe| probe.partial_cmp(&t).unwrap()) {
                Ok(i) => i,
                Err(i) => i.min(pitch_track.midis.len() - 1),
            };
            let midi = pitch_track.midis[i];
            let mut display = midi_to_note(midi);
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
