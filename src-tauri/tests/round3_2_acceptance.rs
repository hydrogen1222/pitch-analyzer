use pitch_analyzer_tauri_lib::japanese::mora::parse_kana_moras;
use pitch_analyzer_tauri_lib::japanese::reading::parse_ruby_text;
use pitch_analyzer_tauri_lib::lyrics::{build_align_units_debug, parse_lrc};
use pitch_analyzer_tauri_lib::models::{PitchTrack, SungNoteClass};
use pitch_analyzer_tauri_lib::note_engine::game::{
    assert_game_parity, compare_game_reference, GameReferenceNote,
};
use pitch_analyzer_tauri_lib::note_engine::postprocess::CanonicalNotePostProcessor;
use pitch_analyzer_tauri_lib::note_engine::{MusicalNoteEvent, MusicalNoteSource};

#[test]
fn ruby_parser_uses_display_coordinates_and_keeps_literal_parentheses() {
    let parsed = parse_ruby_text("冷(つめ)たく微笑(ほほえ)んだ").expect("ruby parse");
    assert_eq!(parsed.display_text, "冷たく微笑んだ");
    assert_eq!(parsed.annotations.len(), 2);
    assert_eq!(parsed.annotations[0].surface, "冷");
    assert_eq!(parsed.annotations[0].reading, "つめ");
    assert_eq!(
        (
            parsed.annotations[0].display_start,
            parsed.annotations[0].display_end
        ),
        (0, 1)
    );
    assert_eq!(parsed.annotations[1].surface, "微笑");
    assert_eq!(parsed.annotations[1].reading, "ほほえ");
    assert_eq!(
        (
            parsed.annotations[1].display_start,
            parsed.annotations[1].display_end
        ),
        (3, 5)
    );

    for text in [
        "(コーラス)",
        "（笑）",
        "(yeah)",
        "愛(",
        "愛()",
        "愛(あい",
        "愛(ai)",
    ] {
        let parsed = parse_ruby_text(text).expect("malformed ruby must be safe");
        assert!(
            parsed.annotations.is_empty(),
            "literal text was misclassified: {text}"
        );
    }
    let escaped = parse_ruby_text(r"愛\(あい\)").expect("escaped ruby");
    assert_eq!(escaped.display_text, "愛(あい)");
    assert!(escaped.annotations.is_empty());
}

#[test]
fn ruby_reading_priority_and_multiglyph_group_are_canonical() {
    let lines = parse_lrc("[00:00.000]運命(さだめ)と二人(ふたり)", Some(2.0));
    let line = &lines[0];
    assert_eq!(line.primary_text, "運命と二人");
    let fate = line
        .reading_spans
        .iter()
        .find(|span| span.surface == "運命")
        .expect("ruby span");
    assert_eq!(fate.reading, "さだめ");
    assert_eq!(parse_kana_moras(&fate.reading).len(), 3);

    let group = line
        .reading_display_groups
        .iter()
        .find(|group| group.surface == "二人")
        .expect("二人 display group");
    assert_eq!(group.reading, "ふたり");
    assert_eq!(parse_kana_moras(&group.reading).len(), 3);
    assert_eq!(group.char_end - group.char_start, 2);
    assert!(line
        .moras
        .iter()
        .filter(|mora| mora.display_group_id == group.id)
        .all(|mora| mora.display_start == group.char_start && mora.display_end == group.char_end));
    assert_eq!(
        line.tokens
            .iter()
            .filter(|token| token.reading_group_ids.contains(&group.id))
            .count(),
        2,
        "both glyphs share the display group, not separate acoustic units"
    );
    let units = build_align_units_debug(line);
    assert_eq!(
        units.len(),
        line.moras.len(),
        "moras must not duplicate per glyph"
    );
}

#[test]
fn ruby_and_enhanced_lrc_anchors_share_display_coordinates() {
    let lines = parse_lrc(
        "[00:00.000]<00:00.000>冷(つめ)<00:00.500>たく[00:01.000]\n[00:00.000]中文翻译[00:01.000]",
        Some(2.0),
    );
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line.primary_text, "冷たく");
    assert_eq!(line.translations, vec!["中文翻译"]);
    assert_eq!(line.ruby_annotations[0].display_start, 0);
    assert_eq!(line.reading_display_groups[0].start_time, Some(0.0));
    assert_eq!(line.reading_display_groups[0].end_time, Some(0.5));
    assert_eq!(line.tokens[0].start_time, Some(0.0));
    assert_eq!(line.tokens[0].end_time, Some(0.5));
}

fn game_event(id: u32, start: f32, end: f32, midi: f32) -> MusicalNoteEvent {
    MusicalNoteEvent {
        id,
        start,
        end,
        midi_float: midi,
        midi_rounded: midi.round() as i32,
        note_name: pitch_analyzer_tauri_lib::models::midi_to_note_name(midi),
        confidence: 0.0,
        source: MusicalNoteSource::Game,
        model_confidence: None,
        boundary_confidence: None,
        is_slur: None,
        evidence: None,
        class: None,
    }
}

fn fcpe_track(midi: f32) -> PitchTrack {
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    PitchTrack {
        times: times.clone(),
        frequencies: vec![440.0; times.len()],
        confidences: vec![0.95; times.len()],
        midis: vec![midi; times.len()],
        ..Default::default()
    }
}

#[test]
fn game_fcpe_evidence_and_consolidation_are_conservative() {
    let processor = CanonicalNotePostProcessor::default();
    let mut track = fcpe_track(69.0);
    let notes = processor.process(
        &[
            game_event(0, 0.0, 0.30, 69.0),
            game_event(1, 0.30, 0.60, 69.0),
        ],
        &track,
    );
    assert_eq!(notes.len(), 1, "adjacent same-pitch events must merge");
    assert_eq!(notes[0].class, SungNoteClass::Stable);
    assert_eq!(notes[0].event_ids, vec![0, 1]);
    assert!(notes[0].fcpe_support > 0.9);

    track.midis.fill(65.0);
    let mismatch = processor.process(&[game_event(0, 0.0, 1.0, 69.0)], &track);
    assert_eq!(mismatch[0].class, SungNoteClass::Uncertain);
    assert!(processor.accepted_events(&mismatch).is_empty());

    let melisma = processor.process(
        &[
            game_event(0, 0.0, 0.40, 69.0),
            game_event(1, 0.40, 0.80, 71.0),
        ],
        &fcpe_track(69.0),
    );
    assert_eq!(
        melisma.len(),
        2,
        "a stable two-semitone transition is meaningful"
    );
}

#[test]
fn game_reference_comparator_reports_acceptance_metrics() {
    let actual = vec![game_event(0, 0.0, 0.5, 60.0), game_event(1, 0.5, 1.0, 62.0)];
    let reference = vec![
        GameReferenceNote {
            start: 0.01,
            end: 0.49,
            midi: 60.0,
        },
        GameReferenceNote {
            start: 0.51,
            end: 1.01,
            midi: 62.0,
        },
    ];
    let report = compare_game_reference(&actual, &reference);
    assert_game_parity(&report).expect("parity fixture should pass");
}
