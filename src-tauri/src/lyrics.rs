// 歌词解析: LRC + TXT, tokenizer, 字级对齐, NoteEvent 绑定, primary note 选择

use crate::models::{LyricLine, LyricToken, NoteEvent, NoteTrackingParams, PitchNote, PitchTrack};
use regex::Regex;
use std::cmp::Ordering;

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
                primary_note: None,
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
                primary_note: None,
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

// ── Token 时间分配 / 对齐 ──────────────────────────────────

/// 字级对齐参数
#[derive(Debug, Clone, Copy)]
pub struct TokenAlignParams {
    /// 最小字时长，防止出现极端短字
    pub min_token_duration_ms: f32,
    /// 对齐窗口在 voiced 区域外扩的 margin
    pub voiced_margin_ms: f32,
}

impl Default for TokenAlignParams {
    fn default() -> Self {
        Self {
            min_token_duration_ms: 60.0,
            voiced_margin_ms: 80.0,
        }
    }
}

/// 均匀分配 token 时间 (作为 fallback，不再是主要算法)
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
        distribute_line_times(line, start, end);
    }
}

fn distribute_line_times(line: &mut LyricLine, start: f32, end: f32) {
    if line.tokens.is_empty() {
        return;
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

/// 在句级约束内，根据人声音频特征自动分配字级时间。
///
/// 特征: voiced/unvoiced、F0 变化、RMS 能量包络；字符数量作为最终约束；
/// 通过 DP 代价函数选择 N-1 个最佳切分点。无音频特征时回退到均匀分配。
pub fn align_token_times(lines: &mut [LyricLine], track: &PitchTrack, params: &TokenAlignParams) {
    for line in lines.iter_mut() {
        let (start, end) = match (line.start_time, line.end_time) {
            (Some(s), Some(e)) if e > s => (s, e),
            _ => continue,
        };
        if line.tokens.is_empty() {
            continue;
        }
        // 已有逐字时间 (如 enhanced LRC) → 不重新估计
        if line.tokens.iter().all(|t| t.start_time.is_some() && t.end_time.is_some()) {
            continue;
        }
        if !dp_align_line(line, track, start, end, params) {
            distribute_line_times(line, start, end);
        }
    }
}

/// 基于特征 DP 的句内字级对齐。返回是否成功；失败由调用方回退均匀分配。
fn dp_align_line(
    line: &mut LyricLine,
    track: &PitchTrack,
    line_start: f32,
    line_end: f32,
    params: &TokenAlignParams,
) -> bool {
    let n_tokens = line.tokens.len();
    if n_tokens < 2 || track.times.is_empty() {
        return false;
    }

    // 收集 line 范围内帧索引
    let mut fidx: Vec<usize> = Vec::new();
    for i in 0..track.times.len() {
        let t = track.times[i];
        if t >= line_start && t < line_end {
            fidx.push(i);
        }
    }
    if fidx.len() < 2 {
        return false;
    }

    // 对齐窗口限定到 voiced 发声区域 (首尾留 margin)
    let margin = params.voiced_margin_ms / 1000.0;
    let mut v_first: Option<usize> = None;
    let mut v_last: usize = fidx[0];
    for &fi in &fidx {
        if !track.midis[fi].is_nan() {
            if v_first.is_none() {
                v_first = Some(fi);
            }
            v_last = fi;
        }
    }
    let align_start = match v_first {
        Some(fi) => line_start.max(track.times[fi] - margin),
        None => line_start,
    };
    let align_end = match v_first {
        Some(_) => line_end.min(track.times[v_last] + margin),
        None => line_end,
    };
    if align_end <= align_start {
        return false;
    }

    // 重新只取窗口内帧
    let fidx: Vec<usize> = fidx
        .into_iter()
        .filter(|&i| track.times[i] >= align_start && track.times[i] < align_end)
        .collect();
    if fidx.len() < 2 {
        return false;
    }

    let total_dur = align_end - align_start;
    let ideal = total_dur / n_tokens as f32;
    let min_dur = (params.min_token_duration_ms / 1000.0).min(ideal);

    // 边界得分: 无音间隙 / F0 变化 / RMS 能量谷 → 好的音节边界
    let boundary_score = |fi: usize| -> f32 {
        let mut s = 0.0f32;
        if track.midis[fi].is_nan() {
            s += 2.0; // 落在无声间隙
        } else if fi > 0 && track.midis[fi - 1].is_nan() {
            s += 1.0; // 起音处
        }
        if fi > 0 {
            let m0 = track.midis[fi - 1];
            let m1 = track.midis[fi];
            if !m0.is_nan() && !m1.is_nan() && m0.round() as i32 != m1.round() as i32 {
                s += 1.5; // F0 换音
            }
        }
        if fi > 0 && track.rms.len() > fi {
            let r0 = track.rms[fi - 1];
            let r1 = track.rms[fi];
            if r1 < r0 * 0.6 {
                s += 1.0; // 能量谷
            }
        }
        s
    };

    let time_at = |j: usize| track.times[fidx[j]];
    let inf = f32::INFINITY;
    let m = fidx.len();
    let last_k = n_tokens - 2; // 需要内部边界的最大 token 下标 (0..=n-2 有内部边界)

    let mut dp = vec![vec![inf; m]; n_tokens];
    let mut parent = vec![vec![usize::MAX; m]; n_tokens];

    let seg_cost = |prev_t: f32, cur_t: f32, score: f32| -> f32 {
        let dur = cur_t - prev_t;
        if dur < min_dur {
            return inf;
        }
        let dur_cost = (dur - ideal).abs() / ideal.max(1e-3);
        let bnd_cost = -score * 0.8;
        dur_cost + bnd_cost
    };

    // 第一个 token 从 align_start 开始
    for j in 0..m {
        let t1 = time_at(j);
        if t1 <= align_start {
            continue;
        }
        dp[0][j] = seg_cost(align_start, t1, boundary_score(fidx[j]));
    }

    // 中间 token
    for k in 1..=last_k {
        for j in k..m {
            let t_j = time_at(j);
            let mut best = inf;
            let mut bp = usize::MAX;
            for p in (k - 1)..j {
                if dp[k - 1][p] == inf {
                    continue;
                }
                let t_p = time_at(p);
                let c = dp[k - 1][p] + seg_cost(t_p, t_j, boundary_score(fidx[j]));
                if c < best {
                    best = c;
                    bp = p;
                }
            }
            dp[k][j] = best;
            parent[k][j] = bp;
        }
    }

    // 最后一个 token 结束于 align_end
    let mut best_cost = inf;
    let mut best_j = usize::MAX;
    for j in last_k..m {
        if dp[last_k][j] == inf {
            continue;
        }
        let t_j = time_at(j);
        let c = dp[last_k][j] + seg_cost(t_j, align_end, 0.0);
        if c < best_cost {
            best_cost = c;
            best_j = j;
        }
    }
    if best_j == usize::MAX {
        return false;
    }

    // 回溯边界
    let mut b_after = vec![usize::MAX; n_tokens];
    let mut cur = best_j;
    for k in (0..=last_k).rev() {
        b_after[k] = cur;
        cur = parent[k][cur];
    }

    let mut prev_time = align_start;
    for k in 0..n_tokens {
        let t_end = if k == n_tokens - 1 {
            align_end
        } else {
            time_at(b_after[k])
        };
        line.tokens[k].start_time = Some(prev_time);
        line.tokens[k].end_time = Some(t_end);
        prev_time = t_end;
    }
    true
}

// ── Token ↔ pitch 绑定 ────────────────────────────────────

const MIN_NOTE_FRAMES: usize = 5;
const DOMINANT_NOTE_RATIO: f32 = 0.65;

/// 绑定 pitch 到每个 token，并选择 primary_note。
///
/// 优先使用 NoteEvent (Annotation Track)；无 NoteEvent 时回退到帧级分段。
/// 绑定时忽略 token 首尾 edge_ignore margin，使用中心稳定区域。
pub fn bind_pitch_to_tokens(
    lines: &mut [LyricLine],
    pitch_track: &PitchTrack,
    confidence_threshold: f32,
    note_tracking: &NoteTrackingParams,
) {
    if pitch_track.times.is_empty() {
        return;
    }
    let edge_ignore = note_tracking.edge_ignore_ms / 1000.0;
    let min_dur = note_tracking.min_note_duration_ms / 1000.0;

    for line in lines.iter_mut() {
        for token in &mut line.tokens {
            let (t_start, t_end) = match (token.start_time, token.end_time) {
                (Some(s), Some(e)) => (s, e),
                _ => continue,
            };
            // 中心稳定区域
            let c_start = t_start + edge_ignore;
            let c_end = t_end - edge_ignore;
            let (c_start, c_end) = if c_end > c_start {
                (c_start, c_end)
            } else {
                (t_start, t_end)
            };

            if !pitch_track.note_events.is_empty() {
                token.pitch_notes =
                    note_events_in_range(&pitch_track.note_events, c_start, c_end, note_tracking);
            } else {
                token.pitch_notes =
                    frame_segment_notes(pitch_track, c_start, c_end, confidence_threshold);
            }
            token.primary_note =
                select_primary_note(&token.pitch_notes, min_dur, confidence_threshold);
        }
    }
}

/// 取与 token 中心区域有足够重叠的 NoteEvent (起始时间裁剪到 token 内)
fn note_events_in_range(
    events: &[NoteEvent],
    start: f32,
    end: f32,
    params: &NoteTrackingParams,
) -> Vec<PitchNote> {
    let min_overlap = (params.edge_ignore_ms / 1000.0).max(0.0);
    events
        .iter()
        .filter_map(|ev| {
            let ov_start = ev.start.max(start);
            let ov_end = ev.end.min(end);
            let overlap = ov_end - ov_start;
            if overlap >= min_overlap && overlap > 0.0 {
                Some(note_event_to_pitch_note(ev, ov_start, ov_end))
            } else {
                None
            }
        })
        .collect()
}

fn note_event_to_pitch_note(ev: &NoteEvent, start: f32, end: f32) -> PitchNote {
    let dur = (end - start).max(0.0);
    PitchNote {
        start_time: start,
        end_time: end,
        median_midi: ev.midi as f32,
        mean_midi: ev.midi as f32,
        rounded_midi: ev.midi,
        confidence_mean: ev.confidence,
        point_count: (dur / 0.01).round().max(1.0) as usize,
    }
}

/// 帧级分段 (无 NoteEvent 时的回退路径)
fn frame_segment_notes(
    pitch_track: &PitchTrack,
    c_start: f32,
    c_end: f32,
    confidence_threshold: f32,
) -> Vec<PitchNote> {
    let mut seg_times = Vec::new();
    let mut seg_midis = Vec::new();
    let mut seg_conf = Vec::new();
    for i in 0..pitch_track.times.len() {
        let t = pitch_track.times[i];
        if t < c_start {
            continue;
        }
        if t >= c_end {
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
    segment_pitch_notes(&seg_times, &seg_midis, &seg_conf, c_start, c_end)
}

/// 从 NoteEvent / 帧分段结果中选择主音:
/// score = voiced_duration × mean_confidence，剔除非常短、低 confidence 的候选。
pub fn select_primary_note(
    notes: &[PitchNote],
    min_duration: f32,
    min_confidence: f32,
) -> Option<PitchNote> {
    let filtered = notes
        .iter()
        .filter(|n| (n.end_time - n.start_time) >= min_duration && n.confidence_mean >= min_confidence)
        .max_by(|a, b| score_cmp(a, b));
    match filtered {
        Some(n) => Some(n.clone()),
        None => notes.iter().max_by(|a, b| score_cmp(a, b)).cloned(),
    }
}

fn score_cmp(a: &PitchNote, b: &PitchNote) -> Ordering {
    let sa = (a.end_time - a.start_time) * a.confidence_mean;
    let sb = (b.end_time - b.start_time) * b.confidence_mean;
    sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
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
