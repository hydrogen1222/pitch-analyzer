// Forced Alignment 模块 (任务书 Phase 5)
//
// 负责在 LRC 行级约束窗口内, 把已知歌词 (reading / mora / phoneme)
// 与音频特征进行强制时间对齐。
//
// 优先级:
//   EnhancedLrc (逐字已有时间) > ForcedAlign (声学模型) > MoraDp (声学启发式) > WeightedFallback

use crate::models::{AlignmentSource, LyricLine, PitchTrack};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// 对齐单元 (Mora 级)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedMora {
    #[serde(default)]
    pub mora_index: usize,
    pub mora: String,
    pub start: f32,
    pub end: f32,
    pub confidence: f32,
}

/// 行级对齐结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineAlignmentResult {
    pub line_index: usize,
    pub source: AlignmentSource,
    pub confidence: f32,
    pub moras: Vec<AlignedMora>,
}

/// Forced Aligner 统一抽象 trait
pub trait ForcedAlignBackend: Send + Sync {
    fn name(&self) -> &'static str;

    /// 对单行在音频局部窗口内执行强制对齐
    fn align_line(
        &self,
        line: &LyricLine,
        track: &PitchTrack,
        audio_path: &Path,
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String>;
}

/// 基于 torchaudio MMS-FA 的真实声学强制对齐后端。
///
/// 权重和推理运行在 Python helper 中；Rust 只负责传递音频/已知 mora，
/// 并校验返回的时间单调且位于 LRC 行窗口内。未配置 Python 或 helper 时，
/// 上层保持 MoraDP 回退，不会伪造 `ForcedAlign` 来源。
#[derive(Debug, Clone)]
pub struct MmsForcedAlignBackend {
    python: PathBuf,
    helper: PathBuf,
}

#[derive(Debug, Serialize)]
struct MmsRequest<'a> {
    audio_path: &'a str,
    window_start: f32,
    window_end: f32,
    moras: Vec<MmsMora<'a>>,
}

#[derive(Debug, Serialize)]
struct MmsMora<'a> {
    kana: &'a str,
    phonemes: &'a [String],
}

#[derive(Debug, Deserialize)]
struct MmsResponse {
    #[serde(default)]
    source: String,
    #[serde(default)]
    model: String,
    moras: Vec<AlignedMora>,
}

impl MmsForcedAlignBackend {
    /// 从环境变量构造 backend:
    /// `PITCH_ANALYZER_FA_PYTHON` 可指定 Python，
    /// `PITCH_ANALYZER_FA_HELPER` 可指定 helper 脚本。
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
            .unwrap_or_else(default_helper);
        if !helper.is_file() {
            return Err(format!("MMS FA helper 不存在: {}", helper.display()));
        }
        Ok(Self { python, helper })
    }

    pub fn helper_path(&self) -> &Path {
        &self.helper
    }

    fn run_helper(
        &self,
        line: &LyricLine,
        audio_path: &Path,
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String> {
        if line.moras.is_empty() {
            return Err("该歌词行没有可用于 FA 的 mora".to_string());
        }
        if window_end.partial_cmp(&window_start) != Some(Ordering::Greater) {
            return Err("FA 窗口无效".to_string());
        }
        let audio_string = audio_path.to_string_lossy().to_string();
        let request = MmsRequest {
            audio_path: &audio_string,
            window_start,
            window_end,
            moras: line
                .moras
                .iter()
                .map(|m| MmsMora {
                    kana: &m.kana,
                    phonemes: &m.phonemes,
                })
                .collect(),
        };
        let payload =
            serde_json::to_vec(&request).map_err(|e| format!("FA 请求序列化失败: {e}"))?;

        let mut child = Command::new(&self.python)
            .arg(&self.helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("启动 MMS FA Python 失败: {e}"))?;
        child
            .stdin
            .take()
            .ok_or_else(|| "无法打开 FA helper stdin".to_string())?
            .write_all(&payload)
            .map_err(|e| format!("写入 FA helper 请求失败: {e}"))?;
        let output = child
            .wait_with_output()
            .map_err(|e| format!("等待 MMS FA 完成失败: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "MMS FA 推理失败 ({}): {}",
                output.status,
                stderr.trim()
            ));
        }
        let response: MmsResponse = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("解析 MMS FA 输出失败: {e}"))?;
        if response.source != "ForcedAlign" || response.model.is_empty() {
            return Err("MMS FA helper 未返回真实 ForcedAlign 模型标识".to_string());
        }
        if response.moras.len() != line.moras.len() {
            return Err(format!(
                "MMS FA 返回 {} 个 mora，期望 {} 个",
                response.moras.len(),
                line.moras.len()
            ));
        }

        let mut previous_end = window_start;
        for (i, aligned) in response.moras.iter().enumerate() {
            if aligned.mora_index != i
                || aligned.mora != line.moras[i].kana
                || aligned.end.partial_cmp(&aligned.start) != Some(Ordering::Greater)
                || aligned.start < window_start - 0.02
                || aligned.end > window_end + 0.02
                || aligned.start + 0.001 < previous_end
                || !aligned.start.is_finite()
                || !aligned.end.is_finite()
            {
                return Err(format!("MMS FA 返回的第 {} 个 mora 时间无效", i));
            }
            previous_end = aligned.end;
        }
        let confidence = response.moras.iter().map(|m| m.confidence).sum::<f32>()
            / response.moras.len().max(1) as f32;
        Ok(LineAlignmentResult {
            line_index: 0,
            source: AlignmentSource::ForcedAlign,
            confidence: confidence.clamp(0.0, 1.0),
            moras: response.moras,
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

fn default_helper() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("forced_align_mms.py")
}

impl ForcedAlignBackend for MmsForcedAlignBackend {
    fn name(&self) -> &'static str {
        "torchaudio-mms-fa"
    }

    fn align_line(
        &self,
        line: &LyricLine,
        _track: &PitchTrack,
        audio_path: &Path,
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String> {
        let mut result = self.run_helper(line, audio_path, window_start, window_end)?;
        result.line_index = 0;
        Ok(result)
    }
}
