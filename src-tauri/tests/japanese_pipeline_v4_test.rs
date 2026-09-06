// Round4 Commit 6: Japanese Pipeline v4 tests
//
// 验收 (任务书 §8 / Commit 6):
//   - mora parser 外来音扩展 + Unicode 规范化 (§8.4)
//   - JapanesePhonemizer trait (§8.5)
//   - pitch-bearing nucleus 策略 (P1-3) 与 nucleus 范围
//   - Reading 优先级: explicit ruby > UniDic > kana > unknown (§8.1)
//   - 半角片假名歌词全链路可用

use pitch_analyzer_tauri_lib::alignment::projection::{nucleus_hint_for, project_line_v4};
use pitch_analyzer_tauri_lib::japanese::phoneme::{JapanesePhonemizer, RuleBasedPhonemizer};
use pitch_analyzer_tauri_lib::language::LanguageRouter;
use pitch_analyzer_tauri_lib::lyrics::parse_lrc_with_language;
use pitch_analyzer_tauri_lib::models::{PitchBearingPolicy, ProjectLanguage, SingingLanguage};

fn project(text: &str) -> pitch_analyzer_tauri_lib::alignment::projection::LineV4Layers {
    let lines = parse_lrc_with_language(
        &format!("[00:01.00]{}", text),
        Some(6.0),
        ProjectLanguage::Japanese,
    );
    project_line_v4(&lines[0], SingingLanguage::Japanese)
}

/// §8.5: phonemizer 抽象可用且覆盖外来音
#[test]
fn rule_based_phonemizer_covers_foreign_moras() {
    let phonemizer = RuleBasedPhonemizer;
    for word in [
        "ヴァ", "ウェ", "ウォ", "ツァ", "ツィ", "ツェ", "ツォ", "クァ", "グァ", "シェ", "ジェ",
        "チェ", "ディ", "デュ", "トゥ", "ドゥ",
    ] {
        let specs = phonemizer
            .phonemize(word)
            .unwrap_or_else(|e| panic!("{word}: {e}"));
        assert!(
            !specs.is_empty(),
            "{} must produce phonemes (no silent fabrication gap)",
            word
        );
        assert!(
            specs.iter().any(|s| s.pitch_bearing),
            "{} must contain a pitch-bearing phoneme (vowel)",
            word
        );
    }
}

#[test]
fn phonemizer_symbols_and_classes() {
    let specs = RuleBasedPhonemizer.phonemize("きゃっと").unwrap();
    let symbols: Vec<_> = specs.iter().map(|s| s.symbol.as_str()).collect();
    assert_eq!(symbols, vec!["k", "y", "a", "cl", "t", "o"]);
    let bearing: Vec<bool> = specs.iter().map(|s| s.pitch_bearing).collect();
    assert_eq!(bearing, vec![false, false, true, false, false, true]);
}

/// P1-3: nucleus 是 pitch-bearing 音素范围; 促音无 nucleus
#[test]
fn nucleus_hint_follows_pitch_bearing_phonemes() {
    let layers = project("きゃっと");
    // unit 0 = きゃ → nucleus 只含 a
    let hint0 = nucleus_hint_for(&layers.pronunciation_units[0], &layers.phonemes);
    assert_eq!(hint0, Some((2, 3)), "きゃ nucleus must be the 'a' phoneme");
    // unit 1 = っ → None (NonPitchBearing)
    let hint1 = nucleus_hint_for(&layers.pronunciation_units[1], &layers.phonemes);
    assert_eq!(hint1, None);
    // unit 2 = と → t o → 全局音素 id: t=4, o=5 → nucleus (5, 6)
    let hint2 = nucleus_hint_for(&layers.pronunciation_units[2], &layers.phonemes);
    assert_eq!(hint2, Some((5, 6)));
}

/// §8.1: explicit ruby 优先于 UniDic (運命 → さだめ 而不是 うんめい)
#[test]
fn explicit_ruby_overrides_unidic() {
    let lines = parse_lrc_with_language(
        "[00:01.00]運命(さだめ)",
        Some(6.0),
        ProjectLanguage::Japanese,
    );
    let readings: Vec<_> = lines[0]
        .reading_display_groups
        .iter()
        .map(|g| g.reading.as_str())
        .collect();
    assert_eq!(readings, vec!["さ", "だ", "め"]);
    assert!(
        lines[0]
            .moras
            .iter()
            .all(|m| m.kana != "う" && m.kana != "んめ"),
        "UniDic reading must not leak through explicit ruby"
    );
}

/// §8.4: 半角片假名歌词全链路 (分词 → 语言检测 → mora)
#[test]
fn half_width_kana_lyrics_end_to_end() {
    let lines = parse_lrc_with_language("[00:01.00]ｽｰﾊﾟｰ", Some(6.0), ProjectLanguage::Auto);
    assert!(
        !lines[0].tokens.is_empty(),
        "half-width kana must tokenize as kana (not punctuation)"
    );
    // 语言检测: 半角假名 = 假名 (Japanese)
    let decision = LanguageRouter.decide_line("ｽｰﾊﾟｰ", ProjectLanguage::Auto, None);
    assert_eq!(decision.language, SingingLanguage::Japanese);
    let moras: Vec<_> = lines[0].moras.iter().map(|m| m.kana.as_str()).collect();
    assert_eq!(moras, vec!["ス", "ー", "パ", "ー"]);
    assert!(
        lines[0].moras.iter().all(|m| !m.phonemes.is_empty()),
        "half-width kana moras must carry phonemes"
    );
}

/// LongVowel / Conditional 策略贯通到 v4 单位
#[test]
fn long_vowel_is_conditional_pitch_bearing() {
    let layers = project("スーパー");
    for unit in &layers.pronunciation_units {
        if unit.text == "ー" {
            assert_eq!(
                unit.kind,
                pitch_analyzer_tauri_lib::models::PronunciationUnitKind::JapaneseLongVowel
            );
            assert_eq!(unit.pitch_bearing, PitchBearingPolicy::Conditional);
            // 长音的音素是母音延续 ":"
            let pid = unit.phoneme_ids[0];
            assert_eq!(layers.phonemes[pid].symbol, ":");
            assert!(layers.phonemes[pid].pitch_bearing);
        }
    }
}
