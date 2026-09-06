// Legacy MMS-FA backend with a persistent worker (Round4 Commit 8, P0-4)
//
// 旧实现每行启动一个 Python + 重新加载音频 + 重新加载模型, 生产不可接受。
// 现改为长生命周期 JSONL worker (scripts/legacy_mms_worker.py):
//   - 模型只加载一次 (init)
//   - 音频只 decode/resample 一次 (init, 按音频路径缓存)
//   - 一次 align() 请求可批处理多段 (§27.3: 结果带 segment_id/unit_id)
//
// 定位 (§10.4): Legacy Experimental — torchaudio FA API 已在 2.8 deprecated;
// 不作为长期生产主线, 生产方向是 singing-domain FA (singing_fa.rs)。

use super::types::{
    AlignedUnit, AlignmentError, AlignmentRequest, AlignmentResult, BackendCapabilities,
    ExpectedUnit, SegmentAlignmentResult,
};
use super::{
    validate_aligned_spans, AlignedMora, ForcedAlignBackend, LineAlignmentResult,
    SongForcedAlignBackend,
};
use crate::models::{AlignmentSource, LyricLine, SingingLanguage};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// 窗口容差: 只吸收帧量化误差 (§12.2: 不是跨音节借音)
const WINDOW_TOLERANCE_SECS: f32 = 0.02;

#[derive(Debug, Serialize)]
struct WorkerRequest<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    segments: Option<Vec<WorkerSegment<'a>>>,
}

#[derive(Debug, Serialize)]
struct WorkerSegment<'a> {
    id: usize,
    window_start: f32,
    window_end: f32,
    units: Vec<WorkerUnit<'a>>,
}

#[derive(Debug, Serialize)]
struct WorkerUnit<'a> {
    id: usize,
    symbols: &'a [String],
}

#[derive(Debug, Deserialize)]
struct WorkerResponse {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    segments: Vec<WorkerSegmentResult>,
}

#[derive(Debug, Deserialize)]
struct WorkerSegmentResult {
    segment_id: usize,
    units: Vec<WorkerUnitResult>,
}

#[derive(Debug, Deserialize)]
struct WorkerUnitResult {
    unit_id: usize,
    start: f32,
    end: f32,
    #[serde(default)]
    raw_score: Option<f32>,
}

struct Worker {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    audio_path: String,
}

impl Worker {
    fn spawn(python: &Path, helper: &Path, audio_path: &str) -> Result<Self, AlignmentError> {
        let mut child = Command::new(python)
            .arg(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| AlignmentError::DependencyMissing(format!("spawn FA worker: {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(AlignmentError::BackendUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(AlignmentError::BackendUnavailable)?;
        let mut worker = Worker {
            child,
            stdin,
            reader: BufReader::new(stdout),
            audio_path: audio_path.to_string(),
        };
        let init = WorkerRequest {
            kind: "init",
            audio_path: Some(audio_path),
            segments: None,
        };
        worker.send(&init)?;
        let response: WorkerResponse = worker
            .read_response()
            .ok_or(AlignmentError::BackendUnavailable)?;
        if response.kind == "error" {
            return Err(worker_error(response));
        }
        if response.kind != "ready" || response.model.is_empty() {
            return Err(AlignmentError::InvalidModelOutput(
                "worker did not report ready".to_string(),
            ));
        }
        Ok(worker)
    }

    fn send<T: Serialize>(&mut self, value: &T) -> Result<(), AlignmentError> {
        let line = serde_json::to_string(value)
            .map_err(|e| AlignmentError::InvalidModelOutput(e.to_string()))?;
        writeln!(self.stdin, "{line}").map_err(|_| AlignmentError::BackendUnavailable)?;
        self.stdin
            .flush()
            .map_err(|_| AlignmentError::BackendUnavailable)
    }

    fn read_response(&mut self) -> Option<WorkerResponse> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(line.trim()).ok(),
        }
    }

    fn shutdown(mut self) {
        let _ = self.send(&WorkerRequest {
            kind: "shutdown",
            audio_path: None,
            segments: None,
        });
        let _ = self.child.wait();
    }
}

fn worker_error(response: WorkerResponse) -> AlignmentError {
    match response.code.as_str() {
        "model_load_failed" => AlignmentError::ModelLoadFailed(response.message),
        "dependency_missing" => AlignmentError::DependencyMissing(response.message),
        "audio_invalid" => AlignmentError::AudioWindowInvalid,
        "not_ready" | "invalid_request" => AlignmentError::InvalidModelOutput(response.message),
        _ => AlignmentError::InvalidModelOutput(response.message),
    }
}

pub struct MmsForcedAlignBackend {
    python: PathBuf,
    helper: PathBuf,
    state: Mutex<Option<Worker>>,
}

impl std::fmt::Debug for MmsForcedAlignBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmsForcedAlignBackend")
            .field("python", &self.python)
            .field("helper", &self.helper)
            .finish()
    }
}

impl MmsForcedAlignBackend {
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_with_helper(None)
    }

    pub fn from_env_with_helper(resource_helper: Option<&Path>) -> Result<Self, String> {
        let python = std::env::var_os("PITCH_ANALYZER_FA_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(default_python);
        let helper = std::env::var_os("PITCH_ANALYZER_FA_HELPER")
            .map(PathBuf::from)
            .or_else(|| resource_helper.map(Path::to_path_buf))
            .unwrap_or_else(default_worker_helper);
        let backend = Self::with_explicit_paths(&python, &helper);
        if !helper.is_file() {
            return Err(format!("MMS FA worker 不存在: {}", helper.display()));
        }
        Ok(backend)
    }

    /// 显式指定 python 与 helper 脚本 (Tier B 协议测试 / 高级配置)
    pub fn with_explicit_paths(python: &Path, helper: &Path) -> Self {
        Self {
            python: python.to_path_buf(),
            helper: helper.to_path_buf(),
            state: Mutex::new(None),
        }
    }

    pub fn helper_path(&self) -> &Path {
        &self.helper
    }

    /// 确保存在已初始化 (模型+音频已加载) 的 worker; 音频变化时重建。
    fn ensure_worker(&self, audio_path: &str) -> Result<(), AlignmentError> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| AlignmentError::BackendUnavailable)?;
        let needs_restart = match guard.as_ref() {
            Some(worker) => worker.audio_path != audio_path,
            None => true,
        };
        if needs_restart {
            if let Some(old) = guard.take() {
                old.shutdown();
            }
            let worker = Worker::spawn(&self.python, &self.helper, audio_path)?;
            *guard = Some(worker);
        }
        Ok(())
    }

    fn align_via_worker(
        &self,
        request: &AlignmentRequest,
        audio_path: &str,
    ) -> Result<AlignmentResult, AlignmentError> {
        if request.segments.is_empty() {
            return Ok(AlignmentResult {
                backend: "torchaudio-mms-fa".to_string(),
                segments: Vec::new(),
            });
        }
        // 预校验 (Tier B: 无效语言 / 无效窗口 / 空发音 不进 worker)
        for segment in &request.segments {
            if segment.window_end.partial_cmp(&segment.window_start) != Some(Ordering::Greater) {
                return Err(AlignmentError::AudioWindowInvalid);
            }
            if segment.units.is_empty() {
                return Err(AlignmentError::MissingPronunciation);
            }
        }

        self.ensure_worker(audio_path)?;
        let mut guard = self
            .state
            .lock()
            .map_err(|_| AlignmentError::BackendUnavailable)?;
        let worker = guard.as_mut().ok_or(AlignmentError::BackendUnavailable)?;

        let worker_segments: Vec<WorkerSegment> = request
            .segments
            .iter()
            .map(|segment| WorkerSegment {
                id: segment.id,
                window_start: segment.window_start,
                window_end: segment.window_end,
                units: segment
                    .units
                    .iter()
                    .map(|unit| WorkerUnit {
                        id: unit.id,
                        symbols: &unit.symbols,
                    })
                    .collect(),
            })
            .collect();
        let payload = WorkerRequest {
            kind: "align",
            audio_path: None,
            segments: Some(worker_segments),
        };

        worker.send(&payload)?;
        let response = match worker.read_response() {
            Some(response) => response,
            None => {
                // worker 死亡: 重建一次再试 (幂等: align 无副作用)
                drop(guard);
                self.ensure_worker(audio_path)?;
                let mut guard = self
                    .state
                    .lock()
                    .map_err(|_| AlignmentError::BackendUnavailable)?;
                let worker = guard.as_mut().ok_or(AlignmentError::BackendUnavailable)?;
                worker.send(&payload)?;
                worker
                    .read_response()
                    .ok_or(AlignmentError::BackendUnavailable)?
            }
        };
        if response.kind == "error" {
            return Err(worker_error(response));
        }
        if response.kind != "result" {
            return Err(AlignmentError::InvalidModelOutput(format!(
                "unexpected worker kind: {}",
                response.kind
            )));
        }
        if response.segments.len() != request.segments.len() {
            return Err(AlignmentError::InvalidModelOutput(format!(
                "worker returned {} segments, expected {}",
                response.segments.len(),
                request.segments.len()
            )));
        }

        let mut segments = Vec::with_capacity(response.segments.len());
        for (returned, requested) in response.segments.iter().zip(request.segments.iter()) {
            if returned.segment_id != requested.id {
                return Err(AlignmentError::InvalidModelOutput(format!(
                    "segment id mismatch: {} != {}",
                    returned.segment_id, requested.id
                )));
            }
            if returned.units.len() != requested.units.len() {
                return Err(AlignmentError::InvalidModelOutput(format!(
                    "segment {}: unit count mismatch",
                    requested.id
                )));
            }
            let spans: Vec<(f32, f32)> = returned.units.iter().map(|u| (u.start, u.end)).collect();
            validate_aligned_spans(
                &spans,
                requested.window_start,
                requested.window_end,
                WINDOW_TOLERANCE_SECS,
            )?;
            for unit in &returned.units {
                if !requested
                    .units
                    .iter()
                    .any(|expected| expected.id == unit.unit_id)
                {
                    return Err(AlignmentError::InvalidModelOutput(format!(
                        "segment {}: unknown unit id {}",
                        requested.id, unit.unit_id
                    )));
                }
            }
            segments.push(SegmentAlignmentResult {
                segment_id: returned.segment_id,
                units: returned
                    .units
                    .iter()
                    .map(|u| AlignedUnit {
                        unit_id: u.unit_id,
                        start: u.start,
                        end: u.end,
                        raw_score: u.raw_score,
                    })
                    .collect(),
            });
        }

        Ok(AlignmentResult {
            backend: "torchaudio-mms-fa".to_string(),
            segments,
        })
    }
}

fn default_python() -> PathBuf {
    PathBuf::from(if cfg!(target_os = "windows") {
        "python"
    } else {
        "python3"
    })
}

fn default_worker_helper() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("legacy_mms_worker.py")
}

/// §10.1: 整曲对齐 trait 实现 (长生命周期)
impl super::SongForcedAlignBackend for MmsForcedAlignBackend {
    fn name(&self) -> &'static str {
        "torchaudio-mms-fa"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            languages: vec![
                SingingLanguage::Japanese,
                SingingLanguage::Mandarin,
                SingingLanguage::Unknown,
            ],
            unit_level: super::AlignmentGranularity::Mora,
            singing_domain: false,
            persistent: true,
        }
    }

    fn align(&self, request: &AlignmentRequest) -> Result<AlignmentResult, AlignmentError> {
        // MMS 是多语种 general-speech 模型: ja/zh/Unknown 可用,
        // 其余显式拒绝 (不允许默默当成 universal)
        match request.language {
            SingingLanguage::Japanese
            | SingingLanguage::Mandarin
            | SingingLanguage::Unknown
            | SingingLanguage::Auto => {}
        }
        self.align_via_worker(request, &request.audio.path)
    }
}

/// 旧行级 trait: 通过持久 worker 实现 (每行不再重复 spawn/load)
impl ForcedAlignBackend for MmsForcedAlignBackend {
    fn name(&self) -> &'static str {
        "torchaudio-mms-fa"
    }

    fn align_line(
        &self,
        line: &LyricLine,
        _track: &crate::models::PitchTrack,
        audio_path: &Path,
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String> {
        if line.moras.is_empty() {
            return Err("该歌词行没有可用于 FA 的 mora".to_string());
        }
        let units: Vec<ExpectedUnit> = line
            .moras
            .iter()
            .enumerate()
            .map(|(id, mora)| ExpectedUnit {
                id,
                symbols: mora.phonemes.clone(),
            })
            .collect();
        let request = AlignmentRequest {
            language: SingingLanguage::Unknown,
            sample_rate: 0,
            audio: std::sync::Arc::new(super::types::AudioRef {
                path: audio_path.to_string_lossy().to_string(),
            }),
            segments: vec![super::types::AlignmentSegmentRequest {
                id: 0,
                window_start,
                window_end,
                units,
            }],
        };
        let result = self.align(&request).map_err(|e| format!("FA: {e}"))?;
        let segment = &result.segments[0];
        let mut moras = Vec::with_capacity(segment.units.len());
        for (mora, unit) in line.moras.iter().zip(segment.units.iter()) {
            moras.push(AlignedMora {
                mora_index: unit.unit_id,
                mora: mora.kana.clone(),
                start: unit.start,
                end: unit.end,
                // raw_score 是模型 span score (未 calibration, 保持旧语义)
                confidence: unit.raw_score.unwrap_or(0.0).clamp(0.0, 1.0),
            });
        }
        let mut previous_end = window_start;
        for aligned in &moras {
            if aligned.end.partial_cmp(&aligned.start) != Some(Ordering::Greater)
                || aligned.start + 0.001 < previous_end
                || !aligned.start.is_finite()
                || !aligned.end.is_finite()
            {
                return Err(format!(
                    "MMS FA 返回的第 {} 个 mora 时间无效",
                    aligned.mora_index
                ));
            }
            previous_end = aligned.end;
        }
        let confidence =
            moras.iter().map(|m| m.confidence).sum::<f32>() / moras.len().max(1) as f32;
        Ok(LineAlignmentResult {
            line_index: 0,
            source: AlignmentSource::ForcedAlign,
            confidence: confidence.clamp(0.0, 1.0),
            moras,
        })
    }
}
