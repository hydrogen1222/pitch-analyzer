// Round4 Commit 1: Baseline snapshot tests
//
// 固化 Round4 重构起点 (Commit 1) 的实际行为摘要:
//   - 语言/结构统计 (diagnostics::PipelineSummary)
//   - 对齐来源分布
//   - serde round-trip 兼容
//
// 这些快照锚定 Commit 2/3/4 引入的每一步行为变化:
//   - 语言路由 (Commit 2) 会改变 ZH 样例的 mora/group 数值 (预期: 中文行
//     不再产生日语 mora), 属预期更新;
//   - False Precision 清理 (Commit 4) 会改变对齐来源分布, 属预期更新。
// 除任务书指明的预期变化外, 快照不得静默漂移。
//
// P0-1 baseline 证据 (Commit 1 时点): `我爱你` 被 UniDic 读成 `わ が`,
// `再见` 被读成 `さ い` —— 中文歌词无条件进入日语链路的直接实证。

use pitch_analyzer_tauri_lib::diagnostics::{summarize, LanguageObservation, PipelineSummary};
use pitch_analyzer_tauri_lib::lyrics::{parse_lrc, parse_txt};
use pitch_analyzer_tauri_lib::models::ProjectData;

const JA_PLAIN_LRC: &str = "\
[00:01.00]今日も君を見てる
[00:04.00]ドラマの続きを
";

const JA_RUBY_LRC: &str = "\
[00:01.00]運命(さだめ)だよ
";

const JA_ENHANCED_LRC: &str = "\
[00:01.00]<00:01.00>きょう<00:02.00>も
";

const ZH_PLAIN_TXT: &str = "我爱你
";

const ZH_TIMED_TXT: &str = "\
[00:01.00]我爱你
[00:03.00]再见
";

fn ja_sources_baseline() -> pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
    // 无音频/无 backend: 所有 token 均未对齐 (Commit 1 时点)
    pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
        unaligned: 1,
        ..Default::default()
    }
}

#[test]
fn snapshot_parse_lrc_japanese_plain() {
    let lines = parse_lrc(JA_PLAIN_LRC, Some(6.0));
    let summary = summarize(&lines);
    let expected = PipelineSummary {
        line_count: 2,
        token_count: 15,
        // 今日=きょ・ー (UniDic 词读音), 君=き・み; ドラマ=ど・ら・ま
        mora_count: 17,
        display_group_count: 13,
        ruby_annotation_count: 0,
        tokens_without_pronunciation: 0,
        sources: pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
            unaligned: 15,
            ..Default::default()
        },
        language_observation: LanguageObservation {
            lines_with_kana: 2,
            ..Default::default()
        },
    };
    assert_eq!(summary, expected, "JA plain LRC baseline drifted");
    // 熟字训/词典读音: 今日 → きょ + ー (不是 今=きょ 日=ー 的逐字伪造)
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
fn snapshot_parse_lrc_explicit_ruby() {
    let lines = parse_lrc(JA_RUBY_LRC, Some(6.0));
    let summary = summarize(&lines);
    let expected = PipelineSummary {
        line_count: 1,
        token_count: 4,
        // 運命(さだめ) → さ・だ・め + だ・よ
        mora_count: 5,
        display_group_count: 5,
        ruby_annotation_count: 1,
        tokens_without_pronunciation: 0,
        sources: pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
            unaligned: 4,
            ..Default::default()
        },
        language_observation: LanguageObservation {
            lines_with_kana: 1,
            ..Default::default()
        },
    };
    assert_eq!(summary, expected, "JA ruby LRC baseline drifted");
    assert_eq!(
        lines[0]
            .moras
            .iter()
            .map(|m| m.kana.as_str())
            .collect::<Vec<_>>(),
        vec!["さ", "だ", "め", "だ", "よ"]
    );
}

#[test]
fn snapshot_parse_lrc_enhanced_anchors() {
    let lines = parse_lrc(JA_ENHANCED_LRC, Some(6.0));
    let summary = summarize(&lines);
    let expected = PipelineSummary {
        line_count: 1,
        token_count: 3,
        mora_count: 3,
        display_group_count: 3,
        ruby_annotation_count: 0,
        tokens_without_pronunciation: 0,
        sources: pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
            enhanced_lrc: 3,
            ..Default::default()
        },
        language_observation: LanguageObservation {
            lines_with_kana: 1,
            ..Default::default()
        },
    };
    assert_eq!(summary, expected, "JA enhanced LRC baseline drifted");
    // enhanced LRC 权威逐字时间保留
    for token in &lines[0].tokens {
        assert!(token.start_time.is_some() && token.end_time.is_some());
        assert_eq!(
            token.alignment_source,
            Some(pitch_analyzer_tauri_lib::models::AlignmentSource::EnhancedLrc)
        );
    }
}

/// P0-1 修复 (Round4 Commit 2): 纯中文行不再进入 UniDic 日语链路。
/// Commit 1 基线中 `我爱你` 曾被读成 `わ が`。当前 Auto 路由下纯汉字歧义行
/// 保持 Unknown (不悄悄选语言), Mandarin G2P 由 Commit 7 接入。
#[test]
fn snapshot_parse_txt_chinese_no_longer_reads_japanese() {
    let lines = parse_txt(ZH_PLAIN_TXT);
    let summary = summarize(&lines);
    let expected = PipelineSummary {
        line_count: 1,
        token_count: 3,
        mora_count: 0, // 修复: 不再产生日语 mora
        display_group_count: 3, // 1:1 显示组 (读音为空, 仅承载显示时序)
        ruby_annotation_count: 0,
        tokens_without_pronunciation: 3, // Mandarin pipeline 未接入, 诚实计数
        sources: pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
            unaligned: 3,
            ..Default::default()
        },
        language_observation: LanguageObservation {
            han_only_lines: 1,
            ..Default::default()
        },
    };
    assert_eq!(summary, expected, "ZH routing changed unexpectedly");
    assert!(
        lines[0].moras.is_empty(),
        "Chinese lines must not produce Japanese moras"
    );
    // 读音层全空: 无日语假名读音
    assert!(
        lines[0]
            .reading_display_groups
            .iter()
            .all(|g| g.reading.is_empty())
    );
}

#[test]
fn snapshot_parse_txt_timed_chinese_baseline() {
    let lines = parse_txt(ZH_TIMED_TXT);
    let summary = summarize(&lines);
    let expected = PipelineSummary {
        line_count: 2,
        token_count: 5,
        mora_count: 0, // 修复 (Commit 2): 不再产生日语 mora
        display_group_count: 5,
        ruby_annotation_count: 0,
        tokens_without_pronunciation: 5,
        sources: pitch_analyzer_tauri_lib::diagnostics::AlignmentSourceCounts {
            unaligned: 5,
            ..Default::default()
        },
        language_observation: LanguageObservation {
            han_only_lines: 2,
            ..Default::default()
        },
    };
    assert_eq!(summary, expected, "ZH timed baseline drifted");
}

#[test]
fn snapshot_ja_plain_sources_match_helper() {
    // 防止 helper 与实际分布脱节的小交叉验证
    let lines = parse_lrc(JA_PLAIN_LRC, Some(6.0));
    let mut expected = ja_sources_baseline();
    expected.unaligned = 15;
    assert_eq!(summarize(&lines).sources, expected);
}

#[test]
fn snapshot_project_serde_round_trip() {
    let lines = parse_lrc(JA_PLAIN_LRC, Some(6.0));
    let before = summarize(&lines);
    let project = ProjectData {
        audio_path: None,
        pitch_track: None,
        lyrics: lines,
        analysis_params: None,
        language: pitch_analyzer_tauri_lib::models::ProjectLanguage::Auto,
    };
    let json = serde_json::to_string(&project).expect("serialize project");
    let restored: ProjectData = serde_json::from_str(&json).expect("deserialize project");
    assert_eq!(summarize(&restored.lyrics), before);
}
