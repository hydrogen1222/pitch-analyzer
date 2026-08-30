use pitch_analyzer_tauri_lib::models::NoteTrackingParams;
use pitch_analyzer_tauri_lib::note_engine::{LegacyFcpeNoteTracker, MusicalNoteEngine, MusicalNoteSource};

#[test]
fn test_legacy_fcpe_note_tracker_engine() {
    let tracker = LegacyFcpeNoteTracker::default();
    assert_eq!(tracker.name(), "legacy-fcpe-v2");

    // 100 frames (1s) of stable A4 (69.0)
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let midis = vec![69.0; 100];
    let confidences = vec![0.9; 100];

    let events = tracker.transcribe_from_track(&times, &midis, &confidences);
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.note_name, "A4");
    assert_eq!(ev.midi_rounded, 69);
    assert_eq!(ev.source, MusicalNoteSource::LegacyFcpeTracker);
    assert!((ev.start - 0.0).abs() < 0.05);
    assert!((ev.end - 1.0).abs() < 0.05);
}

#[test]
fn test_vibrato_transcription_stays_single_event() {
    let tracker = LegacyFcpeNoteTracker::new(NoteTrackingParams::default());
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    // Vibrato around 69.0 ± 0.4 semitones at 5.5 Hz
    let midis: Vec<f32> = times
        .iter()
        .map(|&t| 69.0 + 0.4 * (2.0 * std::f32::consts::PI * 5.5 * t).sin())
        .collect();
    let confidences = vec![0.95; 100];

    let events = tracker.transcribe_from_track(&times, &midis, &confidences);
    assert_eq!(events.len(), 1, "Vibrato must transcribe to exactly 1 MusicalNoteEvent");
    assert_eq!(events[0].note_name, "A4");
    assert_eq!(events[0].midi_rounded, 69);
}
