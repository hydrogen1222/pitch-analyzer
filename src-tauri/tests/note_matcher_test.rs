// Round4 Commit 9: LyricNoteMatcher v2 pipeline integration tests

use pitch_analyzer_tauri_lib::alignment::note_matcher::attach_lines;
use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, parse_lrc_with_language, TokenAlignParams,
};
use pitch_analyzer_tauri_lib::models::{
    PitchAttachmentStatus, PitchTrack, ProjectData, ProjectLanguage,
};
use pitch_analyzer_tauri_lib::note_engine::{MusicalNoteEvent, MusicalNoteSource};

fn game_note(id: u32, start: f32, end: f32, midi: f32) -> MusicalNoteEvent {
    MusicalNoteEvent {
        id,
        start,
        end,
        midi_float: midi,
        midi_rounded: midi.round() as i32,
        note_name: pitch_analyzer_tauri_lib::models::midi_to_note_name(midi),
        confidence: 1.0,
        source: MusicalNoteSource::Game,
        model_confidence: None,
        boundary_confidence: None,
        is_slur: None,
        evidence: None,
        class: None,
    }
}

fn voiced_track(notes: Vec<MusicalNoteEvent>) -> PitchTrack {
    let mut track = PitchTrack {
        times: (0..500).map(|i| i as f32 * 0.01).collect(),
        ..Default::default()
    };
    track.frequencies = vec![440.0; 500];
    track.confidences = vec![0.9; 500];
    track.midis = vec![f32::NAN; 500];
    for note in &notes {
        for i in 0..500 {
            let t = i as f32 * 0.01;
            if t >= note.start && t < note.end {
                track.midis[i] = note.midi_float;
            }
        }
    }
    track.musical_notes = notes;
    track.musical_note_source = MusicalNoteSource::Game;
    track
}

/// 日语行: mora 级 attachments 与旧 binder 并行产生; 促音 NonPitchBearing
#[test]
fn japanese_line_attachments_with_sokuon_policy() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]がっこう\n[00:05.00]スーパー",
        Some(10.0),
        ProjectLanguage::Japanese,
    );
    let track = voiced_track(vec![game_note(0, 1.0, 4.5, 64.0)]);
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    attach_lines(&mut lines, &track, 0.3);

    let attachments = &lines[0].pitch_attachments;
    // がっこう = が っ こ う → 4 units
    assert_eq!(attachments.len(), 4);
    // っ (unit 1) → NonPitchBearing
    assert_eq!(
        attachments[1].status,
        PitchAttachmentStatus::NonPitchBearing
    );
    assert!(attachments[1].note_ids.is_empty());
    // 其余 voiced 单位与 1.0-4.5 的 E4 音符真实重叠 → Reliable
    for (i, attachment) in attachments.iter().enumerate() {
        if i == 1 {
            continue;
        }
        assert_eq!(
            attachment.status,
            PitchAttachmentStatus::Reliable,
            "unit {i} must reliably attach"
        );
        assert_eq!(attachment.note_ids, vec![0]);
        assert!(attachment.f0_summary.is_some());
        assert!(attachment.f0_summary.as_ref().unwrap().voiced_coverage > 0.5);
    }
}

/// melisma: 一个 mora 覆盖多个 GAME note → note_ids 保留全部
#[test]
fn melisma_keeps_all_notes() {
    let mut lines = parse_lrc_with_language(
        // DP 需要 >= 2 个对齐单元; "ああ" 提供两个 mora,
        // 第一个 mora (约 1.0-2.0) 依次覆盖 C4/D4 → melisma
        "[00:01.00]ああ",
        Some(4.0),
        ProjectLanguage::Japanese,
    );
    let track = voiced_track(vec![
        game_note(0, 1.0, 1.5, 60.0),
        game_note(1, 1.5, 2.0, 62.0),
        game_note(2, 2.0, 2.5, 64.0),
    ]);
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    attach_lines(&mut lines, &track, 0.3);

    let attachments = &lines[0].pitch_attachments;
    assert_eq!(attachments.len(), 2);
    let first = &attachments[0];
    assert!(
        first.note_ids.len() >= 2,
        "melisma must preserve every overlapping note, got {:?}",
        first.note_ids
    );
    assert_eq!(first.status, PitchAttachmentStatus::Reliable);
}

/// 无对齐 → AlignmentMissing; 无音符 → NoOverlappingNote
#[test]
fn honest_statuses_without_evidence() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみ\n[00:03.00]ら",
        Some(6.0),
        ProjectLanguage::Japanese,
    );
    // 核内有声 (FCPE voiced) 但唯一的音符远在 8-9s → 真实重叠为零
    let mut track = PitchTrack {
        times: (0..500).map(|i| i as f32 * 0.01).collect(),
        ..Default::default()
    };
    track.frequencies = vec![440.0; 500];
    track.confidences = vec![0.9; 500];
    track.midis = vec![60.0; 500]; // 全程 voiced @ C4
    track.musical_notes = vec![game_note(0, 8.0, 9.0, 60.0)]; // 远离本行
    track.musical_note_source = MusicalNoteSource::Game;
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    attach_lines(&mut lines, &track, 0.3);
    assert!(
        lines[0]
            .pitch_attachments
            .iter()
            .any(|a| a.status == PitchAttachmentStatus::NoOverlappingNote),
        "voiced unit with zero real overlap must be NoOverlappingNote"
    );

    // 完全无时间: parse 后直接 attach → AlignmentMissing
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみ\n[00:03.00]ら",
        Some(6.0),
        ProjectLanguage::Japanese,
    );
    let empty = PitchTrack {
        times: (0..100).map(|i| i as f32 * 0.01).collect(),
        frequencies: vec![440.0; 100],
        confidences: vec![0.9; 100],
        midis: vec![f32::NAN; 100],
        musical_notes: vec![game_note(0, 1.0, 2.0, 60.0)],
        musical_note_source: MusicalNoteSource::Game,
        ..Default::default()
    };
    attach_lines(&mut lines, &empty, 0.3);
    assert!(lines[0]
        .pitch_attachments
        .iter()
        .all(|a| a.status == PitchAttachmentStatus::AlignmentMissing));
}

/// attachments 随 ProjectData serde round-trip 保留 (旧工程缺字段可加载)
#[test]
fn attachments_survive_serde_round_trip() {
    let mut lines =
        parse_lrc_with_language("[00:01.00]がっこう", Some(6.0), ProjectLanguage::Japanese);
    // 注意: 全程 voiced (无 NaN 帧) —— NaN 的 JSON 序列化为 null 是
    // ProjectData 的存量限制 (见 Round4 实施报告"已知限制")
    let mut track = voiced_track(vec![game_note(0, 1.0, 3.0, 64.0)]);
    for midi in track.midis.iter_mut() {
        if midi.is_nan() {
            *midi = 64.0;
        }
    }
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    attach_lines(&mut lines, &track, 0.3);
    let before = lines[0].pitch_attachments.clone();
    assert!(!before.is_empty());

    let project = ProjectData {
        audio_path: None,
        pitch_track: Some(track),
        lyrics: lines,
        analysis_params: None,
        language: ProjectLanguage::Auto,
    };
    let json = serde_json::to_string(&project).unwrap();
    let restored: ProjectData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.lyrics[0].pitch_attachments, before);

    // 旧工程 JSON (无新字段) 可加载
    let old: ProjectData =
        serde_json::from_str(r#"{"audio_path":null,"pitch_track":null,"lyrics":[],"analysis_params":null,"language":"Auto"}"#)
            .unwrap();
    assert!(old.lyrics.is_empty());
}
