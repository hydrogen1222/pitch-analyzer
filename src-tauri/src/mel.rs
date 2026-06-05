// Mel spectrogram, 复刻 CleanMelModule (export_fcpe_onnx.py)
//
// 输入: 16 kHz mono f32, 输出: (n_frames, n_mels) 的 log-mel
//
// 关键参数 (来自 fcpe_config.json):
//   sr=16000, hop=160, win=1024, n_fft=1024, n_mels=128, fmin=0, fmax=8000
// 流程:
//   1. reflect pad (win-hop)/2 左, max((win-hop+1)/2, win-n-pad_left) 右
//   2. STFT (Hann window, center=false, normalized=false, onesided=true)
//   3. magnitude = sqrt(real^2 + imag^2 + 1e-9)
//   4. mel = mel_basis @ magnitude
//   5. log(clamp(mel, 1e-5))

use ndarray::Array2;
use rustfft::{num_complex::Complex32, FftPlanner};

pub struct MelConfig {
    pub sr: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub win: usize,
    pub n_mels: usize,
    pub fmin: f32,
    pub fmax: f32,
    pub clip_val: f32,
}

impl Default for MelConfig {
    fn default() -> Self {
        Self {
            sr: 16_000,
            n_fft: 1024,
            hop: 160,
            win: 1024,
            n_mels: 128,
            fmin: 0.0,
            fmax: 8000.0,
            clip_val: 1e-5,
        }
    }
}

pub struct MelExtractor {
    cfg: MelConfig,
    window: Vec<f32>,
    mel_basis: Array2<f32>, // (n_mels, n_fft/2+1)
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl MelExtractor {
    pub fn new(cfg: MelConfig) -> Self {
        let window = hann_window(cfg.win);
        let mel_basis = librosa_mel_filterbank(cfg.sr as f32, cfg.n_fft, cfg.n_mels, cfg.fmin, cfg.fmax);
        let fft = FftPlanner::new().plan_fft_forward(cfg.n_fft);
        Self { cfg, window, mel_basis, fft }
    }

    /// 返回 (n_frames, n_mels) 的 log-mel
    pub fn compute(&self, wav: &[f32]) -> Array2<f32> {
        let n = wav.len();
        let pad_left = (self.cfg.win - self.cfg.hop) / 2;
        let pad_right = std::cmp::max(
            (self.cfg.win - self.cfg.hop + 1) / 2,
            self.cfg.win.saturating_sub(n + pad_left),
        );
        let padded = reflect_pad(wav, pad_left, pad_right);

        // PyTorch stft center=false 帧数: floor((N - n_fft) / hop) + 1
        let total = padded.len();
        let n_frames = if total < self.cfg.n_fft {
            0
        } else {
            (total - self.cfg.n_fft) / self.cfg.hop + 1
        };
        let half = self.cfg.n_fft / 2 + 1;
        let mut mag = Array2::<f32>::zeros((half, n_frames));

        let mut buf = vec![Complex32::new(0.0, 0.0); self.cfg.n_fft];
        for f in 0..n_frames {
            let start = f * self.cfg.hop;
            for i in 0..self.cfg.n_fft {
                let v = padded[start + i] * self.window[i];
                buf[i] = Complex32::new(v, 0.0);
            }
            self.fft.process(&mut buf);
            for k in 0..half {
                let c = buf[k];
                mag[(k, f)] = (c.re * c.re + c.im * c.im + 1e-9).sqrt();
            }
        }

        // mel = mel_basis (n_mels, half) @ mag (half, n_frames) -> (n_mels, n_frames)
        let mel = self.mel_basis.dot(&mag);

        // log clamp + 转置到 (n_frames, n_mels)
        let mut out = Array2::<f32>::zeros((n_frames, self.cfg.n_mels));
        for f in 0..n_frames {
            for m in 0..self.cfg.n_mels {
                out[(f, m)] = mel[(m, f)].max(self.cfg.clip_val).ln();
            }
        }

        // 帧数对齐 torchfcpe: n_frames_target = n / hop + 1, 不够则重复最后一帧
        let target = n / self.cfg.hop + 1;
        if out.nrows() < target && out.nrows() > 0 {
            let pad_n = target - out.nrows();
            let last = out.row(out.nrows() - 1).to_owned();
            let mut padded_mel = Array2::<f32>::zeros((target, self.cfg.n_mels));
            for f in 0..out.nrows() {
                padded_mel.row_mut(f).assign(&out.row(f));
            }
            for f in 0..pad_n {
                padded_mel.row_mut(out.nrows() + f).assign(&last);
            }
            padded_mel
        } else if out.nrows() > target {
            out.slice_move(ndarray::s![..target, ..])
        } else {
            out
        }
    }
}

fn hann_window(n: usize) -> Vec<f32> {
    // PyTorch torch.hann_window 默认 periodic=true: w[i] = 0.5 * (1 - cos(2π i / N))
    (0..n)
        .map(|i| 0.5 - 0.5 * ((2.0 * std::f32::consts::PI * i as f32) / n as f32).cos())
        .collect()
}

fn reflect_pad(x: &[f32], left: usize, right: usize) -> Vec<f32> {
    let n = x.len();
    let mut out = Vec::with_capacity(n + left + right);
    // PyTorch reflect pad: 不重复边界点, 例如 [1,2,3,4] pad_left=2 -> [3,2,1,2,3,4]
    for i in 0..left {
        let idx = left - i; // 1..=left
        out.push(x[idx.min(n.saturating_sub(1))]);
    }
    out.extend_from_slice(x);
    for i in 0..right {
        let idx = (n as isize - 2 - i as isize).max(0) as usize;
        out.push(x[idx.min(n.saturating_sub(1))]);
    }
    out
}

/// 复刻 librosa.filters.mel(sr, n_fft, n_mels, fmin, fmax)
/// 默认 htk=False, norm='slaney'
fn librosa_mel_filterbank(sr: f32, n_fft: usize, n_mels: usize, fmin: f32, fmax: f32) -> Array2<f32> {
    let n_bins = n_fft / 2 + 1;
    let fft_freqs: Vec<f32> = (0..n_bins).map(|k| k as f32 * sr / n_fft as f32).collect();

    let min_mel = hz_to_mel_slaney(fmin);
    let max_mel = hz_to_mel_slaney(fmax);
    let mel_pts: Vec<f32> = (0..n_mels + 2)
        .map(|i| min_mel + (max_mel - min_mel) * i as f32 / (n_mels + 1) as f32)
        .collect();
    let hz_pts: Vec<f32> = mel_pts.iter().map(|&m| mel_to_hz_slaney(m)).collect();

    let mut weights = Array2::<f32>::zeros((n_mels, n_bins));
    let fdiff: Vec<f32> = (0..hz_pts.len() - 1).map(|i| hz_pts[i + 1] - hz_pts[i]).collect();

    for i in 0..n_mels {
        for k in 0..n_bins {
            let lower = (fft_freqs[k] - hz_pts[i]) / fdiff[i];
            let upper = (hz_pts[i + 2] - fft_freqs[k]) / fdiff[i + 1];
            weights[(i, k)] = (lower.min(upper)).max(0.0);
        }
        // Slaney normalization: 2 / (hz[i+2] - hz[i])
        let enorm = 2.0 / (hz_pts[i + 2] - hz_pts[i]);
        for k in 0..n_bins {
            weights[(i, k)] *= enorm;
        }
    }
    weights
}

// Slaney scale (librosa 默认): 1000 Hz 以下线性, 以上对数
fn hz_to_mel_slaney(freq: f32) -> f32 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f32).ln() / 27.0;
    if freq >= min_log_hz {
        min_log_mel + (freq / min_log_hz).ln() / logstep
    } else {
        freq / f_sp
    }
}

fn mel_to_hz_slaney(mel: f32) -> f32 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f32).ln() / 27.0;
    if mel >= min_log_mel {
        min_log_hz * ((mel - min_log_mel) * logstep).exp()
    } else {
        f_sp * mel
    }
}
