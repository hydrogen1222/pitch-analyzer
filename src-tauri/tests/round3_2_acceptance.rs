use pitch_analyzer_tauri_lib::japanese::mora::parse_kana_moras;
use pitch_analyzer_tauri_lib::japanese::reading::parse_ruby_text;
use pitch_analyzer_tauri_lib::lyrics::{bind_pitch_to_tokens, build_align_units_debug, parse_lrc};
use pitch_analyzer_tauri_lib::models::{NoteTrackingParams, PitchTrack, SungNoteClass};
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

    let groups: Vec<_> = line
        .reading_display_groups
        .iter()
        .filter(|group| group.surface == "二人" && group.phonetic)
        .collect();
    assert_eq!(groups.len(), 3, "ふ・た・り must be three kana pitch slots");
    assert_eq!(
        groups
            .iter()
            .map(|group| group.reading.as_str())
            .collect::<String>(),
        "ふたり"
    );
    assert!(groups.iter().all(|group| {
        group.char_end - group.char_start == 2
            && group.mora_end - group.mora_start == 1
            && line
                .moras
                .iter()
                .filter(|mora| mora.display_group_id == group.id)
                .count()
                == 1
    }));
    let units = build_align_units_debug(line);
    assert_eq!(
        units.len(),
        line.moras.len(),
        "moras must not duplicate per glyph"
    );
}

#[test]
fn partial_ruby_keeps_neighboring_kana_and_exact_kana_gets_mora_groups() {
    let lines = parse_lrc(
        "[00:40.980]悲(かな)しい位(くらい)に見(み)つめてた[00:49.240]",
        Some(50.0),
    );
    let line = &lines[0];
    let phonetic_row: String = line
        .reading_display_groups
        .iter()
        .map(|group| {
            if group.reading.is_empty() {
                group.surface.as_str()
            } else {
                group.reading.as_str()
            }
        })
        .collect();
    assert_eq!(
        phonetic_row, "かなしいくらいにみつめてた",
        "explicit ruby must become the canonical kana pitch row"
    );
    assert_eq!(line.primary_text, "悲しい位に見つめてた");

    let surfaces: Vec<&str> = line
        .reading_display_groups
        .iter()
        .map(|group| group.surface.as_str())
        .collect();
    assert!(surfaces.windows(2).any(|pair| pair == ["し", "い"]));
    assert!(surfaces.windows(2).any(|pair| pair == ["つ", "め"]));
    assert!(line
        .reading_display_groups
        .iter()
        .any(|group| group.surface == "悲" && group.phonetic && group.reading == "か"));
    assert!(!line
        .reading_display_groups
        .iter()
        .any(|group| group.reading == "する" || group.reading == "てる"));
}

#[test]
fn kana_surface_uses_one_display_group_per_mora() {
    let line = &parse_lrc("[00:00.000]せつない", Some(2.0))[0];
    let surfaces: Vec<&str> = line
        .reading_display_groups
        .iter()
        .map(|group| group.surface.as_str())
        .collect();
    assert_eq!(surfaces, ["せ", "つ", "な", "い"]);
    assert!(line.reading_display_groups.iter().all(|group| {
        group.mora_end - group.mora_start == 1
            && line
                .moras
                .iter()
                .filter(|mora| mora.display_group_id == group.id)
                .count()
                == 1
    }));
}

#[test]
fn explicit_ruby_mora_can_claim_adjacent_real_vowel_onset() {
    let mut lines = parse_lrc("[00:00.000]背伸(せの)[00:00.800]", Some(1.0));
    assert_eq!(lines[0].reading_display_groups.len(), 2);
    lines[0].reading_display_groups[0].start_time = Some(0.0);
    lines[0].reading_display_groups[0].end_time = Some(0.30);
    lines[0].reading_display_groups[1].start_time = Some(0.30);
    lines[0].reading_display_groups[1].end_time = Some(0.80);

    let mut track = fcpe_track(62.0);
    track.musical_notes = vec![game_event(1, 0.30, 0.80, 62.0)];
    track.musical_note_source = MusicalNoteSource::Game;
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());

    let groups = &lines[0].reading_display_groups;
    assert_eq!(groups[0].reading, "せ");
    assert!(
        groups[0].primary_note.is_some(),
        "the consonant-leading mora must use the adjacent real vowel onset"
    );
    assert!(groups[1].primary_note.is_some());
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

fn fcpe_track_segments(segments: &[(f32, f32, f32)]) -> PitchTrack {
    let times: Vec<f32> = (0..200).map(|i| i as f32 * 0.01).collect();
    let midis: Vec<f32> = times
        .iter()
        .map(|&time| {
            segments
                .iter()
                .find(|&&(start, end, _)| time >= start && time < end)
                .map(|&(_, _, midi)| midi)
                .unwrap_or(f32::NAN)
        })
        .collect();
    let confidences: Vec<f32> = midis
        .iter()
        .map(|midi| if midi.is_finite() { 0.95 } else { 0.0 })
        .collect();
    PitchTrack {
        times,
        frequencies: vec![440.0; midis.len()],
        confidences,
        midis,
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

    // The FCPE contour must follow the GAME candidate windows. A constant
    // FCPE fixture would make the second GAME event uncertain and falsely
    // make this test pass merely because debug candidates are retained.
    let melisma_track = fcpe_track_segments(&[
        (0.0, 0.40, 67.0),  // G4
        (0.40, 0.80, 69.0), // A4
        (0.80, 1.20, 67.0), // G4
    ]);
    let melisma = processor.process(
        &[
            game_event(0, 0.0, 0.40, 67.0),
            game_event(1, 0.40, 0.80, 69.0),
            game_event(2, 0.80, 1.20, 67.0),
        ],
        &melisma_track,
    );
    assert_eq!(
        melisma.len(),
        3,
        "a stable three-note melody must remain three candidates"
    );
    assert!(melisma
        .iter()
        .all(|note| note.class == SungNoteClass::Stable));
    let accepted = processor.accepted_events(&melisma);
    assert_eq!(
        accepted.len(),
        3,
        "all stable melisma notes must be accepted"
    );
    assert_eq!(accepted[0].midi_rounded, 67);
    assert_eq!(accepted[1].midi_rounded, 69);
    assert_eq!(accepted[2].midi_rounded, 67);

    // A short, mismatched fragment sharing the stable note name must not
    // poison the longer same-pitch platform after consolidation.
    let poisoning_track = fcpe_track_segments(&[
        (0.0, 0.30, 69.0),  // long A4 platform
        (0.30, 0.33, 65.0), // short, contradictory FCPE fragment
    ]);
    let poisoned = processor.process(
        &[
            game_event(0, 0.0, 0.30, 69.0),
            game_event(1, 0.30, 0.33, 69.0),
        ],
        &poisoning_track,
    );
    assert_eq!(poisoned.len(), 1, "same-pitch fragments should consolidate");
    assert_eq!(poisoned[0].class, SungNoteClass::Stable);
    assert_eq!(processor.accepted_events(&poisoned).len(), 1);
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
