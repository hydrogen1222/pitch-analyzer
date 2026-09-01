// DSP 后处理: 复刻 analyzer.py 的完整管线
//
// stabilize_vocal_midi → apply_median_midi → apply_savgol_midi → quantize
// 其中 stabilize = confidence_mask + relative_energy_gate + short_return_jitter
//                + remove_short_pitch_islands + apply_hampel

/// f0 → MIDI: midi = 69 + 12 * log2(f0/440), f0 <= 0 → NaN
pub fn f0_to_midi(f0: &[f32]) -> Vec<f32> {
    f0.iter()
        .map(|&f| {
            if f <= 0.0 || f.is_nan() {
                f32::NAN
            } else {
                69.0 + 12.0 * (f / 440.0).log2()
            }
        })
        .collect()
}

/// MIDI → f0: f0 = 440 * 2^((midi-69)/12), NaN → NaN
pub fn midi_to_f0(midi: &[f32]) -> Vec<f32> {
    midi.iter()
        .map(|&m| {
            if m.is_nan() {
                f32::NAN
            } else {
                440.0 * ((m - 69.0) / 12.0).exp2()
            }
        })
        .collect()
}

/// 完整后处理管线 (对应 analyzer.py finalize)
#[allow(clippy::too_many_arguments)] // 历史管线签名, 参数均有用途
pub fn post_process(
    f0: &[f32],
    conf: &[f32],
    rms: &[f32],
    confidence_threshold: f32,
    rms_threshold: f32,
    fmin: f32,
    fmax: f32,
    median_window: usize,
    savgol_window: usize,
    quantize: bool,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    // 1. 频率范围过滤
    let f0: Vec<f32> = f0
        .iter()
        .map(|&f| if f >= fmin && f <= fmax { f } else { 0.0 })
        .collect();

    // 2. 转 MIDI
    let mut midi = f0_to_midi(&f0);

    // 3. stabilize: confidence & rms mask + remove_short_islands + hampel
    midi = stabilize_vocal_midi(&midi, conf, rms, confidence_threshold, rms_threshold);

    // 4. median filter (per segment)
    if median_window > 1 {
        let w = if !median_window.is_multiple_of(2) {
            median_window
        } else {
            median_window + 1
        };
        midi = apply_median_midi(&midi, w);
    }

    // 5. savgol filter (per segment)
    if savgol_window > 3 {
        let w = if !savgol_window.is_multiple_of(2) {
            savgol_window
        } else {
            savgol_window + 1
        };
        midi = apply_savgol_midi(&midi, w);
    }

    // Savitzky-Golay 在音符边缘可能产生很小的单帧回摆。用同一套保守的
    // “短暂偏离后回到原中心”判定做最终收口，避免平滑器重新制造视觉抖动。
    suppress_short_return_jitter(&mut midi);
    suppress_single_frame_zigzags(&mut midi);

    // 6. quantize
    if quantize {
        for m in midi.iter_mut() {
            if !m.is_nan() {
                *m = m.round();
            }
        }
    }

    // 7. back-calculate frequencies
    let final_freqs = midi_to_f0(&midi);

    // 8. timestamps (hop=160, sr=16000 → 10ms per frame)
    let times: Vec<f32> = (0..f0.len()).map(|i| i as f32 * 160.0 / 16000.0).collect();

    (times, final_freqs, midi)
}

#[allow(clippy::needless_range_loop)] // DSP 逐帧索引是算法本体
fn stabilize_vocal_midi(
    midi: &[f32],
    conf: &[f32],
    rms: &[f32],
    conf_threshold: f32,
    rms_threshold: f32,
) -> Vec<f32> {
    let mut out = midi.to_vec();
    // confidence & rms mask
    for i in 0..out.len() {
        let low_conf = i < conf.len() && conf[i] < conf_threshold;
        let low_rms = i < rms.len() && rms[i] < rms_threshold;
        if !out[i].is_nan() && (low_conf || low_rms) {
            out[i] = f32::NAN;
        }
    }
    // FCPE may lock onto a breath/noise component for only 10-20ms even when
    // its confidence clears the absolute thresholds. Repair only when that
    // frame is markedly quieter than its local voiced context and the pitch
    // on both sides agrees; real note transitions therefore remain intact.
    repair_relative_low_energy_spikes(&mut out, rms);
    // 继续吸收音量正常但只有 10-30ms 的亚半音往返。这里只处理“短暂偏离后
    // 回到同一稳定中心”的小幅 excursion；持续换音及 >= 1 半音的倚音不动。
    suppress_short_return_jitter(&mut out);
    // 孤立短 voiced 段: 静音/换气间隙里 1-2 帧的发声毛刺 (置信度略过阈值即可产生),
    // median/hampel 对孤岛无能为力 (nanmedian 有值就返回), 必须整段丢弃
    drop_short_voiced_segments(&mut out, MIN_VOICED_SEGMENT_FRAMES);
    out = remove_short_pitch_islands(&out, conf, ISLAND_JUMP_SEMITONES, ISLAND_MIN_FRAMES);
    out = apply_hampel_midi(&out, 13, 0.9);
    out
}

/// 把长度不足 min_seg_frames 的孤立 voiced 段整体置为 unvoiced
fn drop_short_voiced_segments(midi: &mut [f32], min_seg_frames: usize) {
    for (s, e) in iter_voiced_segments(midi) {
        if e - s + 1 < min_seg_frames {
            for v in &mut midi[s..=e] {
                *v = f32::NAN;
            }
        }
    }
}

/// 找到所有有 voiced (非 NaN) 的连续段
#[allow(clippy::needless_range_loop)] // 段边界按帧索引追踪
fn iter_voiced_segments(midi: &[f32]) -> Vec<(usize, usize)> {
    let mut segs = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..midi.len() {
        if !midi[i].is_nan() && start.is_none() {
            start = Some(i);
        } else if midi[i].is_nan() && start.is_some() {
            segs.push((start.unwrap(), i - 1));
            start = None;
        }
    }
    if let Some(s) = start {
        segs.push((s, midi.len() - 1));
    }
    segs
}

// 短音符清理阈值 (集中管理, 10ms/帧)
/// run 间跳变判定 (半音): 超过则视为不同 run
const ISLAND_JUMP_SEMITONES: f32 = 1.25;
/// 非 run 边界的最短有效音符帧数 (50ms)
const ISLAND_MIN_FRAMES: usize = 5;
/// 孤立 voiced 段的最短帧数 (30ms): 更短的孤岛视为发声毛刺整段丢弃
const MIN_VOICED_SEGMENT_FRAMES: usize = 3;
/// 边界泛音误差 (八度/十二度/双八度) 允许的最长修正帧数 (150ms)
const ISLAND_HARMONIC_MAX_FRAMES: usize = 15;
/// 相对能量比较的单侧邻域 (60ms)
const RELATIVE_RMS_HALF_WINDOW: usize = 6;
/// 比局部中位 RMS 低约 5dB 即视为显著偏低
const RELATIVE_RMS_RATIO: f32 = 0.55;
/// 只修复 10-30ms 的低能量异常，不抹除持续弱音或明确转音
const RELATIVE_RMS_MAX_FRAMES: usize = 3;
/// 前后主音必须基本一致，才允许用其插值替代异常帧
const RELATIVE_RMS_NEIGHBOR_SEMITONES: f32 = 0.75;
/// 25 cents 起纳入低能量毛刺修复
const RELATIVE_RMS_MIN_PITCH_DEVIATION: f32 = 0.25;

/// 亚半音短往返的单侧稳定上下文 (40ms)
const SHORT_RETURN_CONTEXT_FRAMES: usize = 4;
/// 最长吸收 30ms，超过则视为有意的音高运动
const SHORT_RETURN_MAX_FRAMES: usize = 3;
/// 两侧稳定中心必须相差不超过 35 cents
const SHORT_RETURN_CENTER_TOLERANCE: f32 = 0.35;
/// 上下文自身必须足够平稳
const SHORT_RETURN_CONTEXT_SPREAD: f32 = 0.35;
/// 18 cents 以下不必追逐，形成显示所需的迟滞区
const SHORT_RETURN_MIN_DEVIATION: f32 = 0.18;
/// 只吸收轻微抖动；接近或超过一个半音的短音仍保留
const SHORT_RETURN_MAX_DEVIATION: f32 = 0.70;
/// 最终单帧规则允许的 A/C 邻点差异
const SINGLE_FRAME_NEIGHBOR_TOLERANCE: f32 = 0.25;
/// ≥1 半音仍按明确短音保留
const SINGLE_FRAME_MAX_DEVIATION: f32 = 0.95;

#[allow(clippy::needless_range_loop)] // DSP 逐帧索引是算法本体
fn repair_relative_low_energy_spikes(midi: &mut [f32], rms: &[f32]) {
    if midi.len() < 3 || rms.len() < midi.len() {
        return;
    }

    let mut low_energy = vec![false; midi.len()];
    for i in 0..midi.len() {
        if midi[i].is_nan() || !rms[i].is_finite() {
            continue;
        }
        let lo = i.saturating_sub(RELATIVE_RMS_HALF_WINDOW);
        let hi = (i + RELATIVE_RMS_HALF_WINDOW + 1).min(midi.len());
        let local_rms: Vec<f32> = (lo..hi)
            .filter(|&j| j != i && !midi[j].is_nan() && rms[j].is_finite() && rms[j] > 0.0)
            .map(|j| rms[j])
            .collect();
        if local_rms.len() < 4 {
            continue;
        }
        let local_median = nanmedian(&local_rms);
        low_energy[i] = local_median > 0.0 && rms[i] < local_median * RELATIVE_RMS_RATIO;
    }

    let mut start = 0usize;
    while start < low_energy.len() {
        if !low_energy[start] {
            start += 1;
            continue;
        }
        let mut end = start;
        while end + 1 < low_energy.len() && low_energy[end + 1] {
            end += 1;
        }
        let run_len = end - start + 1;
        if run_len <= RELATIVE_RMS_MAX_FRAMES && start > 0 && end + 1 < midi.len() {
            let left = midi[start - 1];
            let right = midi[end + 1];
            if !left.is_nan()
                && !right.is_nan()
                && (left - right).abs() <= RELATIVE_RMS_NEIGHBOR_SEMITONES
            {
                let max_deviation = (start..=end)
                    .map(|i| (midi[i] - (left + right) * 0.5).abs())
                    .fold(0.0f32, f32::max);
                if max_deviation >= RELATIVE_RMS_MIN_PITCH_DEVIATION {
                    for offset in 0..run_len {
                        let t = (offset + 1) as f32 / (run_len + 1) as f32;
                        midi[start + offset] = left + t * (right - left);
                    }
                }
            }
        }
        start = end + 1;
    }
}

/// 修复稳定音高中的 10-30ms 小幅往返。
///
/// 与中值滤波不同，这里要求偏离段前后各有 40ms 的稳定上下文、两侧中心一致，
/// 且紧邻边界已经回到各自中心。因此不会把一个持续的新音误识别为短毛刺。
fn suppress_short_return_jitter(midi: &mut [f32]) {
    if midi.len() < SHORT_RETURN_CONTEXT_FRAMES * 2 + 1 {
        return;
    }

    for (seg_start, seg_end) in iter_voiced_segments(midi) {
        if seg_end - seg_start + 1 < SHORT_RETURN_CONTEXT_FRAMES * 2 + 1 {
            continue;
        }

        let mut start = seg_start + SHORT_RETURN_CONTEXT_FRAMES;
        let last_start = seg_end.saturating_sub(SHORT_RETURN_CONTEXT_FRAMES);
        while start <= last_start {
            let mut matched: Option<(usize, f32, f32)> = None;

            // 先尝试较长的偏离段，避免只修掉一个 30ms hump 的中间一帧。
            for run_len in (1..=SHORT_RETURN_MAX_FRAMES).rev() {
                let end = start + run_len - 1;
                if end + SHORT_RETURN_CONTEXT_FRAMES > seg_end {
                    continue;
                }

                let left = &midi[start - SHORT_RETURN_CONTEXT_FRAMES..start];
                let right = &midi[end + 1..=end + SHORT_RETURN_CONTEXT_FRAMES];
                let left_center = nanmedian(left);
                let right_center = nanmedian(right);
                if (left_center - right_center).abs() > SHORT_RETURN_CENTER_TOLERANCE {
                    continue;
                }
                if left
                    .iter()
                    .any(|value| (*value - left_center).abs() > SHORT_RETURN_CONTEXT_SPREAD)
                    || right
                        .iter()
                        .any(|value| (*value - right_center).abs() > SHORT_RETURN_CONTEXT_SPREAD)
                {
                    continue;
                }

                // 候选必须完整包住 excursion；若紧邻帧仍在偏离，就说明它在持续。
                if (midi[start - 1] - left_center).abs() > SHORT_RETURN_MIN_DEVIATION
                    || (midi[end + 1] - right_center).abs() > SHORT_RETURN_MIN_DEVIATION
                {
                    continue;
                }

                let baseline = (left_center + right_center) * 0.5;
                let deviations: Vec<f32> = midi[start..=end]
                    .iter()
                    .map(|value| *value - baseline)
                    .collect();
                let min_deviation = deviations
                    .iter()
                    .map(|value| value.abs())
                    .fold(f32::INFINITY, f32::min);
                let max_deviation = deviations
                    .iter()
                    .map(|value| value.abs())
                    .fold(0.0f32, f32::max);
                let one_sided = deviations
                    .iter()
                    .all(|value| value.signum() == deviations[0].signum());
                if one_sided
                    && min_deviation >= SHORT_RETURN_MIN_DEVIATION
                    && max_deviation <= SHORT_RETURN_MAX_DEVIATION
                {
                    matched = Some((run_len, left_center, right_center));
                    break;
                }
            }

            if let Some((run_len, left_center, right_center)) = matched {
                for offset in 0..run_len {
                    let t = (offset + 1) as f32 / (run_len + 1) as f32;
                    midi[start + offset] = left_center + t * (right_center - left_center);
                }
                start += run_len;
            } else {
                start += 1;
            }
        }
    }
}

/// 最终滤波器只修复 A-B-C 三帧中的单帧尖点。
/// A/C 必须已经回到同一音高，B 也必须小于一个半音；这能覆盖长上下文不稳定时
/// 仍会被肉眼看到的 10ms 锯齿，同时保留明确的半音短音和持续音高变化。
fn suppress_single_frame_zigzags(midi: &mut [f32]) {
    if midi.len() < 3 {
        return;
    }
    let source = midi.to_vec();
    for i in 1..source.len() - 1 {
        let (left, current, right) = (source[i - 1], source[i], source[i + 1]);
        if left.is_nan() || current.is_nan() || right.is_nan() {
            continue;
        }
        let center = (left + right) * 0.5;
        let deviation = (current - center).abs();
        if (left - right).abs() <= SINGLE_FRAME_NEIGHBOR_TOLERANCE
            && (SHORT_RETURN_MIN_DEVIATION..=SINGLE_FRAME_MAX_DEVIATION).contains(&deviation)
        {
            midi[i] = center;
        }
    }
}

#[allow(clippy::needless_range_loop)] // DSP 逐帧索引是算法本体
fn remove_short_pitch_islands(
    midi: &[f32],
    conf: &[f32],
    jump_threshold: f32,
    min_frames: usize,
) -> Vec<f32> {
    let mut out = midi.to_vec();
    #[allow(clippy::needless_range_loop)] // DSP 逐帧索引是算法本体
    for (seg_start, seg_end) in iter_voiced_segments(midi) {
        let seg_len = seg_end - seg_start + 1;
        if seg_len < 3 {
            continue;
        }
        // 按 jump 分 sub-runs
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut run_start = seg_start;
        for i in (seg_start + 1)..=seg_end {
            if (out[i] - out[i - 1]).abs() > jump_threshold {
                runs.push((run_start, i - 1));
                run_start = i;
            }
        }
        runs.push((run_start, seg_end));

        for ri in 0..runs.len() {
            let (rs, re) = runs[ri];
            let run_len = re - rs + 1;

            // 首尾 run: "短暂异常 + 相邻稳定主音" 判断。
            // 不要简单删除首尾保护：只有很短、与主音差距大且置信度低(或恰为八度错误)才修正为相邻稳定音。
            if ri == 0 || ri == runs.len() - 1 {
                if runs.len() < 2 {
                    continue;
                }
                let (nbr_s, nbr_e) = if ri == 0 {
                    runs[1]
                } else {
                    runs[runs.len() - 2]
                };
                let nbr_len = nbr_e - nbr_s + 1;
                if nbr_len < min_frames {
                    continue; // 相邻 run 也不是稳定主音
                }
                let cur_med = nanmedian(&out[rs..=re]);
                let nbr_med = nanmedian(&out[nbr_s..=nbr_e]);
                let diff = (cur_med - nbr_med).abs();
                if diff <= jump_threshold {
                    continue; // 差距不大 → 保留 (真实短音/倚音)
                }
                let cur_conf = nanmean(&conf[rs..=re.min(conf.len() - 1)]);
                let nbr_conf = nanmean(&conf[nbr_s..=nbr_e.min(conf.len() - 1)]);

                // 常见泛音误差: 八度(12), 十二度(19), 两个八度(24), 甚至更高
                let is_harmonic = (diff - 12.0).abs() <= 1.5
                    || (diff - 19.0).abs() <= 1.5
                    || (diff - 24.0).abs() <= 1.5
                    || (diff - 28.0).abs() <= 1.5;

                // 如果是边界，对于泛音误差我们容忍更长的帧数（比如最多 15 帧 / 150ms）
                // 对于一般的置信度低的杂音，容忍 min_frames
                let allowed_len = if is_harmonic {
                    ISLAND_HARMONIC_MAX_FRAMES
                } else {
                    min_frames
                };

                if run_len < allowed_len && (is_harmonic || cur_conf < nbr_conf) {
                    for j in rs..=re {
                        out[j] = nbr_med;
                    }
                }
                continue;
            }

            // 非首尾 run: 需要满足短时长
            if run_len >= min_frames {
                continue;
            }

            // 中间 run: 仅在前后主音接近时插值过渡
            let (prev_s, prev_e) = runs[ri - 1];
            let (next_s, next_e) = runs[ri + 1];
            let prev_med = nanmedian(&out[prev_s..=prev_e]);
            let next_med = nanmedian(&out[next_s..=next_e]);
            if (prev_med - next_med).abs() <= 0.75 {
                let start_val = out[prev_e];
                let end_val = out[next_s];
                let len = re - rs + 1;
                for j in 0..len {
                    let t = (j + 1) as f32 / (len + 2) as f32;
                    out[rs + j] = start_val + t * (end_val - start_val);
                }
            }
        }
    }
    out
}

#[allow(clippy::needless_range_loop)] // DSP 逐帧索引是算法本体
fn apply_hampel_midi(midi: &[f32], window: usize, threshold: f32) -> Vec<f32> {
    let mut out = midi.to_vec();
    let w = if !window.is_multiple_of(2) {
        window
    } else {
        window + 1
    };
    let half = w / 2;

    for (seg_start, seg_end) in iter_voiced_segments(midi) {
        if seg_end - seg_start + 1 < 3 {
            continue;
        }
        let seg = &midi[seg_start..=seg_end];
        let seg_len = seg.len();
        for local_idx in 0..seg_len {
            let lo = local_idx.saturating_sub(half);
            let hi = (local_idx + half + 1).min(seg_len);
            let med = nanmedian(&seg[lo..hi]);
            if !med.is_nan() && (seg[local_idx] - med).abs() > threshold {
                out[seg_start + local_idx] = med;
            }
        }
    }
    out
}

fn apply_median_midi(midi: &[f32], window: usize) -> Vec<f32> {
    let mut out = midi.to_vec();
    for (seg_start, seg_end) in iter_voiced_segments(midi) {
        let seg_len = seg_end - seg_start + 1;
        let k = if seg_len < window {
            if seg_len % 2 != 0 {
                seg_len
            } else {
                seg_len - 1
            }
        } else {
            window
        };
        if k >= 3 {
            let seg = &midi[seg_start..=seg_end];
            let filtered = median_filter(seg, k);
            out[seg_start..=seg_end].copy_from_slice(&filtered);
        }
    }
    out
}

fn apply_savgol_midi(midi: &[f32], window: usize) -> Vec<f32> {
    let mut out = midi.to_vec();
    for (seg_start, seg_end) in iter_voiced_segments(midi) {
        let seg_len = seg_end - seg_start + 1;
        if seg_len <= window {
            continue;
        }
        let seg = &midi[seg_start..=seg_end];
        let filtered = savgol_filter(seg, window, 3);
        // 只写回完整窗口覆盖的帧: 边缘半窗内样本不足, 拟合偏差大, 保留原值
        let half = window / 2;
        out[seg_start + half..seg_start + seg_len - half]
            .copy_from_slice(&filtered[half..seg_len - half]);
    }
    out
}

// --- 基础数值工具 ---

fn nanmedian(x: &[f32]) -> f32 {
    let mut valid: Vec<f32> = x.iter().filter(|&&v| !v.is_nan()).copied().collect();
    if valid.is_empty() {
        return f32::NAN;
    }
    valid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = valid.len();
    if n.is_multiple_of(2) {
        (valid[n / 2 - 1] + valid[n / 2]) / 2.0
    } else {
        valid[n / 2]
    }
}

fn nanmean(x: &[f32]) -> f32 {
    let valid: Vec<f32> = x.iter().filter(|&&v| !v.is_nan()).copied().collect();
    if valid.is_empty() {
        return 0.0;
    }
    valid.iter().sum::<f32>() / valid.len() as f32
}

/// 简单 median filter (只处理有效段内的值，不含 NaN)
fn median_filter(x: &[f32], k: usize) -> Vec<f32> {
    let n = x.len();
    let half = k / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        out.push(nanmedian(&x[lo..hi]));
    }
    out
}

/// Savitzky-Golay 滤波器 (polyorder=3)
/// 简化实现: 对有效值做多项式拟合，或直接用卷积系数
/// 这里用卷积系数法 (与 scipy.signal.savgol_coeffs 一致)
#[allow(clippy::needless_range_loop)] // 卷积窗口索引是算法本体
fn savgol_filter(x: &[f32], window: usize, polyorder: usize) -> Vec<f32> {
    let coeffs = savgol_coeffs(window, polyorder);
    let n = x.len();
    let half = window / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut acc = 0.0f32;
        let mut w_sum = 0.0f32;
        for j in 0..window {
            let idx = i as isize + j as isize - half as isize;
            if idx >= 0 && (idx as usize) < n {
                let v = x[idx as usize];
                if !v.is_nan() {
                    acc += coeffs[j] * v;
                    w_sum += coeffs[j].abs();
                }
            }
        }
        out.push(if w_sum > 0.0 { acc } else { f32::NAN });
    }
    out
}

/// 计算 Savitzky-Golay 卷积系数 (中心点平滑)
/// 与 scipy.signal.savgol_coeffs(window, polyorder, deriv=0) 一致
fn savgol_coeffs(window: usize, polyorder: usize) -> Vec<f32> {
    let half = window as isize / 2;
    let m = polyorder + 1;
    let n = window;

    // 构建 Vandermonde-like 矩阵 A (n x m)
    let mut a = vec![0.0f64; n * m];
    for i in 0..n {
        let x = (i as isize - half) as f64;
        for j in 0..m {
            a[i * m + j] = x.powi(j as i32);
        }
    }

    // A^T A
    let mut ata = vec![0.0f64; m * m];
    for i in 0..m {
        for j in 0..m {
            let mut s = 0.0;
            for k in 0..n {
                s += a[k * m + i] * a[k * m + j];
            }
            ata[i * m + j] = s;
        }
    }

    // 求逆 (Gauss-Jordan, m 通常很小 = 4)
    let mut inv = vec![0.0f64; m * m];
    for i in 0..m {
        inv[i * m + i] = 1.0;
    }
    for col in 0..m {
        let pivot = ata[col * m + col];
        if pivot.abs() < 1e-12 {
            continue;
        }
        for j in 0..m {
            ata[col * m + j] /= pivot;
            inv[col * m + j] /= pivot;
        }
        for row in 0..m {
            if row == col {
                continue;
            }
            let factor = ata[row * m + col];
            for j in 0..m {
                ata[row * m + j] -= factor * ata[col * m + j];
                inv[row * m + j] -= factor * inv[col * m + j];
            }
        }
    }

    // coeffs[i] = sum(inv[0][j] * a[i][j] for j)  (deriv=0 → 第 0 行)
    let mut coeffs = Vec::with_capacity(n);
    for i in 0..n {
        let mut c = 0.0f64;
        for j in 0..m {
            c += inv[j] * a[i * m + j]; // inv 第 0 行
        }
        coeffs.push(c as f32);
    }
    coeffs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midis_from(vals: &[(f32, usize)]) -> Vec<f32> {
        vals.iter().flat_map(|&(m, len)| vec![m; len]).collect()
    }

    fn conf_vec(len: usize, v: f32) -> Vec<f32> {
        vec![v; len]
    }

    /// 短暂八度错误 (段中): C4 C4 C5(20ms) C4 C4 → C5 被插值为过渡，全部变回 C4
    #[test]
    fn test_short_octave_island_mid() {
        let midis = midis_from(&[(60.0, 10), (72.0, 2), (60.0, 10)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        let valid: Vec<f32> = out.iter().filter(|m| !m.is_nan()).copied().collect();
        assert!(
            valid.iter().all(|&m| (m - 60.0).abs() < 1.0),
            "mid-run octave island should be smoothed to C4: {:?}",
            valid
        );
    }

    /// 句首八度错误: C5(30ms) → C4(500ms) → 首部短暂 C5 被修正为相邻主音 C4
    #[test]
    fn test_sentence_start_octave_fixed() {
        let midis = midis_from(&[(72.0, 3), (60.0, 50)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        let head: Vec<f32> = out[0..3].to_vec();
        assert!(
            head.iter().all(|&m| (m - 60.0).abs() < 1.0),
            "sentence-start C5 must be fixed to C4: {:?}",
            head
        );
    }

    /// 尾音错误: E4(500ms) → E5(20ms) → 尾部 E5 被修正回 E4
    #[test]
    fn test_tail_octave_fixed() {
        let midis = midis_from(&[(64.0, 50), (76.0, 2)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        let tail: Vec<f32> = out[50..52].to_vec();
        assert!(
            tail.iter().all(|&m| (m - 64.0).abs() < 1.0),
            "tail E5 must be fixed to E4: {:?}",
            tail
        );
    }

    /// 真实倚音: D4(100ms) → E4(400ms) → 首部足够长，不得被删除
    #[test]
    fn test_real_appoggiatura_kept() {
        let midis = midis_from(&[(62.0, 10), (64.0, 40)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        assert!(
            (out[0] - 62.0).abs() < 0.5,
            "appoggiatura D4 must be kept: {}",
            out[0]
        );
        assert!(
            (out[9] - 62.0).abs() < 0.5,
            "appoggiatura tail must be kept: {}",
            out[9]
        );
        assert!(
            (out[10] - 64.0).abs() < 0.5,
            "main note E4 must be kept: {}",
            out[10]
        );
    }

    /// 极短噪声: C4 中间 1 帧 G5(10ms) → 前后主音接近 → 插值抹平
    #[test]
    fn test_short_noise_interpolated() {
        let midis = midis_from(&[(60.0, 10), (79.0, 1), (60.0, 10)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        let valid: Vec<f32> = out.iter().filter(|m| !m.is_nan()).copied().collect();
        assert!(
            valid.iter().all(|&m| (m - 60.0).abs() < 1.0),
            "1-frame noise must be smoothed away: {:?}",
            valid
        );
    }

    /// 短音但置信度高、且非八度 → 保留 (不误删真实短音)
    #[test]
    fn test_short_low_diff_kept() {
        // 首部 2 帧 61 (C#4) 后接 60 (C4): 差距 1 半音 < jump_threshold → 不是异常段
        let midis = midis_from(&[(61.0, 2), (60.0, 20)]);
        let conf = conf_vec(midis.len(), 0.8);
        let out = remove_short_pitch_islands(&midis, &conf, 1.25, 5);
        // 由于 diff <= jump_threshold，两个 run 其实是同一段，原样保留
        assert!(!out[0].is_nan());
    }

    /// 静音间隙里的孤立发声毛刺 (1-2 帧) → 整段丢弃为 unvoiced
    #[test]
    fn test_isolated_voiced_blip_dropped() {
        let mut midis = vec![f32::NAN; 30];
        midis[10] = 84.0;
        midis[11] = 84.3;
        let conf = conf_vec(30, 0.9);
        let rms = conf_vec(30, 0.1);
        let out = stabilize_vocal_midi(&midis, &conf, &rms, 0.3, 0.005);
        assert!(
            out[10].is_nan() && out[11].is_nan(),
            "isolated blip must be dropped: {:?}",
            &out[8..14]
        );

        // 连续正常发声段不受影响
        let mut midis2 = vec![f32::NAN; 10];
        midis2.extend(vec![60.0, 60.1, 60.2]);
        midis2.extend(vec![f32::NAN; 10]);
        let out2 =
            stabilize_vocal_midi(&midis2, &conf_vec(23, 0.9), &conf_vec(23, 0.1), 0.3, 0.005);
        // 3 帧段恰好在阈值上, 保留
        assert!(
            !out2[10].is_nan() && !out2[12].is_nan(),
            "3-frame segment must be kept"
        );
    }

    #[test]
    fn test_relative_low_energy_single_frame_jitter_is_repaired() {
        let mut midis = midis_from(&[(60.0, 8), (60.7, 1), (60.0, 8)]);
        let mut rms = vec![0.1; midis.len()];
        rms[8] = 0.02;
        repair_relative_low_energy_spikes(&mut midis, &rms);
        assert!(
            (midis[8] - 60.0).abs() < 0.05,
            "low-energy 10ms jitter should follow its audible neighbors: {}",
            midis[8]
        );
    }

    #[test]
    fn test_equal_energy_short_note_is_preserved() {
        let mut midis = midis_from(&[(60.0, 8), (60.7, 1), (60.0, 8)]);
        let rms = vec![0.1; midis.len()];
        repair_relative_low_energy_spikes(&mut midis, &rms);
        assert_eq!(midis[8], 60.7, "audible short pitch movement must remain");
    }

    #[test]
    fn test_sustained_soft_note_is_not_treated_as_a_spike() {
        let mut midis = midis_from(&[(60.0, 8), (62.0, 8), (60.0, 8)]);
        let mut rms = vec![0.1; midis.len()];
        rms[8..16].fill(0.02);
        repair_relative_low_energy_spikes(&mut midis, &rms);
        assert!(
            midis[8..16].iter().all(|midi| (*midi - 62.0).abs() < 0.01),
            "a sustained soft note is not a 10-20ms noise spike"
        );
    }

    #[test]
    fn test_low_energy_transition_is_preserved_when_neighbors_disagree() {
        let mut midis = midis_from(&[(60.0, 8), (61.0, 1), (62.0, 8)]);
        let mut rms = vec![0.1; midis.len()];
        rms[8] = 0.02;
        repair_relative_low_energy_spikes(&mut midis, &rms);
        assert_eq!(midis[8], 61.0, "real transition must not be flattened");
    }

    #[test]
    fn test_single_frame_subsemitone_return_jitter_is_suppressed() {
        let mut midis = midis_from(&[(60.0, 8), (60.32, 1), (60.0, 8)]);
        suppress_short_return_jitter(&mut midis);
        assert!(
            (midis[8] - 60.0).abs() < 0.01,
            "10ms return jitter should be absorbed: {}",
            midis[8]
        );
    }

    #[test]
    fn test_three_frame_subsemitone_return_jitter_is_suppressed() {
        let mut midis = midis_from(&[(60.0, 8), (60.52, 3), (60.0, 8)]);
        suppress_short_return_jitter(&mut midis);
        assert!(
            midis[8..11].iter().all(|midi| (*midi - 60.0).abs() < 0.01),
            "30ms return jitter should be absorbed: {:?}",
            &midis[8..11]
        );
    }

    #[test]
    fn test_sustained_or_semitone_pitch_motion_is_preserved() {
        let mut sustained = midis_from(&[(60.0, 8), (60.52, 4), (60.0, 8)]);
        suppress_short_return_jitter(&mut sustained);
        assert!(
            sustained[8..12]
                .iter()
                .all(|midi| (*midi - 60.52).abs() < 0.01),
            "40ms sustained motion must not be partially erased: {:?}",
            &sustained[8..12]
        );

        let mut semitone = midis_from(&[(60.0, 8), (61.0, 2), (60.0, 8)]);
        suppress_short_return_jitter(&mut semitone);
        assert!(
            semitone[8..10]
                .iter()
                .all(|midi| (*midi - 61.0).abs() < 0.01),
            "a clear semitone ornament must remain: {:?}",
            &semitone[8..10]
        );

        let mut wide_single_frame = midis_from(&[(60.0, 8), (60.82, 1), (60.0, 8)]);
        suppress_single_frame_zigzags(&mut wide_single_frame);
        assert!(
            (wide_single_frame[8] - 60.0).abs() < 0.01,
            "a 10ms sub-semitone zigzag must be absorbed"
        );
        let mut clear_semitone = midis_from(&[(60.0, 8), (61.0, 1), (60.0, 8)]);
        suppress_single_frame_zigzags(&mut clear_semitone);
        assert_eq!(
            clear_semitone[8], 61.0,
            "a clear semitone must remain even when only one frame is available"
        );
    }
}
