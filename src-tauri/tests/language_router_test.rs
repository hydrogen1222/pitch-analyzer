// Round4 Commit 2: Language Router integration tests
//
// 核心验收 (任务书 Commit 2): 中文行绝不调用 Japanese provider。
// 日语行行为保持不变; 工程覆盖 > 脚本启发式。

use pitch_analyzer_tauri_lib::diagnostics::summarize;
use pitch_analyzer_tauri_lib::lyrics::{parse_lrc_with_language, parse_txt_with_language};
use pitch_analyzer_tauri_lib::models::{ProjectLanguage, SingingLanguage};

const JA_LRC: &str = "\
[00:01.00]今日も君を見てる
[00:04.00]ドラマの続きを
";

const JA_LRC_WITH_KANJI_ONLY_LINE: &str = "\
[00:01.00]未来へ
[00:02.00]きみと歩く
[00:04.00]夢
";

const ZH_LRC: &str = "\
[00:01.00]我爱你
[00:03.00]重新开始
";

fn is_kana(s: &str) -> bool {
    s.chars()
        .any(|c| ('\u{3041}'..='\u{309F}').contains(&c) || ('\u{30A0}'..='\u{30FF}').contains(&c))
}

#[test]
fn chinese_project_never_enters_japanese_pipeline() {
    // 显式 Mandarin 工程
    let lines = parse_lrc_with_language(ZH_LRC, Some(6.0), ProjectLanguage::Mandarin);
    assert_eq!(lines.len(), 2);
    for line in &lines {
        assert!(
            line.moras.is_empty(),
            "Mandarin lines must have no Japanese moras"
        );
        assert!(
            line.reading_spans.iter().all(|s| !is_kana(&s.reading)),
            "Mandarin lines must not receive Japanese kana readings"
        );
    }
    // Auto 工程 (无假名行 → 无法判定 → Unknown, 同样不进日语链路)
    let lines = parse_lrc_with_language(ZH_LRC, Some(6.0), ProjectLanguage::Auto);
    for line in &lines {
        assert!(line.moras.is_empty());
    }
}

#[test]
fn japanese_auto_project_unchanged_by_router() {
    let lines = parse_lrc_with_language(JA_LRC, Some(6.0), ProjectLanguage::Auto);
    assert_eq!(lines.len(), 2);
    // UniDic 词典读音仍然生效
    assert_eq!(
        lines[0]
            .moras
            .iter()
            .map(|m| m.kana.as_str())
            .collect::<Vec<_>>(),
        vec!["きょ", "ー", "も", "き", "み", "を", "み", "て", "る"]
    );
}

#[test]
fn japanese_kanji_only_line_inherits_dominant_japanese() {
    // 「夢」是纯汉字行; 工程含假名行 → 主导语言 Japanese → UniDic 读音
    let lines = parse_lrc_with_language(
        JA_LRC_WITH_KANJI_ONLY_LINE,
        Some(6.0),
        ProjectLanguage::Auto,
    );
    assert_eq!(lines.len(), 3);
    assert!(!is_kana(&lines[2].primary_text));
    assert!(
        !lines[2].moras.is_empty(),
        "kanji-only line in a kana-dominant song must inherit Japanese (UniDic reading)"
    );
    assert!(is_kana(&lines[2].moras[0].kana));
}

#[test]
fn chinese_project_does_not_run_unidic_on_kana_lines_either() {
    // 工程覆盖 Mandarin 优先于脚本证据 (任务书 §6.1)
    let lines = parse_lrc_with_language(JA_LRC, Some(6.0), ProjectLanguage::Mandarin);
    for line in &lines {
        assert!(line.moras.is_empty());
    }
}

#[test]
fn ambiguous_han_lines_not_silently_assigned() {
    // 混合场景: 中文歌夹一行含假名的歌词 → 假名行走日语, 其余纯汉字行歧义
    let mixed = "[00:01.00]我爱你\n[00:02.00]ラララ\n";
    let lines = parse_lrc_with_language(mixed, None, ProjectLanguage::Auto);
    // ラララ → Japanese
    assert!(!lines[1].moras.is_empty());
    // 我爱你 → 歧义, 不悄悄选语言
    assert!(lines[0].moras.is_empty());
}

#[test]
fn enhanced_lrc_timing_survives_for_unknown_language_lines() {
    // P0: 不允许删除 Enhanced LRC 最高优先级语义 —— 中文行也应保留逐字锚点
    let enhanced = "[00:01.00]<00:01.00>我<00:01.50>爱<00:02.00>你\n";
    let lines = parse_lrc_with_language(enhanced, Some(6.0), ProjectLanguage::Auto);
    assert_eq!(lines.len(), 1);
    for token in &lines[0].tokens {
        assert!(token.start_time.is_some() && token.end_time.is_some());
        assert_eq!(
            token.alignment_source,
            Some(pitch_analyzer_tauri_lib::models::AlignmentSource::EnhancedLrc)
        );
    }
}

#[test]
fn parse_txt_with_language_routes_same_as_lrc() {
    let txt = "我爱你\n";
    let lines = parse_txt_with_language(txt, ProjectLanguage::Mandarin);
    assert!(lines[0].moras.is_empty());
    let lines = parse_txt_with_language(txt, ProjectLanguage::Auto);
    assert!(lines[0].moras.is_empty());
}

#[test]
fn mandarin_project_diagnostic_summary_is_honest() {
    // 中文行在 Mandarin pipeline 接入前: 无 mora、无读音 —— 诊断必须如实反映
    let lines = parse_lrc_with_language(ZH_LRC, Some(6.0), ProjectLanguage::Mandarin);
    let summary = summarize(&lines);
    assert_eq!(summary.mora_count, 0);
    assert_eq!(summary.token_count, 7); // 我爱你(3) + 重新开始(4)
    assert_eq!(summary.tokens_without_pronunciation, 7);
    assert_eq!(summary.language_observation.han_only_lines, 2);
    assert_eq!(summary.language_observation.lines_with_kana, 0);
}

#[test]
fn decide_line_language_debug_helper() {
    use pitch_analyzer_tauri_lib::lyrics::decide_line_language;
    let (lang, reason) = decide_line_language("きみ", ProjectLanguage::Auto, None);
    assert_eq!(lang, SingingLanguage::Japanese);
    assert_eq!(
        reason,
        pitch_analyzer_tauri_lib::language::LanguageDecisionReason::ScriptKana
    );
}
