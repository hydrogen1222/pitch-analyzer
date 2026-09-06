// Mandarin pipeline (Round4 Commit 7, 任务书 §9)
//
// 中文从零补齐真实链路 (此前被无条件送进 UniDic 日语词典 — P0-1):
//
//   Chinese source → contextual G2P → Pinyin syllable → initial/final phonemes
//   → forced alignment (Commit 8) → syllable nucleus → notes/F0
//
// 原则:
//   - 一个汉字通常自然映射一个 syllable (§9.1), 但多音字必须 contextual;
//   - phrase/contextual dictionary > single-character fallback (§9.3);
//   - 上下文无法确定 → quality = Ambiguous, 允许用户 override;
//   - tone 不是演唱音高硬约束 (§9.4), 只用于 G2P/诊断;
//   - 无法确定的读音保持 Unknown, 不伪造准确率。

pub mod overrides;
pub mod phoneme;
pub mod pipeline;
pub mod reading;

pub use overrides::{apply_mandarin_overrides, MandarinOverride};
pub use phoneme::pinyin_to_phonemes;
pub use pipeline::build_mandarin_layers;
pub use reading::{
    DictMandarinReadingProvider, MandarinReadingProvider, MandarinSyllableSpan, ReadingQuality,
};
