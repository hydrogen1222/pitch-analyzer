// Round4 Commit 7: Mandarin Pipeline v1 tests (任务书 §9 / Commit 7)
//
// 验收:
//   - 一个汉字 → 一个音节 (§9.1)
//   - 多音字: phrase/contextual dictionary > single-character fallback (§9.3)
//   - 上下文无法确定 → Ambiguous / Unknown, 不伪造
//   - tone 不是演唱音高硬约束 (§9.4): 只存数据
//   - 用户 override 优先
//   - 中文行不再走 UniDic (Commit 2 的延伸验收)

use pitch_analyzer_tauri_lib::language::LanguageRouter;
use pitch_analyzer_tauri_lib::lyrics::parse_txt_with_language;
use pitch_analyzer_tauri_lib::mandarin::reading::{MandarinReadingProvider, ReadingQuality};
use pitch_analyzer_tauri_lib::mandarin::DictMandarinReadingProvider;
use pitch_analyzer_tauri_lib::models::{ProjectLanguage, PronunciationUnitKind, SingingLanguage};

#[test]
fn hanzi_maps_to_one_syllable_each() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("我爱你").expect("analyze");
    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].surface, "我");
    assert_eq!(spans[0].pinyin, "wo3");
    assert_eq!(spans[0].tone, Some(3));
    assert_eq!(spans[1].pinyin, "ai4");
    assert_eq!(spans[2].pinyin, "ni3");
    assert_eq!(spans[0].phonemes, vec!["w", "o"]);
}

/// §9.3: 重新 → 重 chong2 (词组词典消歧, 不是 zhong4)
#[test]
fn phrase_dictionary_resolves_polyphone() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("重新开始").expect("analyze");
    assert_eq!(spans[0].pinyin, "chong2");
    assert_eq!(spans[0].quality, ReadingQuality::Contextual);
}

/// §9.3: 多音字无词组上下文 → 最常见音 + Ambiguous
#[test]
fn unresolved_polyphone_is_ambiguous() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("长").expect("analyze");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].quality, ReadingQuality::Ambiguous);
    // 非多音字 → Dictionary
    let spans = provider.analyze("我").expect("analyze");
    assert_eq!(spans[0].quality, ReadingQuality::Dictionary);
}

/// 词典未覆盖的字: 音节结构存在, 读音 Unknown (不伪造)
#[test]
fn unknown_char_keeps_syllable_structure_with_unknown_reading() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("馫").expect("analyze");
    assert_eq!(spans.len(), 1, "1 char must still be 1 syllable");
    assert_eq!(spans[0].pinyin, "");
    assert!(spans[0].phonemes.is_empty());
    assert_eq!(spans[0].quality, ReadingQuality::Unknown);
}

/// §9.4: tone 只是数据, 不产生任何旋律约束结构
#[test]
fn tone_stored_as_data_only() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("山水").expect("analyze");
    assert_eq!(spans[0].tone, Some(1)); // shan1
    assert_eq!(spans[1].tone, Some(3)); // shui3
                                        // tone 不影响 phoneme 序列 (韵母是 nucleus)
    assert_eq!(spans[0].phonemes, vec!["sh", "an"]);
}

/// 端到端: Mandarin 工程的中文行获得拼音层 + 音节 mora + 1:1 display group
#[test]
fn mandarin_project_builds_pinyin_layers() {
    let mut lines = parse_txt_with_language("重新开始\n", ProjectLanguage::Mandarin);
    let line = &mut lines[0];
    assert_eq!(line.moras.len(), 4, "4 chars -> 4 syllable units");
    assert_eq!(line.moras[0].kana, "chong2");
    assert!(!line.moras[0].phonemes.is_empty());
    assert_eq!(line.reading_spans.len(), 4);
    assert_eq!(line.reading_display_groups.len(), 4);
    // confidence 语义诚实: Contextual=1.0
    assert_eq!(line.moras[0].confidence, 1.0);
    // 拼音不包含日语假名 (P0-1 不回归)
    assert!(line
        .moras
        .iter()
        .all(|m| !pitch_analyzer_tauri_lib::japanese::mora::is_kana_char(
            m.kana.chars().next().unwrap()
        )));
}

/// v4 投影: Mandarin 音节 → MandarinSyllable kind + Nucleus policy
#[test]
fn projection_types_mandarin_syllables() {
    let lines = parse_txt_with_language("我爱你\n", ProjectLanguage::Mandarin);
    let layers = pitch_analyzer_tauri_lib::alignment::projection::project_line_v4(
        &lines[0],
        SingingLanguage::Mandarin,
    );
    assert_eq!(layers.pronunciation_units.len(), 3);
    assert!(layers
        .pronunciation_units
        .iter()
        .all(|u| u.kind == PronunciationUnitKind::MandarinSyllable
            && u.pitch_bearing == pitch_analyzer_tauri_lib::models::PitchBearingPolicy::Nucleus));
    // 音素: 韵母 pitch-bearing
    assert!(layers.phonemes.iter().any(|p| p.pitch_bearing));
}

/// 用户 override: 最高优先级, 替换多音字读音
#[test]
fn user_override_replaces_pinyin() {
    use pitch_analyzer_tauri_lib::mandarin::overrides::{
        apply_mandarin_overrides, MandarinOverride,
    };
    let provider = DictMandarinReadingProvider;
    let mut spans = provider.analyze("长").expect("analyze");
    apply_mandarin_overrides(
        &mut spans,
        &[MandarinOverride {
            char_start: 0,
            char_end: 1,
            pinyin: "zhang3".to_string(),
        }],
    );
    assert_eq!(spans[0].pinyin, "zhang3");
    assert_eq!(spans[0].quality, ReadingQuality::Contextual);
    assert_eq!(spans[0].phonemes, vec!["zh", "ang"]);
}

/// 非汉字片段不产生音节, 混排行仍正确
#[test]
fn latin_and_punctuation_segments_have_no_syllables() {
    let provider = DictMandarinReadingProvider;
    let spans = provider.analyze("爱123you!").expect("analyze");
    assert_eq!(spans.len(), 1, "only the hanzi produces a syllable");
    assert_eq!(spans[0].surface, "爱");
}

/// 语言路由: 显式 Mandarin 的含假名行也不进日语链路 (Commit 2 不回归)
#[test]
fn mandarin_override_beats_kana_script() {
    let decision = LanguageRouter.decide_line("啦啦", ProjectLanguage::Mandarin, None);
    assert_eq!(decision.language, SingingLanguage::Mandarin);
}
