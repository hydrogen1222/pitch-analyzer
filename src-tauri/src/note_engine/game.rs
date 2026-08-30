// OpenVPI GAME Note Engine: Generative Adaptive MIDI Extractor (ONNX Native)
//
// 官方仓库: https://github.com/openvpi/GAME (MIT)
// 推理管线 1:1 参照官方 openvpi/dataset-tools 的 game-infer (C++) 实现:
//
//   encoder.onnx   (waveform,duration[,language]) -> x_seg, x_est, maskT
//   segmenter.onnx (D3PM 迭代 t=0..1, 8 步)        -> boundaries (1×T bool)
//   bd2dur.onnx    (boundaries,maskT)              -> durations, maskN
//   estimator.onnx (x_est,boundaries,maskT,maskN,threshold) -> presence, scores
//
// presence[i] > 0.5 时 round(scores[i]) 为该音符 MIDI 音高。
// Mode B (外部 boundaries): known_durations -> dur2bd.onnx -> boundaries。
//
// 模型目录 (config.json + encoder/segmenter/estimator/bd2dur.onnx) 缺失时
// try_load 返回 Err, 上层必须可见地 fallback 到 LegacyFcpeNoteTracker
// (禁止伪造 source=Game 的假输出 —— Round3 审计 P0)。

use crate::models::midi_to_note_name;
use crate::note_engine::{MusicalNoteEngine, MusicalNoteEvent, MusicalNoteSource};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::{DynTensor, Tensor};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 模型文件集合 (任务书 §13)
#[derive(Debug, Clone)]
pub struct GameModelPaths {
    pub dir: PathBuf,
    pub config_path: PathBuf,
    pub encoder_path: PathBuf,
    pub segmenter_path: PathBuf,
    pub estimator_path: PathBuf,
    pub bd2dur_path: PathBuf,
    /// 外部 boundaries 模式 (Mode B) 所需的 dur2bd 模型
    pub dur2bd_path: PathBuf,
}

impl GameModelPaths {
    pub fn find_in_dir(dir: &Path) -> Option<Self> {
        let config_path = dir.join("config.json");
        let encoder_path = dir.join("encoder.onnx");
        let segmenter_path = dir.join("segmenter.onnx");
        let estimator_path = dir.join("estimator.onnx");
        let bd2dur_path = dir.join("bd2dur.onnx");
        let dur2bd_path = dir.join("dur2bd.onnx");
        if config_path.exists()
            && encoder_path.exists()
            && segmenter_path.exists()
            && estimator_path.exists()
            && bd2dur_path.exists()
            && dur2bd_path.exists()
        {
            Some(Self {
                dir: dir.to_path_buf(),
                config_path,
                encoder_path,
                segmenter_path,
                estimator_path,
                bd2dur_path,
                dur2bd_path,
            })
        } else {
            None
        }
    }
}

/// config.json (缺省值与官方 game-infer 一致)
#[derive(Debug, Clone)]
struct GameConfig {
    timestep: f32,
    sample_rate: u32,
    seg_threshold: f32,
    seg_radius_seconds: f32,
    est_threshold: f32,
}

impl GameConfig {
    fn from_json(v: &serde_json::Value) -> Self {
        Self {
            timestep: v.get("timestep").and_then(|x| x.as_f64()).unwrap_or(0.01) as f32,
            sample_rate: v
                .get("samplerate")
                .and_then(|x| x.as_u64())
                .unwrap_or(44100) as u32,
            seg_threshold: v
                .get("seg_threshold")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.2) as f32,
            seg_radius_seconds: v
                .get("seg_radius_seconds")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.02) as f32,
            est_threshold: v
                .get("est_threshold")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.2) as f32,
        }
    }
}

/// 单 chunk 推理输出 (官方 InferenceOutput)
#[derive(Debug, Default)]
struct ChunkOutput {
    _boundaries: Vec<u8>,
    durations: Vec<f32>,
    presence: Vec<f32>,
    scores: Vec<f32>,
}

pub struct GameNoteEngine {
    config: GameConfig,
    encoder: Mutex<Session>,
    segmenter: Mutex<Session>,
    estimator: Mutex<Session>,
    bd2dur: Mutex<Session>,
    /// Mode B (外部 boundaries) 所需模型
    dur2bd: Mutex<Session>,
    model_tag: String,
}

impl GameNoteEngine {
    pub fn try_load(model_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let paths = GameModelPaths::find_in_dir(model_dir)
            .ok_or_else(|| format!("GAME 模型文件未在 {:?} 找到", model_dir))?;

        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config_path)?)?;
        let config = GameConfig::from_json(&config);

        let make_session = |p: &Path| -> Result<Session, Box<dyn std::error::Error>> {
            Ok(Session::builder()?
                .with_optimization_level(GraphOptimizationLevel::Level3)?
                .with_intra_threads(num_cpus())?
                .commit_from_file(p)?)
        };

        let encoder = make_session(&paths.encoder_path)?;
        let segmenter = make_session(&paths.segmenter_path)?;
        let estimator = make_session(&paths.estimator_path)?;
        let bd2dur = make_session(&paths.bd2dur_path)?;
        let dur2bd = make_session(&paths.dur2bd_path)?;

        let model_tag = model_dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "GAME".to_string());

        Ok(Self {
            config,
            encoder: Mutex::new(encoder),
            segmenter: Mutex::new(segmenter),
            estimator: Mutex::new(estimator),
            bd2dur: Mutex::new(bd2dur),
            dur2bd: Mutex::new(dur2bd),
            model_tag,
        })
    }

    pub fn is_available(model_dir: &Path) -> bool {
        GameModelPaths::find_in_dir(model_dir).is_some()
    }

    pub fn model_tag(&self) -> &str {
        &self.model_tag
    }

    pub fn target_sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    // ── stage 1: encoder ────────────────────────────────────────
    /// (waveform,duration[,language]) -> (x_seg 数据+shape, x_est 数据+shape, maskT)
    #[allow(clippy::type_complexity)]
    fn run_encoder(
        &self,
        waveform: &[f32],
        language: i64,
    ) -> Result<(Vec<f32>, Vec<i64>, Vec<f32>, Vec<i64>, Vec<u8>), String> {
        let duration = waveform.len() as f32 / self.config.sample_rate as f32;
        let mut encoder = self
            .encoder
            .lock()
            .map_err(|_| "encoder session lock poisoned")?;
        let has_language = encoder.inputs.iter().any(|i| i.name == "language");

        let wave = Tensor::from_array(([1usize, waveform.len()], waveform.to_vec()))
            .map_err(|e| e.to_string())?;
        let dur = Tensor::from_array(([1usize], vec![duration])).map_err(|e| e.to_string())?;
        let lang = Tensor::from_array(([1usize], vec![language])).map_err(|e| e.to_string())?;

        let outputs = if has_language {
            encoder
                .run(ort::inputs![
                    "waveform" => wave,
                    "duration" => dur,
                    "language" => lang,
                ])
                .map_err(|e| e.to_string())?
        } else {
            encoder
                .run(ort::inputs![
                    "waveform" => wave,
                    "duration" => dur,
                ])
                .map_err(|e| e.to_string())?
        };

        let x_seg_arr = outputs["x_seg"]
            .try_extract_array::<f32>()
            .map_err(|e| e.to_string())?;
        let x_est_arr = outputs["x_est"]
            .try_extract_array::<f32>()
            .map_err(|e| e.to_string())?;
        let mask_arr = outputs["maskT"]
            .try_extract_array::<bool>()
            .map_err(|e| e.to_string())?;

        Ok((
            x_seg_arr.iter().copied().collect(),
            x_seg_arr.shape().iter().map(|d| *d as i64).collect(),
            x_est_arr.iter().copied().collect(),
            x_est_arr.shape().iter().map(|d| *d as i64).collect(),
            mask_arr.iter().map(|b| u8::from(*b)).collect(),
        ))
    }

    // ── stage 2: segmenter (D3PM 迭代) ──────────────────────────
    fn run_segmenter(
        &self,
        x_seg: &[f32],
        x_seg_shape: &[i64],
        language: i64,
        mask_t: &[u8],
        radius_frames: i64,
    ) -> Result<Vec<u8>, String> {
        let t = mask_t.len();
        if t == 0 {
            return Ok(Vec::new());
        }
        let threshold = self.config.seg_threshold;
        // 官方 generate_d3pm_ts(0.0, 8): 0, 1/8, .., 7/8
        let d3pm_ts: Vec<f32> = (0..8).map(|i| i as f32 / 8.0).collect();

        let mut current: Vec<u8> = vec![0; t];
        let known: Vec<u8> = vec![0; t];

        let mut segmenter = self
            .segmenter
            .lock()
            .map_err(|_| "segmenter session lock poisoned")?;
        let input_names: Vec<String> = segmenter.inputs.iter().map(|i| i.name.clone()).collect();

        for &ts in &d3pm_ts {
            // 每步重新组张量 (张量被 run 消耗)
            let mut inputs: std::collections::HashMap<String, DynTensor> =
                std::collections::HashMap::new();
            for name in &input_names {
                let v: DynTensor = match name.as_str() {
                    "x_seg" => Tensor::from_array((x_seg_shape.to_vec(), x_seg.to_vec()))
                        .map_err(|e| e.to_string())?
                        .upcast(),
                    "known_boundaries" => bool_tensor_2d(&known)?,
                    "prev_boundaries" => bool_tensor_2d(&current)?,
                    "language" => Tensor::from_array(([1usize], vec![language]))
                        .map_err(|e| e.to_string())?
                        .upcast(),
                    "maskT" => bool_tensor_2d(mask_t)?,
                    "threshold" => Tensor::from_array((Vec::<usize>::new(), vec![threshold]))
                        .map_err(|e| e.to_string())?
                        .upcast(),
                    "radius" => Tensor::from_array((Vec::<usize>::new(), vec![radius_frames]))
                        .map_err(|e| e.to_string())?
                        .upcast(),
                    "t" => Tensor::from_array(([1usize], vec![ts]))
                        .map_err(|e| e.to_string())?
                        .upcast(),
                    other => return Err(format!("unknown segmenter input: {}", other)),
                };
                inputs.insert(name.clone(), v);
            }
            let outputs = segmenter
                .run(inputs)
                .map_err(|e| format!("segmenter step t={}: {}", ts, e))?;
            let arr = outputs["boundaries"]
                .try_extract_array::<bool>()
                .map_err(|e| e.to_string())?;
            current = arr.iter().map(|b| u8::from(*b)).collect();
            if current.len() != t {
                current.resize(t, 0);
            }
        }
        Ok(current)
    }

    // ── stage 3: bd2dur ─────────────────────────────────────────
    fn run_bd2dur(&self, boundaries: &[u8], mask_t: &[u8]) -> Result<(Vec<f32>, Vec<u8>), String> {
        let mut bd2dur = self
            .bd2dur
            .lock()
            .map_err(|_| "bd2dur session lock poisoned")?;
        let outputs = bd2dur
            .run(ort::inputs![
                "boundaries" => bool_tensor_2d(boundaries)?,
                "maskT" => bool_tensor_2d(mask_t)?,
            ])
            .map_err(|e| e.to_string())?;
        let d = outputs["durations"]
            .try_extract_array::<f32>()
            .map_err(|e| e.to_string())?;
        let m = outputs["maskN"]
            .try_extract_array::<bool>()
            .map_err(|e| e.to_string())?;
        Ok((
            d.iter().copied().collect(),
            m.iter().map(|b| u8::from(*b)).collect(),
        ))
    }

    // ── stage 3b: dur2bd (Mode B 外部 boundaries) ───────────────
    fn run_dur2bd(&self, known_durations: &[f32], mask_t: &[u8]) -> Result<Vec<u8>, String> {
        let mut sess = self
            .dur2bd
            .lock()
            .map_err(|_| "dur2bd session lock poisoned")?;
        let kd = Tensor::from_array(([1usize, known_durations.len()], known_durations.to_vec()))
            .map_err(|e| e.to_string())?;
        let outputs = sess
            .run(ort::inputs![
                "durations" => kd,
                "maskT" => bool_tensor_2d(mask_t)?,
            ])
            .map_err(|e| e.to_string())?;
        let arr = outputs["boundaries"]
            .try_extract_array::<bool>()
            .map_err(|e| e.to_string())?;
        Ok(arr.iter().map(|b| u8::from(*b)).collect())
    }

    /// estimator (x_est 原始 shape 版本)
    fn run_estimator_3d(
        &self,
        x_est: &[f32],
        x_est_shape: &[i64],
        boundaries: &[u8],
        mask_t: &[u8],
        mask_n: &[u8],
    ) -> Result<(Vec<f32>, Vec<f32>), String> {
        let threshold = self.config.est_threshold;
        let mut estimator = self
            .estimator
            .lock()
            .map_err(|_| "estimator session lock poisoned")?;
        let outputs = estimator
            .run(ort::inputs![
                "x_est" => Tensor::from_array((x_est_shape.to_vec(), x_est.to_vec()))
                    .map_err(|e| e.to_string())?,
                "boundaries" => bool_tensor_2d(boundaries)?,
                "maskT" => bool_tensor_2d(mask_t)?,
                "maskN" => bool_tensor_2d(mask_n)?,
                "threshold" => Tensor::from_array((Vec::<usize>::new(), vec![threshold]))
                    .map_err(|e| e.to_string())?,
            ])
            .map_err(|e| e.to_string())?;
        let p = outputs["presence"]
            .try_extract_array::<bool>()
            .map_err(|e| e.to_string())?;
        let s = outputs["scores"]
            .try_extract_array::<f32>()
            .map_err(|e| e.to_string())?;
        Ok((
            p.iter().map(|b| if *b { 1.0 } else { 0.0 }).collect(),
            s.iter().copied().collect(),
        ))
    }

    /// 单 chunk 完整推理 (官方 forward)
    fn forward_chunk(
        &self,
        waveform: &[f32],
        language: i64,
        known_durations: Option<&[f32]>,
    ) -> Result<ChunkOutput, String> {
        let (x_seg, x_seg_shape, x_est, x_est_shape, mask_t) =
            self.run_encoder(waveform, language)?;
        let t = mask_t.len();
        if t == 0 {
            return Ok(ChunkOutput::default());
        }
        let seg_radius_frames = (self.config.seg_radius_seconds / self.config.timestep)
            .round()
            .max(1.0) as i64;

        let boundaries = match known_durations {
            Some(kd) if !kd.is_empty() => self.run_dur2bd(kd, &mask_t)?,
            _ => self.run_segmenter(&x_seg, &x_seg_shape, language, &mask_t, seg_radius_frames)?,
        };

        let (durations, mask_n) = self.run_bd2dur(&boundaries, &mask_t)?;
        if durations.is_empty() || mask_n.is_empty() {
            return Ok(ChunkOutput {
                _boundaries: boundaries,
                durations,
                presence: Vec::new(),
                scores: Vec::new(),
            });
        }
        let (presence, scores) =
            self.run_estimator_3d(&x_est, &x_est_shape, &boundaries, &mask_t, &mask_n)?;
        Ok(ChunkOutput {
            _boundaries: boundaries,
            durations,
            presence,
            scores,
        })
    }

    /// 静音切片 (官方 Game::get_midi 的 Slicer 语义简化实现):
    /// 长音频在低能量处切开, 避免 D3PM/estimator 处理超长序列
    fn slice(audio: &[f32], sample_rate: u32) -> Vec<(usize, usize)> {
        let sr = sample_rate as usize;
        let hop = (sr / 100).max(1); // 10ms
        let threshold = 0.02f32;
        let max_chunk = sr * 20;
        let min_silence = sr / 5; // 200ms
        let min_chunk = sr / 2; // 500ms

        let n_frames = audio.len().div_ceil(hop);
        let rms: Vec<f32> = (0..n_frames)
            .map(|ri| {
                let i = ri * hop;
                let end = (i + hop).min(audio.len());
                let n = (end - i).max(1);
                let s: f32 = audio[i..end].iter().map(|x| x * x).sum();
                (s / n as f32).sqrt()
            })
            .collect();

        let mut chunks: Vec<(usize, usize)> = Vec::new();
        let mut chunk_start = 0usize;
        let mut silence_run = 0usize;
        let mut silence_center: Option<usize> = None;
        for (ri, &r) in rms.iter().enumerate() {
            let pos = ((ri + 1) * hop).min(audio.len());
            if r < threshold {
                silence_run += hop;
                if silence_center.is_none() && silence_run >= min_silence / 2 {
                    silence_center = Some(pos);
                }
            } else {
                silence_run = 0;
                silence_center = None;
            }
            let chunk_len = pos - chunk_start;
            if chunk_len >= min_chunk && silence_run >= min_silence {
                if let Some(c) = silence_center {
                    if c - chunk_start >= min_chunk && audio.len() - c >= min_chunk / 4 {
                        chunks.push((chunk_start, c));
                        chunk_start = c;
                        silence_run = 0;
                        silence_center = None;
                    }
                }
            }
            if chunk_len >= max_chunk {
                chunks.push((chunk_start, pos));
                chunk_start = pos;
                silence_run = 0;
                silence_center = None;
            }
        }
        if audio.len() - chunk_start >= min_chunk / 4 {
            chunks.push((chunk_start, audio.len()));
        }
        chunks
    }
}

fn bool_tensor_2d(data: &[u8]) -> Result<DynTensor, String> {
    let bools: Vec<bool> = data.iter().map(|b| *b != 0).collect();
    let t = Tensor::from_array(([1usize, bools.len()], bools)).map_err(|e| e.to_string())?;
    Ok(t.upcast())
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .max(1)
}

impl MusicalNoteEngine for GameNoteEngine {
    fn name(&self) -> &'static str {
        "openvpi-game-v1"
    }

    fn transcribe(
        &self,
        audio: &[f32],
        sample_rate: u32,
        optional_boundaries: Option<&[f32]>,
    ) -> Result<Vec<MusicalNoteEvent>, Box<dyn std::error::Error>> {
        if audio.is_empty() {
            return Ok(Vec::new());
        }

        // 重采样到模型目标采样率 (官方 44100)
        let audio = crate::audio::resample_to(audio, sample_rate, self.config.sample_rate)?;
        let sr = self.config.sample_rate;
        let language = 0i64; // universal (0 = unknown/universal, 官方默认)

        let chunks = Self::slice(&audio, sr);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        let mut events: Vec<MusicalNoteEvent> = Vec::new();
        let mut next_id: u32 = 0;

        for (c_start, c_end) in &chunks {
            let wave = &audio[*c_start..*c_end];
            if wave.len() < (sr / 20) as usize {
                continue;
            }
            // Mode B: 外部 boundaries (绝对秒) → chunk 内 known durations (秒)
            let known_durations: Option<Vec<f32>> = optional_boundaries.and_then(|bd| {
                if bd.len() < 2 {
                    return None;
                }
                let mut in_chunk: Vec<f32> = Vec::new();
                let c0 = *c_start as f32 / sr as f32;
                let mut prev = 0.0f32;
                for &b in bd {
                    let rel = b - c0;
                    if rel > 0.05 && rel < wave.len() as f32 / sr as f32 {
                        in_chunk.push(rel - prev);
                        prev = rel;
                    }
                }
                let total = wave.len() as f32 / sr as f32;
                if total > prev + 0.05 {
                    in_chunk.push(total - prev);
                }
                (in_chunk.len() >= 2).then_some(in_chunk)
            });

            let out = self.forward_chunk(wave, language, known_durations.as_deref())?;
            if out.durations.is_empty() || out.presence.is_empty() || out.scores.is_empty() {
                continue;
            }

            // durations 单位自适应: 模型可能输出秒或帧, 按总和≈chunk 时长判定
            let sum: f32 = out.durations.iter().sum();
            let chunk_dur = wave.len() as f32 / sr as f32;
            let scale = if (sum - chunk_dur).abs() < chunk_dur * 0.2 {
                1.0
            } else if ((sum * self.config.timestep) - chunk_dur).abs() < chunk_dur * 0.2 {
                self.config.timestep
            } else {
                chunk_dur / sum.max(1e-6)
            };

            let mut t = *c_start as f32 / sr as f32;
            for i in 0..out.durations.len() {
                let d = out.durations[i] * scale;
                let (note_start, note_end) = (t, t + d);
                t = note_end;
                if out.presence.get(i).copied().unwrap_or(0.0) > 0.5 {
                    let midi_f = out.scores.get(i).copied().unwrap_or(60.0);
                    let midi = midi_f.round() as i32;
                    events.push(MusicalNoteEvent {
                        id: next_id,
                        start: note_start,
                        end: note_end,
                        midi_float: midi_f,
                        midi_rounded: midi,
                        note_name: midi_to_note_name(midi_f),
                        confidence: 0.9,
                        source: MusicalNoteSource::Game,
                        boundary_confidence: None,
                        is_slur: Some(false),
                    });
                    next_id += 1;
                }
            }
        }

        Ok(events)
    }
}
