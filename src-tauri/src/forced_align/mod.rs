// Forced Alignment 模块 (Round4 任务书 §10)
//
// Backend 抽象 v2:
//   - SongForcedAlignBackend: 长生命周期整曲对齐 (§10.1), 纯声学请求
//   - ForcedAlignBackend: 旧行级 trait (兼容现有 pipeline 调用点)
//
// 优先级: EnhancedLrc (逐字已有时间) > ForcedAlign (声学模型) > MoraDp (声学启发式) > WeightedFallback (VisualOnly)
//
// 后端实现:
//   - legacy_mms: torchaudio MMS-FA persistent worker (模型只加载一次,
//     音频只 decode/resample 一次, 一次批处理多行 — P0-4)
//   - singing_fa: singing-domain FA 实验骨架 (无模型时不伪造结果)

pub mod legacy_mms;
pub mod manager;
pub mod singing_fa;
pub mod types;

use crate::models::{AlignmentSource, LyricLine, PitchTrack};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::Path;

pub use legacy_mms::MmsForcedAlignBackend;
pub use manager::{AlignmentBackendManager, SongForcedAlignBackend};
pub use singing_fa::SingingFaBackend;
pub use types::{
    AlignmentError, AlignmentGranularity, AlignmentRequest, AlignmentResult, BackendCapabilities,
    ExpectedUnit,
};

/// 对齐单元 (Mora 级, 旧行级协议)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedMora {
    #[serde(default)]
    pub mora_index: usize,
    pub mora: String,
    pub start: f32,
    pub end: f32,
    pub confidence: f32,
}

/// 行级对齐结果 (旧行级协议)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineAlignmentResult {
    pub line_index: usize,
    pub source: AlignmentSource,
    pub confidence: f32,
    pub moras: Vec<AlignedMora>,
}

/// 旧行级 FA trait: 现有 alignment pipeline 的调用点。
/// 生产路径由 SongForcedAlignBackend (长生命周期) 承担。
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

/// 校验一段对齐结果: 时间单调、位于窗口内 (容差吸收帧量化, 任务书 §12.2)
pub fn validate_aligned_spans(
    spans: &[(f32, f32)],
    window_start: f32,
    window_end: f32,
    tolerance: f32,
) -> Result<(), AlignmentError> {
    let mut previous_end = window_start;
    for (start, end) in spans {
        if end.partial_cmp(start) != Some(Ordering::Greater) {
            return Err(AlignmentError::InvalidModelOutput(
                "non-positive span duration".to_string(),
            ));
        }
        if *start < window_start - tolerance || *end > window_end + tolerance {
            return Err(AlignmentError::InvalidModelOutput(
                "span outside window".to_string(),
            ));
        }
        if *start + 0.001 < previous_end {
            return Err(AlignmentError::InvalidModelOutput(
                "non-monotonic spans".to_string(),
            ));
        }
        previous_end = *end;
    }
    Ok(())
}
