// Note Event Tracker v2: Clean Pitch Track → Annotation Note Track
//
// v2 核心变化 (任务书 Phase A): 不再 "先 round() 再分段 + debounce 补救"。
// 主算法直接工作在连续 midi/cents 空间:
//
//   1. 有效帧筛选 + 微间隙桥接 (两侧同 target 的 ≤30ms 无声不拆音)
//   2. 稳健基线 (小窗口中位数, 只用于事件识别, 不改写 UI 的连续 F0)
//   3. stable-plateau 状态机:
//        中心 ±stay_radius (40c) 内  → 永不换音 (vibrato 跨半音取整不再分裂)
//        偏离超过 switch_deviation  → 收集新目标候选 (run)
//        候选确认 = 持续 ≥confirm_duration 且平台稳定 (MAD ≤ max_mad)
//                   且与旧中心音程分离 ≥ switch_separation
//   4. 短倚音例外: ≥appoggiatura_min 且音程明显且非八度误差 → 保留
//
// 由此:
//   vibrato (围绕同一中心摆动)   → 一个音符 + gesture, 不产生新事件
//   glide (连续滑移)             → 两端音符, 中间半音不成事件
//   true melisma (多个稳定平台)  → 全部保留
//
// 输出 NoteEvent (v2 字段: center_midi / stable_duration / tracker_version),
// 供歌词绑定与字幕使用; 连续 Clean 曲线永不被本层改写。

use crate::models::{
    midi_to_note_name, NoteEvent, NoteTrackingParams, PitchGesture, PitchGestureKind,
};

/// 候选平台评估窗口 (帧)
const CANDIDATE_WINDOW_FRAMES: usize = 10;
/// 初始音符确认窗 (帧): 歌曲开头的第一个音符无条件在此之后确认。
/// 中心取窗口中位数 —— vibrato 围绕中位数对称摆动, 不会因取整边界反复分裂;
/// 后续换音仍由 stay/switch/separation 滞回严格把关。
const INITIAL_CONFIRM_FRAMES: usize = 20;
/// 中心慢速漂移系数 (跟踪真实音准漂移, 不追 vibrato)
const DRIFT_FACTOR: f32 = 0.05;

/// 中位数
fn median(values: &[f32]) -> f32 {
    let mut v: Vec<f32> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.is_empty() {
        return f32::NAN;
    }
    if v.len().is_multiple_of(2) {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
    } else {
        v[v.len() / 2]
    }
}

/// 稳健基线: 每个有效帧取 ±2 帧的中位数 (只用于事件识别)
fn robust_baseline(midis: &[f32]) -> Vec<f32> {
    let n = midis.len();
    let mut baseline = vec![f32::NAN; n];
    for i in 0..n {
        if midis[i].is_nan() {
            continue;
        }
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(n);
        let win: Vec<f32> = midis[lo..hi]
            .iter()
            .copied()
            .filter(|v| !v.is_nan())
            .collect();
        if !win.is_empty() {
            baseline[i] = median(&win);
        }
    }
    baseline
}

fn hop_of(times: &[f32]) -> f32 {
    if times.len() < 2 {
        return 0.01;
    }
    let span = (times[times.len() - 1] as f64) - (times[0] as f64);
    if span <= 0.0 {
        return 0.01;
    }
    (span / (times.len() - 1) as f64).max(1e-3) as f32
}

/// 候选平台: 一段偏离当前中心的连续帧
#[derive(Debug, Clone)]
struct Pending {
    start: usize,
    last: usize,
    values: Vec<f32>, // 基线值 (连续 midi)
}

impl Pending {
    fn push(&mut self, i: usize, v: f32) {
        self.last = i;
        self.values.push(v);
    }
    fn run_frames(&self) -> usize {
        self.last - self.start + 1
    }
    /// 最近 W 帧的稳健中心
    fn center(&self, w: usize) -> f32 {
        let n = self.values.len();
        let lo = n.saturating_sub(w);
        median(&self.values[lo..])
    }
    /// 最近 W 帧相对 center 的 MAD (cents)
    fn mad_cents(&self, w: usize, center: f32) -> f32 {
        let n = self.values.len();
        let lo = n.saturating_sub(w);
        let devs: Vec<f32> = self.values[lo..]
            .iter()
            .map(|v| (v - center).abs() * 100.0)
            .collect();
        median(&devs)
    }
}

/// 短 run 最小音程分离 (半音): vibrato 半周期 (~75c) 不得冒充倚音
const SHORT_RUN_MIN_SEPARATION_CENTS: f32 = 100.0;

/// 短 run 去留判定 (初始/被打断的 pending):
/// true = 保留为短音符 (倚音类), false = 丢弃 (噪声/八度误差/vibrato 半周期)
/// 需要: 时长 ≥ appoggiatura_min 且 平台稳定 (MAD) 且 与后续音分离 ≥1 半音 且 非八度误差
fn keep_short_run(
    pending: &Pending,
    next_center: f32,
    params: &NoteTrackingParams,
    hop: f32,
) -> bool {
    let dur_ms = pending.run_frames() as f32 * hop * 1000.0;
    if dur_ms < params.appoggiatura_min_ms {
        return false; // 太短 (20~40ms flicker / 八度 glitch / 1 帧噪声)
    }
    let run_center = pending.center(CANDIDATE_WINDOW_FRAMES);
    // 平台稳定度: 摆动的半周期 (围绕旧中心下落) MAD 高, 不是稳定倚音
    let mad = pending.mad_cents(CANDIDATE_WINDOW_FRAMES, run_center);
    if mad > params.candidate_max_mad_cents {
        return false;
    }
    let sep = (run_center - next_center).abs() * 100.0;
    if sep < SHORT_RUN_MIN_SEPARATION_CENTS {
        return false; // 与后续主音无明显音程
    }
    // 八度类误差 (±12 半音附近) 即便时长足够也丢弃
    if (sep - 1200.0).abs() <= 150.0 {
        return false;
    }
    true
}

/// NoteTracker v2 主入口
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
    let hop = hop_of(times);
    let baseline = robust_baseline(&midis[..n]);

    let stay = params.stay_radius_cents / 100.0; // semitones
    let switch = params.switch_deviation_cents / 100.0;
    let separation_min = params.switch_separation_cents / 100.0;
    let confirm_frames = ((params.confirm_duration_ms / 1000.0 / hop).round() as usize).max(2);
    let bridge = params.bridge_gap_frames;

    struct FinishedNote {
        start: usize,
        end: usize,
        center: f32,
    }
    let mut finished: Vec<FinishedNote> = Vec::new();

    let mut note_start: usize = 0;
    let mut note_end: usize = 0;
    let mut center: f32 = 0.0;
    let mut has_note = false;
    let mut pending: Option<Pending> = None;

    // pending 落地: 达确认条件 → 新音符; 否则短倚音判定 / 并回当前音
    #[allow(clippy::too_many_arguments)] // 状态机显式传参
    fn resolve_pending(
        finished: &mut Vec<FinishedNote>,
        note_start: &mut usize,
        note_end: &mut usize,
        center: &mut f32,
        has_note: &mut bool,
        pending: Pending,
        next_center: f32,
        confirm_frames: usize,
        params: &NoteTrackingParams,
    ) {
        let p_center = pending.center(CANDIDATE_WINDOW_FRAMES);
        let stable = pending.run_frames() >= confirm_frames
            && pending.mad_cents(CANDIDATE_WINDOW_FRAMES, p_center)
                <= params.candidate_max_mad_cents;
        if stable {
            if *has_note {
                let fs = FinishedNote {
                    start: *note_start,
                    end: *note_end,
                    center: *center,
                };
                finished.push(fs);
            }
            *note_start = pending.start;
            *note_end = pending.last;
            *center = p_center;
            *has_note = true;
        } else if *has_note {
            // 被打断的短 run: 并回当前音符 (flicker / 短暂偏离)
            *note_end = (*note_end).max(pending.last);
        } else {
            // 无当前音符: 短倚音判定
            if keep_short_run(&pending, next_center, params, 0.01) {
                finished.push(FinishedNote {
                    start: pending.start,
                    end: pending.last,
                    center: p_center,
                });
            }
        }
    }

    let mut i = 0usize;
    while i < n {
        // ── 无声帧: 间隙处理 ──
        if midis[i].is_nan() || baseline[i].is_nan() {
            if !has_note && pending.is_none() {
                i += 1;
                continue;
            }
            let mut j = i;
            while j < n && (midis[j].is_nan() || baseline[j].is_nan()) {
                j += 1;
            }
            let gap_frames = j - i;
            let next_val = if j < n { Some(baseline[j]) } else { None };

            let can_bridge = gap_frames <= bridge && next_val.is_some() && has_note;
            if can_bridge {
                let nv = next_val.unwrap();
                let reference = pending
                    .as_ref()
                    .map(|p| p.center(CANDIDATE_WINDOW_FRAMES))
                    .unwrap_or(center);
                let continuous = (nv - center).abs() <= stay
                    || (pending.is_some() && (nv - reference).abs() <= switch);
                if continuous {
                    if let Some(p) = pending.as_mut() {
                        p.last = j; // pending 跨隙延伸
                    } else {
                        note_end = j - 1;
                    }
                    i = j;
                    continue;
                }
            }

            // 不可桥接: 收尾
            if let Some(p) = pending.take() {
                let next_center = next_val.unwrap_or(p.center(CANDIDATE_WINDOW_FRAMES));
                resolve_pending(
                    &mut finished,
                    &mut note_start,
                    &mut note_end,
                    &mut center,
                    &mut has_note,
                    p,
                    next_center,
                    confirm_frames,
                    params,
                );
            }
            if has_note {
                finished.push(FinishedNote {
                    start: note_start,
                    end: note_end,
                    center,
                });
                has_note = false;
            }
            i = j;
            continue;
        }

        let x = baseline[i];

        // ── 尚无音符: 初始 pending ──
        // 接受窗 = 全量中位数 ± switch: vibrato/scoop 围绕中位数整体 riding,
        // 不因半周期摆动反复打断 (半周期短 run 已被 keep_short_run 稳定性拒绝)
        if !has_note {
            match pending.take() {
                None => {
                    pending = Some(Pending {
                        start: i,
                        last: i,
                        values: vec![x],
                    });
                }
                Some(mut p) => {
                    let pc_all = median(&p.values);
                    if (x - pc_all).abs() <= switch {
                        p.push(i, x);
                        // 初始音符确认: 无条件 (开头必有真实发音),
                        // 中心 = 全量中位数, vibrato 围绕它对称摆动不分裂
                        if p.run_frames() >= INITIAL_CONFIRM_FRAMES {
                            note_start = p.start;
                            note_end = p.last;
                            center = pc_all;
                            has_note = true;
                            pending = None;
                        } else {
                            pending = Some(p);
                        }
                    } else {
                        // 初始 pending 被打断: 短倚音判定 (相对新起点)
                        resolve_pending(
                            &mut finished,
                            &mut note_start,
                            &mut note_end,
                            &mut center,
                            &mut has_note,
                            p,
                            x,
                            confirm_frames,
                            params,
                        );
                        pending = Some(Pending {
                            start: i,
                            last: i,
                            values: vec![x],
                        });
                    }
                }
            }
            i += 1;
            continue;
        }

        // ── 有当前音符 ──
        let dev = (x - center).abs();
        if dev <= stay {
            // 回到中心附近: 未确认的 pending 作废 (run 内的帧回并当前音符)
            pending.take();
            note_end = i;
            center += (x - center) * DRIFT_FACTOR;
            i += 1;
            continue;
        }

        // 偏离: 收集/扩展候选
        match pending.take() {
            None => {
                pending = Some(Pending {
                    start: i,
                    last: i,
                    values: vec![x],
                });
            }
            Some(mut p) => {
                p.push(i, x);
                let pc = p.center(CANDIDATE_WINDOW_FRAMES);
                let mad = p.mad_cents(CANDIDATE_WINDOW_FRAMES, pc);
                let separation = (pc - center).abs();
                if p.run_frames() >= confirm_frames
                    && mad <= params.candidate_max_mad_cents
                    && separation >= separation_min
                {
                    // 确认新音符
                    finished.push(FinishedNote {
                        start: note_start,
                        end: note_end,
                        center,
                    });
                    note_start = p.start;
                    note_end = p.last;
                    center = pc;
                } else {
                    pending = Some(p);
                }
            }
        }
        i += 1;
    }

    // ── 收尾 ──
    if let Some(p) = pending.take() {
        let pc = p.center(CANDIDATE_WINDOW_FRAMES);
        let mad = p.mad_cents(CANDIDATE_WINDOW_FRAMES, pc);
        let stable = p.run_frames() >= confirm_frames && mad <= params.candidate_max_mad_cents;
        // 分离度不足 (如 vibrato 谷底驻留) → 并回当前音符, 不构成新音
        let separation_ok = !has_note || (pc - center).abs() >= separation_min;
        if stable && separation_ok {
            if has_note {
                finished.push(FinishedNote {
                    start: note_start,
                    end: note_end,
                    center,
                });
                has_note = false;
            }
            finished.push(FinishedNote {
                start: p.start,
                end: p.last,
                center: pc,
            });
        } else if has_note {
            note_end = note_end.max(p.last);
            finished.push(FinishedNote {
                start: note_start,
                end: note_end,
                center,
            });
            has_note = false;
        } else if keep_short_run(&p, pc, params, hop) {
            finished.push(FinishedNote {
                start: p.start,
                end: p.last,
                center: pc,
            });
        }
    }
    if has_note {
        finished.push(FinishedNote {
            start: note_start,
            end: note_end,
            center,
        });
    }

    // ── 输出 ──
    let mut events: Vec<NoteEvent> = Vec::with_capacity(finished.len());
    for f in finished {
        let start_t = times[f.start];
        let end_t = times[f.end] + hop;
        let mut conf_sum = 0.0f64;
        let mut conf_cnt = 0usize;
        for k in f.start..=f.end.min(n - 1) {
            if !midis[k].is_nan() {
                conf_sum += confidences[k] as f64;
                conf_cnt += 1;
            }
        }
        let confidence = if conf_cnt > 0 {
            (conf_sum / conf_cnt as f64) as f32
        } else {
            0.0
        };
        let midi = f.center.round() as i32;
        let gesture = PitchGesture {
            start: start_t,
            end: end_t,
            kind: PitchGestureKind::Stable,
            center_midi: Some(f.center),
            from_midi: None,
            to_midi: None,
            depth_cents: None,
            rate_hz: None,
            confidence,
        };
        events.push(NoteEvent {
            start: start_t,
            end: end_t,
            midi,
            note_name: midi_to_note_name(f.center),
            confidence,
            center_midi: Some(f.center),
            stable_duration: end_t - start_t,
            gestures: vec![gesture],
            tracker_version: 2,
        });
    }
    events
}
