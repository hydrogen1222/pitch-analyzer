// Pitch-bearing policy (Round4 任务书 P1-3 / P1-4 / §11)
//
// 日语 mora 不直接作为 pitch-bearing 单位:
//   mora → phonemes → pitch-bearing nucleus → FCPE / GAME evidence
// 促音/长音/拨音的语义在此显式化, 供投影层 (Commit 5) 与
// note matcher (Commit 9) 共用, 保证单一事实源。

use crate::models::{
    PhonemeClass, PhonemeUnit, PitchBearingPolicy, PronunciationUnit, PronunciationUnitKind,
};

/// mora 假名 → 发音单位类型
pub fn classify_unit_kind(kana: &str) -> PronunciationUnitKind {
    match kana {
        "っ" | "ッ" => PronunciationUnitKind::JapaneseSokuon,
        "ー" => PronunciationUnitKind::JapaneseLongVowel,
        "ん" | "ン" => PronunciationUnitKind::JapaneseMoraicNasal,
        _ => PronunciationUnitKind::JapaneseMora,
    }
}

/// 单位类型 → pitch-bearing 策略 (任务书 P1-4):
///   っ: 物理无音高 → None (默认显示 —, 不伪造独立 pitch measurement)
///   ー: 前一母音核的延续, 可共享前一 note 或跨 note → Conditional
///   ん: 可 voiced, 取决于实际 voicing 证据 → Conditional
///   普通 mora / 中文音节: 母音核载体 → Nucleus
pub fn pitch_bearing_policy(kind: PronunciationUnitKind) -> PitchBearingPolicy {
    match kind {
        PronunciationUnitKind::JapaneseSokuon => PitchBearingPolicy::None,
        PronunciationUnitKind::JapaneseLongVowel => PitchBearingPolicy::Conditional,
        PronunciationUnitKind::JapaneseMoraicNasal => PitchBearingPolicy::Conditional,
        PronunciationUnitKind::JapaneseMora => PitchBearingPolicy::Nucleus,
        PronunciationUnitKind::MandarinSyllable => PitchBearingPolicy::Nucleus,
        PronunciationUnitKind::Silence | PronunciationUnitKind::Punctuation => {
            PitchBearingPolicy::None
        }
        PronunciationUnitKind::Unknown => PitchBearingPolicy::Conditional,
    }
}

/// 音素符号 → 分类。符号集来自 RuleBasedPhonemizer 规则表;
/// 替换 phonemizer (OpenJTalk / FA dictionary) 时同步扩展此映射。
pub fn classify_phoneme_class(symbol: &str) -> PhonemeClass {
    match symbol {
        "a" | "i" | "u" | "e" | "o" => PhonemeClass::Vowel,
        // 长音符: 前一母音的延续 (§8/§11: 共享母音核)
        ":" => PhonemeClass::Vowel,
        "N" => PhonemeClass::Nasal,
        "cl" => PhonemeClass::Closure,
        "y" | "w" => PhonemeClass::Glide,
        "sil" | "pau" => PhonemeClass::Silence,
        "" => PhonemeClass::Unknown,
        _ => {
            if symbol.chars().all(|c| c.is_ascii_alphabetic()) {
                PhonemeClass::Consonant
            } else {
                PhonemeClass::Unknown
            }
        }
    }
}

/// 音素级 pitch-bearing: 母音/鼻音可承载 pitch。
/// 滑音 (y/w) 属于起音部分 (任务书 §11.1: きゃ = k y a 的 nucleus 只是 a),
/// 辅音与闭塞 (cl) 不承载。
pub fn phoneme_pitch_bearing(class: PhonemeClass) -> bool {
    matches!(class, PhonemeClass::Vowel | PhonemeClass::Nasal)
}

/// 单位内 pitch-bearing 音素的下标范围 [start, end) (§11.1 的 nucleus 提示)。
/// 例: きょ = k y a → (2, 3) (只取母音核 a)。
/// 返回 None: 单位物理无音高 (っ) 或没有任何 voiced 音素。
pub fn nucleus_phoneme_range(
    unit: &PronunciationUnit,
    phonemes: &[PhonemeUnit],
) -> Option<(usize, usize)> {
    if unit.pitch_bearing == PitchBearingPolicy::None {
        return None;
    }
    let mut first: Option<usize> = None;
    let mut last: usize = 0;
    for pid in &unit.phoneme_ids {
        let Some(p) = phonemes.get(*pid) else {
            continue;
        };
        if p.pitch_bearing {
            if first.is_none() {
                first = Some(*pid);
            }
            last = *pid;
        }
    }
    first.map(|start| (start, last + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(kind: PronunciationUnitKind, policy: PitchBearingPolicy) -> PronunciationUnit {
        PronunciationUnit {
            id: 0,
            language: SingingLanguage::Japanese,
            text: "きょ".to_string(),
            normalized: "きょ".to_string(),
            kind,
            source_span_ids: Vec::new(),
            phoneme_ids: (0..3).collect(),
            pitch_bearing: policy,
        }
    }

    use crate::models::SingingLanguage;

    #[test]
    fn special_mora_kinds() {
        assert_eq!(
            classify_unit_kind("っ"),
            PronunciationUnitKind::JapaneseSokuon
        );
        assert_eq!(
            classify_unit_kind("ー"),
            PronunciationUnitKind::JapaneseLongVowel
        );
        assert_eq!(
            classify_unit_kind("ん"),
            PronunciationUnitKind::JapaneseMoraicNasal
        );
        assert_eq!(
            classify_unit_kind("か"),
            PronunciationUnitKind::JapaneseMora
        );
        assert_eq!(
            classify_unit_kind("ッ"),
            PronunciationUnitKind::JapaneseSokuon
        );
    }

    #[test]
    fn policies_match_task_book() {
        assert_eq!(
            pitch_bearing_policy(PronunciationUnitKind::JapaneseSokuon),
            PitchBearingPolicy::None
        );
        assert_eq!(
            pitch_bearing_policy(PronunciationUnitKind::JapaneseLongVowel),
            PitchBearingPolicy::Conditional
        );
        assert_eq!(
            pitch_bearing_policy(PronunciationUnitKind::JapaneseMoraicNasal),
            PitchBearingPolicy::Conditional
        );
        assert_eq!(
            pitch_bearing_policy(PronunciationUnitKind::JapaneseMora),
            PitchBearingPolicy::Nucleus
        );
    }

    #[test]
    fn nucleus_range_excludes_consonants() {
        let mut phonemes: Vec<PhonemeUnit> = Vec::new();
        for (i, (symbol, class)) in [
            ("k", PhonemeClass::Consonant),
            ("y", PhonemeClass::Glide),
            ("a", PhonemeClass::Vowel),
        ]
        .into_iter()
        .enumerate()
        {
            phonemes.push(PhonemeUnit {
                id: i,
                symbol: symbol.to_string(),
                class,
                pronunciation_unit_id: 0,
                pitch_bearing: phoneme_pitch_bearing(class),
                alignment: None,
            });
        }
        let u = unit(
            PronunciationUnitKind::JapaneseMora,
            PitchBearingPolicy::Nucleus,
        );
        // きゃ = k y a → nucleus 只含 a (y 是起音, 任务书 §11.1)
        assert_eq!(nucleus_phoneme_range(&u, &phonemes), Some((2, 3)));

        // っ (None policy) → 无 nucleus
        let sokuon = unit(
            PronunciationUnitKind::JapaneseSokuon,
            PitchBearingPolicy::None,
        );
        assert_eq!(nucleus_phoneme_range(&sokuon, &phonemes), None);
    }
}
