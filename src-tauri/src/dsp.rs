// DSP 后处理: 复刻 analyzer.py 的完整管线
//
// stabilize_vocal_midi → apply_median_midi → apply_savgol_midi → quantize
// 其中 stabilize = confidence_mask + remove_short_pitch_islands + apply_hampel

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
pub fn post_process(
    f0: &[f32],
    conf: &[f32],
    confidence_threshold: f32,
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

    // 3. stabilize: confidence mask + remove_short_islands + hampel
    midi = stabilize_vocal_midi(&midi, conf, confidence_threshold);

    // 4. median filter (per segment)
    if median_window > 1 {
        let w = if median_window % 2 != 0 {
            median_window
        } else {
            median_window + 1
        };
        midi = apply_median_midi(&midi, w);
    }

    // 5. savgol filter (per segment)
    if savgol_window > 3 {
        let w = if savgol_window % 2 != 0 {
            savgol_window
        } else {
            savgol_window + 1
        };
        midi = apply_savgol_midi(&midi, w);
    }

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

fn stabilize_vocal_midi(midi: &[f32], conf: &[f32], threshold: f32) -> Vec<f32> {
    let mut out = midi.to_vec();
    // confidence mask
    for i in 0..out.len() {
        if !out[i].is_nan() && i < conf.len() && conf[i] < threshold {
            out[i] = f32::NAN;
        }
    }
    out = remove_short_pitch_islands(&out, 1.25, 5);
    out = apply_hampel_midi(&out, 13, 0.9);
    out
}

/// 找到所有有 voiced (非 NaN) 的连续段
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

fn remove_short_pitch_islands(midi: &[f32], jump_threshold: f32, min_frames: usize) -> Vec<f32> {
    let mut out = midi.to_vec();
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
            if re - rs + 1 >= min_frames {
                continue;
            }
            if ri == 0 || ri == runs.len() - 1 {
                continue;
            }
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

fn apply_hampel_midi(midi: &[f32], window: usize, threshold: f32) -> Vec<f32> {
    let mut out = midi.to_vec();
    let w = if window % 2 != 0 { window } else { window + 1 };
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
            for i in 0..seg_len {
                out[seg_start + i] = filtered[i];
            }
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
        for i in 0..seg_len {
            out[seg_start + i] = filtered[i];
        }
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
    if n % 2 == 0 {
        (valid[n / 2 - 1] + valid[n / 2]) / 2.0
    } else {
        valid[n / 2]
    }
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
