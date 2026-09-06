// v4 projection (Round4 Commit 5)
//
// 把旧三层文本结构 (ReadingSpan / MoraUnit / phoneme 表) 投影为
// v4 canonical 数据模型 (SourceSpan / PronunciationUnit / PhonemeUnit)。
//
// 规则 (任务书 §7/§8):
//   - SourceSpan 只回答"这是原文的哪一段", 不存 pitch;
//   - 读音未知的 span 不伪造 PronunciationUnit;
//   - 促音/长音/拨音的 kind 与 pitch-bearing 策略显式化 (P1-4);
//   - 不做"一汉字一 mora"的逐字伪造映射 (§8.2)。
//
// AcousticSpan / PitchAttachment 由对齐后端与 matcher 回填 (Commit 8/9)。

use crate::japanese::pitch_policy::{
    classify_phoneme_class, classify_unit_kind, phoneme_pitch_bearing, pitch_bearing_policy,
};
use crate::japanese::unidic::katakana_to_hiragana;
use crate::models::{PhonemeUnit, PronunciationUnit, SingingLanguage, SourceKind, SourceSpan};

/// 一行歌词的 v4 文本/发音层投影结果。
/// phonemes 与 pronunciation_units 通过 id 关联 (全行唯一)。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LineV4Layers {
    pub source_spans: Vec<SourceSpan>,
    pub pronunciation_units: Vec<PronunciationUnit>,
    pub phonemes: Vec<PhonemeUnit>,
}

/// 投影一行歌词。`language` 是该行已解析的语言 (LanguageRouter 决策结果)。
pub fn project_line_v4(line: &crate::models::LyricLine, language: SingingLanguage) -> LineV4Layers {
    let mut out = LineV4Layers::default();

    // 1. SourceSpan: 原文层, 与 ReadingSpan 一一对应;
    //    无读音层的行 (中文歧义/Unknown) 从 display token 兜底构建,
    //    保证"原文是哪一段"这一层永远存在
    if !line.reading_spans.is_empty() {
        for span in &line.reading_spans {
            let id = out.source_spans.len();
            out.source_spans.push(SourceSpan {
                id,
                surface: span.surface.clone(),
                char_start: span.char_start,
                char_end: span.char_end,
                language,
                pronunciation_ids: Vec::new(),
                source_kind: classify_source_kind(&span.surface),
            });
        }
    } else {
        for token in &line.tokens {
            let id = out.source_spans.len();
            out.source_spans.push(SourceSpan {
                id,
                surface: token.text.clone(),
                char_start: token.char_start,
                char_end: token.char_end,
                language,
                pronunciation_ids: Vec::new(),
                source_kind: classify_source_kind(&token.text),
            });
        }
    }

    // 2. PronunciationUnit + PhonemeUnit: 从行 mora 序列投影。
    //    mora 是读音展开的真实结果 (ruby > UniDic > kana), 不按汉字拆分。
    for mora in &line.moras {
        let unit_id = out.pronunciation_units.len();
        let kind = classify_unit_kind(&mora.kana);
        let normalized = katakana_to_hiragana(&mora.kana);
        let pitch_bearing = pitch_bearing_policy(kind);

        let mut phoneme_ids = Vec::with_capacity(mora.phonemes.len());
        for symbol in &mora.phonemes {
            let class = classify_phoneme_class(symbol);
            let pid = out.phonemes.len();
            out.phonemes.push(PhonemeUnit {
                id: pid,
                symbol: symbol.clone(),
                class,
                pronunciation_unit_id: unit_id,
                pitch_bearing: phoneme_pitch_bearing(class),
                alignment: None,
            });
            phoneme_ids.push(pid);
        }

        out.pronunciation_units.push(PronunciationUnit {
            id: unit_id,
            language,
            text: mora.kana.clone(),
            normalized,
            kind,
            source_span_ids: if mora.reading_span_id != usize::MAX {
                vec![mora.reading_span_id]
            } else {
                Vec::new()
            },
            phoneme_ids,
            pitch_bearing,
        });
    }

    // 3. SourceSpan.pronunciation_ids 反向关联
    for unit in &out.pronunciation_units {
        for span_id in &unit.source_span_ids {
            if let Some(span) = out.source_spans.get_mut(*span_id) {
                span.pronunciation_ids.push(unit.id);
            }
        }
    }

    out
}

/// 原文片段书写类型: 全部同类 → 该类; 否则 Mixed
pub fn classify_source_kind(surface: &str) -> SourceKind {
    let mut kinds: Vec<SourceKind> = Vec::new();
    for c in surface.chars() {
        if c.is_whitespace() {
            continue;
        }
        let kind = if crate::japanese::mora::is_kana_char(c) || c == '\u{30FC}' {
            SourceKind::Kana
        } else if is_han_char(c) {
            SourceKind::Han
        } else if c.is_ascii_alphabetic() {
            SourceKind::Latin
        } else if c.is_ascii_digit() {
            SourceKind::Number
        } else {
            SourceKind::Punctuation
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    match kinds.len() {
        0 => SourceKind::Punctuation,
        1 => kinds[0],
        _ => SourceKind::Mixed,
    }
}

fn is_han_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

/// 单位内 pitch-bearing 音素范围 [start, end) 的投影层包装 (P1-3)。
/// None = 物理无音高 (促音) 或无 voiced 音素。Commit 8/9 的 FA 与
/// matcher 用它把 pitch 判定收窄到母音核。
pub fn nucleus_hint_for(
    unit: &PronunciationUnit,
    phonemes: &[PhonemeUnit],
) -> Option<(usize, usize)> {
    crate::japanese::pitch_policy::nucleus_phoneme_range(unit, phonemes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::parse_lrc_with_language;
    use crate::models::{
        PhonemeClass, PitchBearingPolicy, ProjectLanguage, PronunciationUnitKind,
    };

    fn project(text: &str) -> LineV4Layers {
        let lines = parse_lrc_with_language(
            &format!("[00:01.00]{}", text),
            Some(6.0),
            ProjectLanguage::Japanese,
        );
        project_line_v4(&lines[0], SingingLanguage::Japanese)
    }

    #[test]
    fn today_becomes_one_source_span_two_units() {
        // 今日 → きょ・ー/…: 词典读音按词展开, 不做 今=きょ / 日=… 的逐字伪造
        let layers = project("今日も");
        let han: Vec<_> = layers
            .source_spans
            .iter()
            .filter(|s| s.source_kind == SourceKind::Han)
            .collect();
        assert_eq!(han.len(), 1, "今日 must be a single source span");
        assert_eq!(han[0].surface, "今日");
        let units: Vec<_> = layers
            .pronunciation_units
            .iter()
            .map(|u| u.text.as_str())
            .collect();
        assert_eq!(units, vec!["きょ", "ー", "も"]);
    }

    #[test]
    fn sokuon_long_vowel_nasal_semantics() {
        let layers = project("がっこう");
        let kinds: Vec<_> = layers.pronunciation_units.iter().map(|u| u.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PronunciationUnitKind::JapaneseMora,
                PronunciationUnitKind::JapaneseSokuon,
                PronunciationUnitKind::JapaneseMora,
                PronunciationUnitKind::JapaneseMora,
            ]
        );
        let policies: Vec<_> = layers
            .pronunciation_units
            .iter()
            .map(|u| u.pitch_bearing)
            .collect();
        assert_eq!(
            policies,
            vec![
                PitchBearingPolicy::Nucleus,
                PitchBearingPolicy::None,
                PitchBearingPolicy::Nucleus,
                PitchBearingPolicy::Nucleus,
            ]
        );

        let layers = project("スーパー");
        let kinds: Vec<_> = layers.pronunciation_units.iter().map(|u| u.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PronunciationUnitKind::JapaneseMora,
                PronunciationUnitKind::JapaneseLongVowel,
                PronunciationUnitKind::JapaneseMora,
                PronunciationUnitKind::JapaneseLongVowel,
            ]
        );
        // 长音是母音延续 (Conditional), 规范化为平假名
        let lv = &layers.pronunciation_units[1];
        assert_eq!(lv.pitch_bearing, PitchBearingPolicy::Conditional);
        assert_eq!(lv.normalized, "ー");

        let layers = project("さんぽ");
        assert_eq!(
            layers.pronunciation_units[1].kind,
            PronunciationUnitKind::JapaneseMoraicNasal
        );
        assert_eq!(
            layers.pronunciation_units[1].pitch_bearing,
            PitchBearingPolicy::Conditional
        );
    }

    #[test]
    fn phoneme_classes_and_pitch_bearing() {
        let layers = project("きゃっと");
        // きゃ = k y a, っ = cl, と = t o
        let classes: Vec<_> = layers.phonemes.iter().map(|p| p.class).collect();
        assert_eq!(
            classes,
            vec![
                PhonemeClass::Consonant,
                PhonemeClass::Glide,
                PhonemeClass::Vowel,
                PhonemeClass::Closure,
                PhonemeClass::Consonant,
                PhonemeClass::Vowel,
            ]
        );
        let bearing: Vec<bool> = layers.phonemes.iter().map(|p| p.pitch_bearing).collect();
        assert_eq!(bearing, vec![false, false, true, false, false, true]);
        // 反向关联完整
        for unit in &layers.pronunciation_units {
            for pid in &unit.phoneme_ids {
                assert_eq!(layers.phonemes[*pid].pronunciation_unit_id, unit.id);
            }
        }
    }

    #[test]
    fn source_kind_classification() {
        assert_eq!(classify_source_kind("今日"), SourceKind::Han);
        assert_eq!(classify_source_kind("きゃ"), SourceKind::Kana);
        assert_eq!(classify_source_kind("Hello"), SourceKind::Latin);
        assert_eq!(classify_source_kind("123"), SourceKind::Number);
        assert_eq!(classify_source_kind("…!"), SourceKind::Punctuation);
        assert_eq!(classify_source_kind("La!"), SourceKind::Mixed);
    }
}
