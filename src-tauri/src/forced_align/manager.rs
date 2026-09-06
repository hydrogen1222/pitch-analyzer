// Alignment Backend Manager (Round4 任务书 P0-3 / §10)
//
// 后端优先级:
//   Enhanced LRC (权威显示时序, 不是 FA)
//     ↓
//   Production Singing FA backend (若已安装, §10.3)
//     ↓
//   Legacy MMS backend (Experimental, persistent worker)
//     ↓
//   MoraDP emergency fallback (Heuristic)
//     ↓
//   Weighted fallback (VisualOnly)
//
// UI 必须显示当前实际 backend (诊断日志 + debug 导出)。

use super::types::{AlignmentError, AlignmentRequest, AlignmentResult, BackendCapabilities};

/// 整曲对齐 backend 抽象 (§10.1)
pub trait SongForcedAlignBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> BackendCapabilities;

    fn align(&self, request: &AlignmentRequest) -> Result<AlignmentResult, AlignmentError>;
}

/// 后端选择结果 (诊断可见)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendChoice {
    SingingFa,
    LegacyMms,
    None,
}

pub struct AlignmentBackendManager {
    choice: BackendChoice,
}

impl AlignmentBackendManager {
    /// 按优先级解析可用后端。singing_fa 配置了模型即优先 (即使 runtime
    /// 未接入 —— align() 会返回 DependencyMissing 并显式回退)。
    pub fn resolve(singing_fa_configured: bool, legacy_mms_available: bool) -> Self {
        let choice = if singing_fa_configured {
            BackendChoice::SingingFa
        } else if legacy_mms_available {
            BackendChoice::LegacyMms
        } else {
            BackendChoice::None
        };
        eprintln!(
            "[alignment-manager] active backend: {:?} (priority: singing_fa > legacy_mms > mora_dp > weighted_fallback)",
            choice
        );
        Self { choice }
    }

    pub fn choice(&self) -> &BackendChoice {
        &self.choice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_priority_singing_fa_first() {
        let manager = AlignmentBackendManager::resolve(true, true);
        assert_eq!(manager.choice(), &BackendChoice::SingingFa);
        let manager = AlignmentBackendManager::resolve(false, true);
        assert_eq!(manager.choice(), &BackendChoice::LegacyMms);
        let manager = AlignmentBackendManager::resolve(false, false);
        assert_eq!(manager.choice(), &BackendChoice::None);
    }
}
