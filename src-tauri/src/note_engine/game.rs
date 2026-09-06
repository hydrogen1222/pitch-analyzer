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
// 模型目录 (config.json + encoder/segmenter/estimator/bd2dur/dur2bd.onnx) 缺失时
// try_load 返回 Err, 上层必须可见地 fallback 到 LegacyFcpeNoteTracker
// (禁止伪造 source=Game 的假输出 —— Round3 审计 P0)。

use crate::models::midi_to_note_name;
use crate::models::SingingLanguage;
use crate::note_engine::{
    MusicalNoteEngine, MusicalNoteEvent, MusicalNoteSource, NoteTranscriptionRequest,
};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::{DynTensor, Tensor};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Reference fixture format used by the development parity harness. It is
/// intentionally model-output-only so the official GAME implementation can
/// be run outside this Rust application and compared deterministically.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameReferenceNote {
    pub start: f32,
    pub end: f32,
    pub midi: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameParityReport {
    pub reference_note_count: usize,
    pub rust_note_count: usize,
    pub pitch_agreement: f32,
    pub onset_mae: f32,
    pub offset_mae: f32,
    pub note_count_difference: f32,
}

/// Compare sorted Rust notes with the same-WAV official/reference output.
/// The acceptance thresholds mirror the Round3.2 task book and are kept in
/// one place so the harness and CI report cannot silently diverge.
pub fn compare_game_reference(
    rust_notes: &[MusicalNoteEvent],
    reference: &[GameReferenceNote],
) -> GameParityReport {
    let mut actual: Vec<&MusicalNoteEvent> = rust_notes.iter().collect();
    actual.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut expected: Vec<&GameReferenceNote> = reference.iter().collect();
    expected.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // GAME 的 segmenter 是 D3PM 扩散: 官方 decode 在每步前按 p(t) 随机丢弃
    // mutable boundaries (d3pm.rs remove_boundaries 的 torch.rand), 因此两条
    // 推理在临界 boundary 上有 ±1 音符的固有方差。直接 zip 会让一次错位
    // 污染全部后续 MAE; 这里用贪心最近邻配对 (onset 差 <= 0.25s)。
    const MATCH_TOLERANCE: f32 = 0.25;
    let mut pairs: Vec<(&MusicalNoteEvent, &GameReferenceNote)> = Vec::new();
    {
        let mut used_ref = vec![false; expected.len()];
        for a in &actual {
            let mut best: Option<(usize, f32)> = None;
            for (ri, b) in expected.iter().enumerate() {
                if used_ref[ri] {
                    continue;
                }
                let d = (a.start - b.start).abs();
                if d <= MATCH_TOLERANCE && best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((ri, d));
                }
            }
            if let Some((ri, _)) = best {
                used_ref[ri] = true;
                pairs.push((a, expected[ri]));
            }
        }
    }

    let compared = pairs.len();
    let pitch_agreement = if compared == 0 {
        if actual.is_empty() && expected.is_empty() {
            1.0
        } else {
            0.0
        }
    } else {
        pairs
            .iter()
            .filter(|(a, b)| (a.midi_float - b.midi).abs() <= 0.5)
            .count() as f32
            / compared as f32
    };
    let onset_mae = if compared == 0 {
        f32::INFINITY
    } else {
        pairs
            .iter()
            .map(|(a, b)| (a.start - b.start).abs())
            .sum::<f32>()
            / compared as f32
    };
    let offset_mae = if compared == 0 {
        f32::INFINITY
    } else {
        pairs
            .iter()
            .map(|(a, b)| (a.end - b.end).abs())
            .sum::<f32>()
            / compared as f32
    };
    let note_count_difference = if reference.is_empty() {
        if rust_notes.is_empty() {
            0.0
        } else {
            f32::INFINITY
        }
    } else {
        (rust_notes.len() as f32 - reference.len() as f32).abs() / reference.len() as f32
    };
    GameParityReport {
        reference_note_count: reference.len(),
        rust_note_count: rust_notes.len(),
        pitch_agreement,
        onset_mae,
        offset_mae,
        note_count_difference,
    }
}

pub fn assert_game_parity(report: &GameParityReport) -> Result<(), String> {
    if report.pitch_agreement < 0.90
        || report.onset_mae > 0.050
        || report.offset_mae > 0.080
        || report.note_count_difference > 0.10
    {
        return Err(format!(
            "GAME parity failed: pitch={:.3}, onset={:.3}s, offset={:.3}s, count_delta={:.3}",
            report.pitch_agreement,
            report.onset_mae,
            report.offset_mae,
            report.note_count_difference
        ));
    }
    Ok(())
}

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

/// Chunking 配置 (Round4 Commit 10, P1-5): 数值不写死在函数里,
/// 可通过模型 config.json 的 "chunking" 对象覆盖。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameChunkingConfig {
    /// 单 chunk 的核心区上限 (秒); 建议先 benchmark 18-20s
    pub max_core_secs: f32,
    /// 相邻 chunk 的 context 重叠 (秒)
    pub overlap_secs: f32,
    /// 静音判定 RMS 阈值
    pub silence_threshold: f32,
    /// 最短静音段 (秒), 优先在此类位置切块
    pub min_silence_secs: f32,
    /// 跨块 reconcile: 同音高事件间隙 <= 该值合并 (同一持续音被边界切开)
    pub reconcile_gap_secs: f32,
}

impl Default for GameChunkingConfig {
    fn default() -> Self {
        Self {
            max_core_secs: 20.0,
            overlap_secs: 1.0,
            silence_threshold: 0.02,
            min_silence_secs: 0.2,
            reconcile_gap_secs: 0.05,
        }
    }
}

impl GameChunkingConfig {
    fn from_json(v: &serde_json::Value) -> Self {
        let get = |key: &str, fallback: f32| -> f32 {
            v.get("chunking")
                .and_then(|c| c.get(key))
                .and_then(|x| x.as_f64())
                .map(|x| x as f32)
                .unwrap_or(fallback)
        };
        Self {
            max_core_secs: get("max_core_secs", 20.0),
            overlap_secs: get("overlap_secs", 1.0),
            silence_threshold: get("silence_threshold", 0.02),
            min_silence_secs: get("min_silence_secs", 0.2),
            reconcile_gap_secs: get("reconcile_gap_secs", 0.05),
        }
    }
}

/// 一个推理块: [context_start | core_start .. core_end | context_end] (采样点)。
/// 推理覆盖 context; note 起始边界只承认 core 内的。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkSpan {
    pub context_start: usize,
    pub core_start: usize,
    pub core_end: usize,
    pub context_end: usize,
}

/// config.json (缺省值与官方 game-infer 一致)。
/// `languages` 是模型包自带的 id 映射 (官方 ONNX.md: en/ja/zh → 数字),
/// 不同模型包映射可能不同, 必须动态读取, 禁止硬编码 ja=2 / zh=3。
#[derive(Debug, Clone)]
struct GameConfig {
    timestep: f32,
    sample_rate: u32,
    seg_threshold: f32,
    seg_radius_seconds: f32,
    est_threshold: f32,
    languages: HashMap<String, i64>,
    chunking: GameChunkingConfig,
}

impl GameConfig {
    fn from_json(v: &serde_json::Value) -> Self {
        let languages = v
            .get("languages")
            .and_then(|x| x.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(key, value)| value.as_i64().map(|id| (key.to_lowercase(), id)))
                    .collect()
            })
            .unwrap_or_default();
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
            languages,
            chunking: GameChunkingConfig::from_json(v),
        }
    }

    /// 解析 SingingLanguage → 模型 language id (Round4 Commit 3, P0-2)。
    /// 规则 (任务书 §3 P0-2):
    ///   Japanese -> lookup "ja"; Mandarin -> lookup "zh";
    ///   Unknown/Auto unresolved -> 0;
    ///   model config 无该语言 -> 0 + diagnostic warning。
    fn language_id(&self, language: SingingLanguage) -> i64 {
        let Some(key) = (match language {
            SingingLanguage::Japanese => Some("ja"),
            SingingLanguage::Mandarin => Some("zh"),
            // universal / unset: 官方语义 0 = unset
            SingingLanguage::Unknown | SingingLanguage::Auto => None,
        }) else {
            return 0;
        };
        match self.languages.get(key).copied() {
            Some(id) => {
                eprintln!(
                    "[game] language {} -> model id {} (from model config)",
                    key, id
                );
                id
            }
            None => {
                eprintln!(
                    "[game] model config has no id for language '{key}' (languages: {:?}); \
                     falling back to universal id 0",
                    self.languages
                );
                0
            }
        }
    }
}

/// 单 chunk 推理输出 (官方 InferenceOutput)
#[derive(Debug, Default)]
struct ChunkOutput {
    _boundaries: Vec<u8>,
    durations: Vec<f32>,
    mask_n: Vec<u8>,
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
                mask_n,
                presence: Vec::new(),
                scores: Vec::new(),
            });
        }
        let (presence, scores) =
            self.run_estimator_3d(&x_est, &x_est_shape, &boundaries, &mask_t, &mask_n)?;
        Ok(ChunkOutput {
            _boundaries: boundaries,
            durations,
            mask_n,
            presence,
            scores,
        })
    }

    /// Overlap 切块规划 (Round4 Commit 10, P1-5):
    /// 优先在静音处切; 无静音可用时才硬切 max_core。
    /// 每个 chunk 带 [context | core | context] 结构:
    /// 推理跑整个 context, 但 note 只在 core 内拥有起始边界 ——
    /// 左/右 context 中的边界由相邻 chunk 负责 (丢弃边缘低可信边界),
    /// 跨块同一持续音由 reconcile 合并。
    fn plan_chunks(audio: &[f32], sample_rate: u32, cfg: &GameChunkingConfig) -> Vec<ChunkSpan> {
        let sr = sample_rate as usize;
        let hop = (sr / 100).max(1);
        let max_core = ((cfg.max_core_secs * sr as f32) as usize).max(hop);
        let overlap = (cfg.overlap_secs * sr as f32) as usize;
        let min_silence = (cfg.min_silence_secs * sr as f32) as usize;
        let min_chunk = (sr / 2).max(1);
        let audio_len = audio.len();
        if audio_len == 0 {
            return Vec::new();
        }

        let n_frames = audio_len.div_ceil(hop);
        let rms: Vec<f32> = (0..n_frames)
            .map(|ri| {
                let i = ri * hop;
                let end = (i + hop).min(audio_len);
                let n = (end - i).max(1);
                let s: f32 = audio[i..end].iter().map(|x| x * x).sum();
                (s / n as f32).sqrt()
            })
            .collect();

        let mut cuts: Vec<usize> = vec![0];
        let mut current = 0usize;
        while audio_len - current > max_core {
            let upper = (current + max_core).min(audio_len);
            let search_from = current + min_chunk;
            // 静音优先: 在 (current+min_chunk, upper] 内找最后一个静音中心
            let cut = Self::last_silence_center(&rms, hop, search_from, upper, min_silence, cfg)
                .filter(|&c| c - current >= min_chunk && audio_len - c >= min_chunk / 4)
                .unwrap_or(upper);
            cuts.push(cut);
            current = cut;
        }
        cuts.push(audio_len);

        let mut chunks = Vec::with_capacity(cuts.len() - 1);
        for window in cuts.windows(2) {
            let core_start = window[0];
            let core_end = window[1];
            let context_start = if core_start == 0 {
                0
            } else {
                core_start.saturating_sub(overlap)
            };
            let context_end = if core_end == audio_len {
                audio_len
            } else {
                (core_end + overlap).min(audio_len)
            };
            chunks.push(ChunkSpan {
                context_start,
                core_start,
                core_end,
                context_end,
            });
        }
        chunks
    }

    /// 在 [from, to) 帧范围内寻找最后一个 >= min_silence 的静音段中心
    fn last_silence_center(
        rms: &[f32],
        hop: usize,
        from: usize,
        to: usize,
        min_silence: usize,
        cfg: &GameChunkingConfig,
    ) -> Option<usize> {
        let mut silence_run = 0usize;
        let mut last_center: Option<usize> = None;
        for (ri, &r) in rms.iter().enumerate() {
            let pos = ((ri + 1) * hop).min(to.max(1));
            if pos <= from {
                if r < cfg.silence_threshold {
                    continue; // 起点之前的静音不计入本段
                }
                silence_run = 0;
                continue;
            }
            if r < cfg.silence_threshold {
                silence_run += hop;
                if silence_run >= min_silence {
                    let center = pos.saturating_sub(silence_run / 2);
                    if center > from && center < to {
                        last_center = Some(center);
                    }
                }
            } else {
                silence_run = 0;
            }
            if pos >= to {
                break;
            }
        }
        last_center
    }

    /// 跨块 reconcile (Round4 Commit 10): 相邻同音高且间隙极小的
    /// 事件合并 (同一持续音被 chunk 边界切开的情况)。
    fn reconcile_chunk_notes(
        mut events: Vec<MusicalNoteEvent>,
        merge_gap_secs: f32,
    ) -> Vec<MusicalNoteEvent> {
        events.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut output: Vec<MusicalNoteEvent> = Vec::new();
        for event in events {
            if let Some(last) = output.last_mut() {
                let same_pitch = last.midi_rounded == event.midi_rounded;
                let contiguous = event.start <= last.end + merge_gap_secs;
                if same_pitch && contiguous {
                    last.end = last.end.max(event.end);
                    // 事件时长加权平均 midi
                    let l_dur = (last.end - last.start).max(1e-3);
                    let r_dur = (event.end - event.start).max(1e-3);
                    last.midi_float =
                        (last.midi_float * l_dur + event.midi_float * r_dur) / (l_dur + r_dur);
                    last.midi_rounded = last.midi_float.round() as i32;
                    last.note_name = midi_to_note_name(last.midi_float);
                    continue;
                }
            }
            output.push(event);
        }
        output
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

impl GameNoteEngine {
    /// 测试辅助事件 (chunking tests)
    fn test_event(id: u32, start: f32, end: f32, midi: f32) -> MusicalNoteEvent {
        MusicalNoteEvent {
            id,
            start,
            end,
            midi_float: midi,
            midi_rounded: midi.round() as i32,
            note_name: midi_to_note_name(midi),
            confidence: 0.0,
            source: MusicalNoteSource::Game,
            model_confidence: None,
            boundary_confidence: None,
            is_slur: None,
            evidence: None,
            class: None,
        }
    }
}

impl MusicalNoteEngine for GameNoteEngine {
    fn name(&self) -> &'static str {
        "openvpi-game-v1"
    }

    fn transcribe(
        &self,
        request: &NoteTranscriptionRequest,
    ) -> Result<Vec<MusicalNoteEvent>, Box<dyn std::error::Error>> {
        let audio = request.audio;
        let sample_rate = request.sample_rate;
        let optional_boundaries = request.known_boundaries;
        if audio.is_empty() {
            return Ok(Vec::new());
        }

        // 重采样到模型目标采样率 (官方 44100)
        let audio = crate::audio::resample_to(audio, sample_rate, self.config.sample_rate)?;
        let sr = self.config.sample_rate;
        // P0-2: 语言 id 来自模型 config, 不再硬编码 0
        let language = self.config.language_id(request.language);

        // Round4 Commit 10 (P1-5): overlap 切块规划 (静音优先, 硬切兜底)
        let chunking = self.config.chunking;
        let chunks = Self::plan_chunks(&audio, sr, &chunking);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        let mut events: Vec<MusicalNoteEvent> = Vec::new();
        let mut next_id: u32 = 0;

        for chunk in &chunks {
            let wave = &audio[chunk.context_start..chunk.context_end];
            if wave.len() < (sr / 20) as usize {
                continue;
            }
            // Mode B: 外部 boundaries (绝对秒) → known durations。
            // Mode B 语义定义在全曲边界上, 与 chunked ownership 冲突时
            // 以单 chunk (全曲) 方式处理: 即 context == core == 全曲。
            let known_durations: Option<Vec<f32>> = optional_boundaries.and_then(|bd| {
                if bd.len() < 2 {
                    return None;
                }
                let mut in_chunk: Vec<f32> = Vec::new();
                let c0 = chunk.context_start as f32 / sr as f32;
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

            // 官方 GAME 的 bd2dur 输出已经是秒；Game::get_midi 直接把
            // durations 累加到音符边界，不根据 chunk 时长二次归一化。
            // 这点必须保持固定，否则不同长度切片会产生与官方实现不同的
            // onset/offset。maskN 只保留有效的 note slots。
            let valid_durations: Vec<f32> = out
                .durations
                .iter()
                .enumerate()
                .filter_map(|(i, duration)| {
                    (out.mask_n.get(i).copied().unwrap_or(0) != 0).then_some(*duration)
                })
                .collect();
            let sum: f32 = valid_durations.iter().sum();
            let chunk_dur = wave.len() as f32 / sr as f32;
            if !sum.is_finite() || sum <= 0.0 {
                return Err("GAME bd2dur returned no positive valid duration"
                    .to_string()
                    .into());
            }
            if (sum - chunk_dur).abs() > chunk_dur * 0.25 {
                return Err(format!(
                    "GAME bd2dur duration mismatch: output={sum:.4}s, chunk={chunk_dur:.4}s"
                )
                .into());
            }

            // 事件时间从 context 起点累计; 归属: 起始边界必须在 core 内
            // (P1-5: 左/右 context 中的边界由相邻 chunk 拥有, 此处丢弃)
            let core_start_secs = chunk.core_start as f32 / sr as f32;
            let core_end_secs = chunk.core_end as f32 / sr as f32;
            let mut t = chunk.context_start as f32 / sr as f32;
            for i in 0..out.durations.len() {
                if out.mask_n.get(i).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let d = out.durations[i];
                if !d.is_finite() || d <= 0.0 {
                    return Err(
                        format!("GAME bd2dur returned invalid duration at slot {i}: {d}").into(),
                    );
                }
                let (note_start, note_end) = (t, t + d);
                t = note_end;
                let owns_onset = note_start >= core_start_secs && note_start < core_end_secs;
                if out.presence.get(i).copied().unwrap_or(0.0) > 0.5 && owns_onset {
                    let midi_f = out.scores.get(i).copied().unwrap_or(60.0);
                    let midi = midi_f.round() as i32;
                    events.push(MusicalNoteEvent {
                        id: next_id,
                        start: note_start,
                        end: note_end,
                        midi_float: midi_f,
                        midi_rounded: midi,
                        note_name: midi_to_note_name(midi_f),
                        // The released GAME ONNX estimator exposes a hard
                        // presence decision, not a calibrated confidence.
                        // Keep this raw candidate unscored; FCPE evidence is
                        // attached by CanonicalNotePostProcessor.
                        confidence: 0.0,
                        source: MusicalNoteSource::Game,
                        model_confidence: None,
                        boundary_confidence: None,
                        is_slur: None,
                        evidence: None,
                        class: None,
                    });
                    next_id += 1;
                }
            }
        }

        // 跨块 reconcile: 同一持续音被 chunk 边界切开 → 合并 (P1-5)
        let merged = Self::reconcile_chunk_notes(events, chunking.reconcile_gap_secs);
        let mut events: Vec<MusicalNoteEvent> = Vec::with_capacity(merged.len());
        for (id, mut event) in merged.into_iter().enumerate() {
            event.id = id as u32;
            events.push(event);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod language_config_tests {
    use super::*;

    #[test]
    fn parses_official_language_map() {
        // 官方 GAME ONNX.md 示例映射; 本仓库捆绑模型包不得假设此映射
        let config = GameConfig::from_json(&serde_json::json!({
            "timestep": 0.01,
            "languages": { "en": 1, "ja": 2, "zh": 3 }
        }));
        assert_eq!(config.language_id(SingingLanguage::Japanese), 2);
        assert_eq!(config.language_id(SingingLanguage::Mandarin), 3);
        // Unknown/Auto → universal id 0
        assert_eq!(config.language_id(SingingLanguage::Unknown), 0);
        assert_eq!(config.language_id(SingingLanguage::Auto), 0);
    }

    #[test]
    fn different_model_mapping_is_respected_not_hardcoded() {
        let config = GameConfig::from_json(&serde_json::json!({
            "languages": { "ja": 7, "zh": 9 }
        }));
        assert_eq!(config.language_id(SingingLanguage::Japanese), 7);
        assert_eq!(config.language_id(SingingLanguage::Mandarin), 9);
    }

    #[test]
    fn missing_languages_block_falls_back_to_universal() {
        let config = GameConfig::from_json(&serde_json::json!({ "timestep": 0.01 }));
        assert!(config.languages.is_empty());
        assert_eq!(config.language_id(SingingLanguage::Japanese), 0);
        assert_eq!(config.language_id(SingingLanguage::Mandarin), 0);
    }

    #[test]
    fn missing_language_key_warns_and_falls_back() {
        let config = GameConfig::from_json(&serde_json::json!({
            "languages": { "en": 1 }
        }));
        // ja/zh 不在映射中 → 0 (diagnostic warning 由 language_id 输出)
        assert_eq!(config.language_id(SingingLanguage::Japanese), 0);
        assert_eq!(config.language_id(SingingLanguage::Mandarin), 0);
        assert_eq!(config.language_id(SingingLanguage::Unknown), 0);
    }

    #[test]
    fn non_integer_language_values_are_ignored() {
        let config = GameConfig::from_json(&serde_json::json!({
            "languages": { "ja": "two", "zh": 3.5, "en": 1 }
        }));
        assert_eq!(config.languages.get("en"), Some(&1));
        assert!(!config.languages.contains_key("ja"));
        assert!(!config.languages.contains_key("zh"));
        assert_eq!(config.language_id(SingingLanguage::Japanese), 0);
    }
}

#[cfg(test)]
mod chunking_tests {
    use super::*;

    fn sr() -> u32 {
        100 // 测试用低采样率: 1 秒 = 100 样本, 10ms hop = 1 样本
    }

    fn tone(len_secs: f32, amp: f32) -> Vec<f32> {
        let n = (len_secs * 100.0) as usize;
        vec![amp; n.max(1)]
    }

    #[test]
    fn short_audio_single_chunk_no_context() {
        let cfg = GameChunkingConfig::default();
        let audio = tone(5.0, 0.5);
        let chunks = GameNoteEngine::plan_chunks(&audio, sr(), &cfg);
        assert_eq!(chunks.len(), 1);
        let c = chunks[0];
        assert_eq!((c.context_start, c.core_start), (0, 0));
        assert_eq!((c.core_end, c.context_end), (audio.len(), audio.len()));
    }

    /// 长音频: 多 chunk; core 连续; 非 首/末 chunk 有对称 context;
    /// context 扩展不超过 overlap
    #[test]
    fn long_audio_planned_with_overlap_context() {
        let cfg = GameChunkingConfig {
            max_core_secs: 2.0,
            overlap_secs: 0.5,
            ..Default::default()
        };
        let audio = tone(6.0, 0.5); // 3 chunks of 2s
        let chunks = GameNoteEngine::plan_chunks(&audio, sr(), &cfg);
        assert_eq!(chunks.len(), 3);
        // core 连续覆盖
        assert_eq!(chunks[0].core_start, 0);
        assert_eq!(chunks[1].core_start, chunks[0].core_end);
        assert_eq!(chunks[2].core_start, chunks[1].core_end);
        assert_eq!(chunks[2].core_end, audio.len());
        // context: 首块无左 context; 中间块左右各 0.5s (50 样本)
        assert_eq!(chunks[0].context_start, 0);
        assert_eq!(chunks[1].context_start, chunks[1].core_start - 50);
        assert_eq!(chunks[1].context_end, chunks[1].core_end + 50);
        assert_eq!(chunks[2].context_end, audio.len());
    }

    /// P1-5: 无静音可用时才硬切; 有静音时优先在静音处切
    #[test]
    fn silence_preferred_over_hard_cut() {
        let cfg = GameChunkingConfig {
            max_core_secs: 2.0,
            overlap_secs: 0.5,
            silence_threshold: 0.02,
            min_silence_secs: 0.2,
            reconcile_gap_secs: 0.05,
        };
        // 1.5s tone + 0.4s silence + 5s tone: 硬切点 2.0s 落在静音段 (1.5-1.9s) 附近
        let mut audio = tone(1.5, 0.5);
        audio.extend(vec![0.0; 40]); // 0.4s 静音
        audio.extend(tone(5.0, 0.5));
        let chunks = GameNoteEngine::plan_chunks(&audio, sr(), &cfg);
        assert!(chunks.len() >= 2);
        // 第一个 cut 应在静音段附近 (1.5-1.9s => 150-190 样本), 而非正好 200
        let first_cut = chunks[0].core_end;
        assert!(
            (150..=200).contains(&first_cut),
            "cut should land in the silence gap, got {first_cut}"
        );
    }

    /// reconcile: 边界两侧同音高事件合并; 不同音高不合并
    #[test]
    fn reconcile_merges_split_sustained_notes() {
        let events = vec![
            GameNoteEngine::test_event(0, 0.0, 2.0, 60.0),
            GameNoteEngine::test_event(1, 2.0, 4.0, 60.0), // 同音被切开
            GameNoteEngine::test_event(2, 4.0, 5.0, 62.0),
        ];
        let merged = GameNoteEngine::reconcile_chunk_notes(events, 0.05);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start, 0.0);
        assert_eq!(merged[0].end, 4.0);
        assert_eq!(merged[0].midi_rounded, 60);
        assert_eq!(merged[1].midi_rounded, 62);
        // 间隙超过 merge_gap 不合并
        let events = vec![
            GameNoteEngine::test_event(0, 0.0, 1.0, 60.0),
            GameNoteEngine::test_event(1, 1.5, 2.0, 60.0),
        ];
        let merged = GameNoteEngine::reconcile_chunk_notes(events, 0.05);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn chunking_config_from_json() {
        let cfg = GameChunkingConfig::from_json(&serde_json::json!({
            "chunking": { "max_core_secs": 18.0, "overlap_secs": 1.5 }
        }));
        assert_eq!(cfg.max_core_secs, 18.0);
        assert_eq!(cfg.overlap_secs, 1.5);
        assert_eq!(cfg.silence_threshold, 0.02); // 缺省
        let cfg = GameChunkingConfig::from_json(&serde_json::json!({}));
        assert_eq!(cfg, GameChunkingConfig::default());
    }
}
