// Forced Alignment v2 types (Round4 任务书 §10 / §27 / §28)
//
// Backend 只接收纯声学任务 (不依赖 LyricLine/Token 数据模型), 可独立测试:
//   AlignmentRequest { language, sample_rate, audio_ref, segments }
//   segment → unit(音素序列) → AlignedPhoneme spans
//
// 结果必须带 segment_id / unit_id (§27.3), 不允许靠 vector 位置猜对应关系。

use crate::models::{SingingLanguage, SingingLanguage as Lang};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 一个期望音素 (id 在 segment 内唯一)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedPhoneme {
    pub id: usize,
    pub symbol: String,
}

/// 一个对齐段 (一行歌词 / 一个窗口)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignmentSegmentRequest {
    /// 调用方指定的段 id, 结果必须原样带回 (§27.3)
    pub id: usize,
    pub window_start: f32,
    pub window_end: f32,
    /// 段内的对齐单元: 每个单元是一串期望音素 (日语 mora / 中文音节)
    pub units: Vec<ExpectedUnit>,
}

/// 对齐单元 = 已知音素序列 (mora / syllable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedUnit {
    pub id: usize,
    pub symbols: Vec<String>,
}

/// 整曲对齐请求。audio 用 Arc 共享 (§27.1: 只 decode/resample 一次)。
#[derive(Clone)]
pub struct AlignmentRequest {
    pub language: Lang,
    pub sample_rate: u32,
    /// 与 worker 协议配合的音频标识 (文件路径); Rust 侧按 §27.1 缓存
    pub audio: Arc<AudioRef>,
    pub segments: Vec<AlignmentSegmentRequest>,
}

/// 音频引用 (文件路径)。长生命周期 backend 在 init 时加载一次。
#[derive(Debug, Clone)]
pub struct AudioRef {
    pub path: String,
}

/// 段内一个已对齐单元
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedUnit {
    pub unit_id: usize,
    pub start: f32,
    pub end: f32,
    /// 模型原始得分 (CTC span score 均值等); 未 calibration 不得冒充概率
    pub raw_score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentAlignmentResult {
    pub segment_id: usize,
    pub units: Vec<AlignedUnit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignmentResult {
    pub backend: String,
    pub segments: Vec<SegmentAlignmentResult>,
}

/// Backend 能力声明 (§10.1)
#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    pub languages: Vec<SingingLanguage>,
    pub unit_level: AlignmentGranularity,
    pub singing_domain: bool,
    /// 模型/音频只加载一次 (P0-4)
    pub persistent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignmentGranularity {
    Phoneme,
    Mora,
    Syllable,
    Character,
}

/// 结构化错误 (§28): UI 显示"为什么回退", 而不是笼统的"对齐失败"
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignmentError {
    BackendUnavailable,
    UnsupportedLanguage,
    MissingPronunciation,
    ModelLoadFailed(String),
    InvalidModelOutput(String),
    AudioWindowInvalid,
    DependencyMissing(String),
}

impl std::fmt::Display for AlignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlignmentError::BackendUnavailable => write!(f, "FA backend unavailable"),
            AlignmentError::UnsupportedLanguage => write!(f, "language unsupported by backend"),
            AlignmentError::MissingPronunciation => write!(f, "segment has no pronunciation"),
            AlignmentError::ModelLoadFailed(m) => write!(f, "model load failed: {m}"),
            AlignmentError::InvalidModelOutput(m) => write!(f, "invalid model output: {m}"),
            AlignmentError::AudioWindowInvalid => write!(f, "audio window invalid"),
            AlignmentError::DependencyMissing(m) => write!(f, "dependency missing: {m}"),
        }
    }
}

impl std::error::Error for AlignmentError {}
