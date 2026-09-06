// Round4 Commit 5: v4 canonical data model tests
//
// 验收 (任务书 Commit 5): SourceSpan / PronunciationUnit / PhonemeUnit /
// AcousticSpan / PitchAttachment 类型 + 旧结构 projection + serde round-trip。

use pitch_analyzer_tauri_lib::alignment::projection::{project_line_v4, LineV4Layers};
use pitch_analyzer_tauri_lib::lyrics::{parse_lrc_with_language, parse_txt_with_language};
use pitch_analyzer_tauri_lib::models::{
    AcousticSpan, AlignmentBackendKind, AlignmentQuality, F0Summary, PhonemeUnit, PitchAttachment,
    PitchAttachmentStatus, PitchBearingPolicy, ProjectLanguage, PronunciationUnit, SingingLanguage,
    SourceSpan,
};

fn project_ja(text: &str) -> LineV4Layers {
    let lines = parse_lrc_with_language(
        &format!("[00:01.00]{}", text),
        Some(6.0),
        ProjectLanguage::Japanese,
    );
    project_line_v4(&lines[0], SingingLanguage::Japanese)
}

#[test]
fn v4_projection_from_old_layers_japanese() {
    let layers = project_ja("二人(ふたり)");
    // 原文层: 二人 (Han, 词级 span) + (ふたり) 被移除 → reading span 覆盖原文
    assert!(!layers.source_spans.is_empty());
    // 发音层: ふ・た・り 三个 unit, 全部挂到 二人 的 span (不伪造逐字映射)
    let units: Vec<_> = layers
        .pronunciation_units
        .iter()
        .map(|u| u.text.as_str())
        .collect();
    assert_eq!(units, vec!["ふ", "た", "り"]);
    let han_span = layers
        .source_spans
        .iter()
        .find(|s| s.source_kind == pitch_analyzer_tauri_lib::models::SourceKind::Han)
        .expect("han span");
    assert_eq!(han_span.pronunciation_ids.len(), 3);
}

#[test]
fn v4_projection_chinese_has_no_fabricated_pronunciation() {
    // 中文歧义行: 无读音 → 不伪造 PronunciationUnit (Commit 7 接入 Mandarin G2P)
    let lines = parse_txt_with_language("我爱你\n", ProjectLanguage::Auto);
    let layers = project_line_v4(&lines[0], SingingLanguage::Unknown);
    assert!(layers.pronunciation_units.is_empty());
    assert_eq!(layers.source_spans.len(), 3);
    assert!(layers
        .source_spans
        .iter()
        .all(|s| s.source_kind == pitch_analyzer_tauri_lib::models::SourceKind::Han));
    assert!(layers.phonemes.is_empty());
}

#[test]
fn v4_serde_round_trip_all_layers() {
    let layers = project_ja("きゃっと");
    let json = serde_json::to_string_pretty(&layers).expect("serialize LineV4Layers");
    let restored: LineV4Layers = serde_json::from_str(&json).expect("deserialize LineV4Layers");
    assert_eq!(restored.source_spans.len(), layers.source_spans.len());
    assert_eq!(
        restored.pronunciation_units.len(),
        layers.pronunciation_units.len()
    );
    assert_eq!(restored.phonemes.len(), layers.phonemes.len());
    assert_eq!(restored.pronunciation_units, layers.pronunciation_units);
    assert_eq!(restored.phonemes, layers.phonemes);
}

#[test]
fn v4_standalone_type_serde_defaults() {
    // AcousticSpan: 未 calibration 时 calibrated_probability 必须显式 None
    let span = AcousticSpan {
        start: 1.0,
        end: 1.4,
        backend: AlignmentBackendKind::LegacyMms,
        quality: AlignmentQuality::Medium,
        raw_score: Some(2.71),
        calibrated_probability: None,
    };
    let json = serde_json::to_string(&span).unwrap();
    let restored: AcousticSpan = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.calibrated_probability, None);
    assert_eq!(restored.quality, AlignmentQuality::Medium);

    // PitchAttachment: many-to-many (1 unit -> N notes) 合法
    let attachment = PitchAttachment {
        pronunciation_unit_id: 3,
        nucleus_span: Some((1.2, 1.5)),
        note_ids: vec![10, 11, 12],
        primary_note_id: None, // Ambiguous: 不强行挑一个
        f0_summary: Some(F0Summary {
            voiced_coverage: 0.91,
            median_midi: Some(64.18),
            mad_cents: Some(17.0),
            p10_midi: Some(63.5),
            p90_midi: Some(64.9),
            slope_cents_per_sec: None,
        }),
        status: PitchAttachmentStatus::Ambiguous,
    };
    let json = serde_json::to_string(&attachment).unwrap();
    let restored: PitchAttachment = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.note_ids, vec![10, 11, 12]);
    assert_eq!(restored.primary_note_id, None);
    assert_eq!(restored.status, PitchAttachmentStatus::Ambiguous);
    assert_eq!(
        restored.f0_summary.as_ref().unwrap().median_midi,
        Some(64.18)
    );

    // 旧工程 JSON 兼容: 缺省字段 serde default 填充
    let minimal: SourceSpan =
        serde_json::from_str(r#"{"id":0,"surface":"今","char_start":0,"char_end":1}"#).unwrap();
    assert_eq!(minimal.language, SingingLanguage::Unknown);
    assert_eq!(
        minimal.source_kind,
        pitch_analyzer_tauri_lib::models::SourceKind::Unknown
    );
    let minimal_unit: PronunciationUnit =
        serde_json::from_str(r#"{"id":0,"text":"きょ"}"#).unwrap();
    assert_eq!(
        minimal_unit.kind,
        pitch_analyzer_tauri_lib::models::PronunciationUnitKind::Unknown
    );
    let minimal_ph: PhonemeUnit = serde_json::from_str(r#"{"id":0,"symbol":"a"}"#).unwrap();
    assert!(!minimal_ph.pitch_bearing);
    assert_eq!(
        minimal_ph.class,
        pitch_analyzer_tauri_lib::models::PhonemeClass::Unknown
    );
    let _ = PitchBearingPolicy::Nucleus;
}

#[test]
fn v4_old_projectdata_still_loads() {
    // 无 language/schema 字段的旧工程 JSON 必须继续加载 (serde default)
    let old_project = r#"{
        "audio_path": null,
        "pitch_track": null,
        "lyrics": [],
        "analysis_params": null
    }"#;
    let restored: pitch_analyzer_tauri_lib::models::ProjectData =
        serde_json::from_str(old_project).unwrap();
    assert_eq!(restored.language, ProjectLanguage::Auto);
    assert!(restored.lyrics.is_empty());
}
