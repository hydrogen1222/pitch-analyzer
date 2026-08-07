// Note Event Tracker: Clean Pitch Track → Annotation Note Track
//
// 目标: 在不改变真实 F0 曲线的前提下，从 clean midi 构建离散稳定音符事件。
//   - minimum duration (短音不成独立事件)
//   - debounce / hysteresis (半音边界抖动、vibrato 不闪烁)
//   - short octave suppression (±12 半音且极短的八度错误)
//   - 真实短倚音 (足够长 + 高置信度) 保留
//
// 输出为 NoteEvent，供歌词绑定与字幕使用，不反过来覆盖真实曲线。

use crate::models::{midi_to_note_name, NoteEvent, NoteTrackingParams};

/// 中间 run 结构 (一个 quantized midi 的连续帧段)
#[derive(Debug, Clone)]
struct RawRun {
    start_idx: usize,
    end_idx: usize, // inclusive
    rounded: i32,
    sum_midi: f64,
    sum_conf: f64,
    count: usize,
}

impl RawRun {
    fn new(idx: usize, midi: f32, conf: f32) -> Self {
        Self {
            start_idx: idx,
            end_idx: idx,
            rounded: midi.round() as i32,
            sum_midi: midi as f64,
            sum_conf: conf as f64,
            count: 1,
        }
    }

    fn len(&self) -> usize {
        self.end_idx - self.start_idx + 1
    }

    fn midi_mean(&self) -> f32 {
        if self.count == 0 {
            self.rounded as f32
        } else {
            (self.sum_midi / self.count as f64) as f32
        }
    }

    fn conf_mean(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            (self.sum_conf / self.count as f64) as f32
        }
    }

    /// 吸收另一个 run (延长到其末尾并合并统计)
    fn absorb(&mut self, other: &RawRun) {
        self.end_idx = other.end_idx;
        self.sum_midi += other.sum_midi;
        self.sum_conf += other.sum_conf;
        self.count += other.count;
    }
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

/// 从 clean pitch track 构建 NoteEvent 列表
pub fn build_note_events(
    times: &[f32],
    midis: &[f32],
    confidences: &[f32],
    params: &NoteTrackingParams,
) -> Vec<NoteEvent> {
    if times.is_empty() || midis.is_empty() {
        return Vec::new();
    }
    let n = times.len().min(midis.len()).min(confidences.len());
    let hop = infer_hop_seconds(times).max(1e-3);
    let to_frames = |ms: f32| ((ms / 1000.0 / hop).round() as usize).max(1);
    let min_frames = to_frames(params.min_note_duration_ms);
    let switch_frames = to_frames(params.switch_confirm_ms);
    let octave_frames = to_frames(params.octave_error_max_ms);
    let hysteresis_semi = params.semitone_hysteresis_cents / 100.0;

    // ── Step 1: 按 quantized midi 切分原始 run，无音帧作边界 ──
    let mut runs: Vec<RawRun> = Vec::new();
    let mut cur: Option<RawRun> = None;
    for i in 0..n {
        let m = midis[i];
        if m.is_nan() {
            if let Some(r) = cur.take() {
                runs.push(r);
            }
            continue;
        }
        let rounded = m.round() as i32;
        if let Some(r) = cur.as_mut() {
            if r.rounded == rounded {
                r.end_idx = i;
                r.sum_midi += m as f64;
                r.sum_conf += confidences[i] as f64;
                r.count += 1;
                continue;
            }
            runs.push(cur.take().unwrap());
        }
        cur = Some(RawRun::new(i, m, confidences[i]));
    }
    if let Some(r) = cur.take() {
        runs.push(r);
    }

    // ── Step 2: hysteresis / debounce 合并 ──
    // 新音与当前音只差 1 半音且持续时间 < switch_confirm_ms → 视为抖动，吸收进当前音
    let mut merged: Vec<RawRun> = Vec::new();
    for run in runs {
        let is_short = run.len() < switch_frames;
        if is_short && !merged.is_empty() {
            let prev = merged.last().unwrap();
            let diff = (run.rounded - prev.rounded).abs() as f32;
            let contiguous = run.start_idx <= prev.end_idx + 1;
            if diff <= 1.0 + hysteresis_semi && contiguous {
                let absorbed = run;
                merged.last_mut().unwrap().absorb(&absorbed);
                continue;
            }
        }
        merged.push(run);
    }
    coalesce_adjacent(&mut merged);

    // ── Step 3: 短暂八度错误抑制 ──
    // 与前后稳定音恰差约 ±12 半音且持续极短 → 视为 octave error
    {
        let mut i = 0usize;
        while i < merged.len() {
            let run = &merged[i];
            let is_short = run.len() <= octave_frames;
            let prev_oct = if i > 0 {
                (run.rounded - merged[i - 1].rounded).abs() as f32
            } else {
                0.0
            };
            let next_oct = if i + 1 < merged.len() {
                (run.rounded - merged[i + 1].rounded).abs() as f32
            } else {
                0.0
            };
            let oct_ok = |d: f32| (d - 12.0).abs() <= 1.5;
            if is_short && i > 0 && i + 1 < merged.len() && oct_ok(prev_oct) && oct_ok(next_oct) {
                drop_run_merge(&mut merged, i);
                // 不推进 i：前邻可能再次变化
            } else {
                i += 1;
            }
        }
        coalesce_adjacent(&mut merged);
    }

    // ── Step 4: 最短时长过滤 ──
    {
        let mut i = 0usize;
        while i < merged.len() {
            if merged[i].len() < min_frames {
                drop_run_merge(&mut merged, i);
            } else {
                i += 1;
            }
        }
        coalesce_adjacent(&mut merged);
    }

    // ── Step 5: 输出 NoteEvent ──
    merged
        .iter()
        .map(|r| {
            let midi = r.midi_mean().round();
            NoteEvent {
                start: times[r.start_idx],
                end: times[r.end_idx] + hop,
                midi: midi as i32,
                note_name: midi_to_note_name(midi),
                confidence: r.conf_mean(),
            }
        })
        .collect()
}

/// 合并相邻同音 run (hysteresis 吸收 / 丢弃后都可能产生同音相邻段)
fn coalesce_adjacent(runs: &mut Vec<RawRun>) {
    let mut i = 0usize;
    while i + 1 < runs.len() {
        let contiguous = runs[i].end_idx + 1 >= runs[i + 1].start_idx;
        if runs[i].rounded == runs[i + 1].rounded && contiguous {
            let next = runs.remove(i + 1);
            runs[i].absorb(&next);
        } else {
            i += 1;
        }
    }
}

/// 丢弃 runs[idx] 并合并进相邻 run
///  - 邻居与当前同音 → 直接吸收 (合并统计)
///  - 两侧同音 (如 C4-C5-C4 的八度错误/噪声) → 合并两侧为一段连续音
///  - 两侧不同音 → 并入较长邻居 (只扩展时间，不污染音高统计)
fn drop_run_merge(runs: &mut Vec<RawRun>, idx: usize) {
    if runs.len() <= 1 {
        runs.remove(idx);
        return;
    }
    let cur_rounded = runs[idx].rounded;
    let prev_rounded = if idx > 0 { Some(runs[idx - 1].rounded) } else { None };
    let next_rounded = if idx + 1 < runs.len() { Some(runs[idx + 1].rounded) } else { None };

    // 1) 邻居与当前同音 → 吸收
    if prev_rounded == Some(cur_rounded) {
        let removed = runs.remove(idx);
        let p = &mut runs[idx - 1];
        p.end_idx = removed.end_idx;
        p.sum_midi += removed.sum_midi;
        p.sum_conf += removed.sum_conf;
        p.count += removed.count;
        return;
    }
    if next_rounded == Some(cur_rounded) {
        let removed = runs.remove(idx);
        let nx = &mut runs[idx];
        nx.start_idx = removed.start_idx;
        nx.sum_midi += removed.sum_midi;
        nx.sum_conf += removed.sum_conf;
        nx.count += removed.count;
        return;
    }
    // 2) 两侧同音 → 合并两侧
    if let (Some(p), Some(n)) = (prev_rounded, next_rounded) {
        if p == n {
            let next_run = runs.remove(idx + 1);
            runs.remove(idx);
            let p = &mut runs[idx - 1];
            p.end_idx = next_run.end_idx;
            p.sum_midi += next_run.sum_midi;
            p.sum_conf += next_run.sum_conf;
            p.count += next_run.count;
            return;
        }
    }
    // 3) 只扩展时间到较长邻居 (不同音，不合并音高统计)
    let prev_len = if idx > 0 { runs[idx - 1].len() } else { 0 };
    let next_len = if idx + 1 < runs.len() { runs[idx + 1].len() } else { 0 };
    let removed = runs.remove(idx);
    if idx > 0 && prev_len >= next_len {
        let p = &mut runs[idx - 1];
        p.end_idx = removed.end_idx;
    } else {
        let nx = &mut runs[idx];
        nx.start_idx = removed.start_idx;
    }
}
