// Forced Alignment 模块 (任务书 Phase 5)
//
// 负责在 LRC 行级约束窗口内, 把已知歌词 (reading / mora / phoneme)
// 与音频特征进行强制时间对齐。
//
// 优先级:
//   EnhancedLrc (逐字已有时间) > ForcedAlign (声学模型) > MoraDp (声学启发式) > WeightedFallback

use crate::models::{AlignmentSource, LyricLine, PitchTrack};
use serde::{Deserialize, Serialize};

/// 对齐单元 (Mora 级)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedMora {
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
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String>;
}
