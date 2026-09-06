// Mandarin user overrides (任务书 §9.3: 允许用户 override 多音字/特殊读法)

use serde::{Deserialize, Serialize};

/// 用户对一段原文 (通常一个汉字) 的读音覆盖。
/// pinyin 用 tone-number 记法 (如 "chong2"); 空串 = 用户显式声明无读音。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandarinOverride {
    pub char_start: usize,
    pub char_end: usize,
    pub pinyin: String,
}

/// 对 provider 分析结果应用用户覆盖:
/// 命中 char 区间的 span 替换拼音并把 quality 提升为 Contextual
/// (用户 override 是最高优先级读音真值)。
pub fn apply_mandarin_overrides(
    spans: &mut Vec<crate::mandarin::reading::MandarinSyllableSpan>,
    overrides: &[MandarinOverride],
) {
    for ov in overrides {
        for span in spans.iter_mut() {
            if span.char_start >= ov.char_start && span.char_end <= ov.char_end {
                span.pinyin = ov.pinyin.clone();
                span.tone = ov
                    .pinyin
                    .chars()
                    .last()
                    .and_then(|c| c.to_digit(10).map(|d| d as u8));
                span.phonemes = crate::mandarin::phoneme::pinyin_to_phonemes(&ov.pinyin);
                span.quality = crate::mandarin::reading::ReadingQuality::Contextual;
            }
        }
    }
}
