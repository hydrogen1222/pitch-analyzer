// Singing-domain FA backend skeleton (Round4 任务书 §10.3)
//
// 优先研究方向: OpenVPI dataset-tools / HubertFA 体系 (歌声域, 而非朗读语音)。
//
// 重要约束: 在模型 runtime 真正接入之前, 本后端必须保持"已配置但不可用"
// 的诚实状态 —— 绝不返回伪造的对齐结果充数。模型接入前还必须完成
// license / redistribution / 体积 / CPU 推理速度 审计 (§30,
// 记录于 THIRD_PARTY_MODELS.md)。

use super::types::{AlignmentError, AlignmentRequest, AlignmentResult, BackendCapabilities};
use super::SongForcedAlignBackend;
use crate::models::SingingLanguage;
use std::path::PathBuf;

pub struct SingingFaBackend {
    model_dir: PathBuf,
}

impl SingingFaBackend {
    /// 环境变量 PITCH_ANALYZER_SINGING_FA_MODEL 指向用户自备的模型目录。
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var_os("PITCH_ANALYZER_SINGING_FA_MODEL").map(PathBuf::from)?;
        dir.is_dir().then(|| Self { model_dir: dir })
    }

    pub fn is_configured() -> bool {
        std::env::var_os("PITCH_ANALYZER_SINGING_FA_MODEL").is_some()
    }

    pub fn model_dir(&self) -> &std::path::Path {
        &self.model_dir
    }
}

impl SongForcedAlignBackend for SingingFaBackend {
    fn name(&self) -> &'static str {
        "singing-fa (experimental)"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            languages: vec![SingingLanguage::Japanese, SingingLanguage::Mandarin],
            unit_level: super::AlignmentGranularity::Phoneme,
            singing_domain: true,
            persistent: true,
        }
    }

    fn align(&self, _request: &AlignmentRequest) -> Result<AlignmentResult, AlignmentError> {
        // 模型 runtime 未接入 (许可审计 + ONNX 推理集成属于后续工作)。
        // 诚实返回 DependencyMissing, 上层回退 MoraDP 并如实标注。
        Err(AlignmentError::DependencyMissing(
            "singing-domain FA runtime is not integrated yet; \
             model licensing audit pending (see THIRD_PARTY_MODELS.md)"
                .to_string(),
        ))
    }
}
