use pitch_analyzer_tauri_lib::lyrics::{export_srt, parse_lrc, distribute_token_times, bind_pitch_to_tokens};
use pitch_analyzer_tauri_lib::models::PitchTrack;
use std::path::PathBuf;

fn mock_track() -> PitchTrack {
    let times: Vec<f32> = (0..1000).map(|i| i as f32 * 0.01).collect(); // 10s @ 100Hz
    let midis: Vec<f32> = (0..1000).map(|_| 60.0).collect(); // C4
    let conf: Vec<f32> = (0..1000).map(|_| 0.9).collect();
    let freq: Vec<f32> = (0..1000).map(|_| 261.63).collect();
    PitchTrack { times, frequencies: freq, confidences: conf, midis }
}

#[test]
fn test_export_srt_with_lyrics() {
    let lrc = "[00:00.50]Hello world\n[00:03.00]再见世界";
    let mut lines = parse_lrc(lrc, Some(8.0));
    distribute_token_times(&mut lines);
    let track = mock_track();
    bind_pitch_to_tokens(&mut lines, &track, 0.3);

    // 第一行的 tokens 都应该有 pitch
    let first = &lines[0];
    assert!(first.tokens.iter().any(|t| !t.pitch_notes.is_empty()),
            "expected at least one token to have pitch notes");

    let out = PathBuf::from("/tmp/test_export.srt");
    export_srt(&track, &lines, &out).unwrap();

    let content = std::fs::read_to_string(&out).unwrap();
    println!("SRT content:\n{}", content);
    assert!(content.contains("-->"));
    assert!(content.contains("[C4]")); // MIDI 60 = C4
    assert!(content.contains("Hello"));
}

#[test]
fn test_export_srt_without_lyrics() {
    let track = mock_track();
    let out = PathBuf::from("/tmp/test_export_no_lyrics.srt");
    export_srt(&track, &[], &out).unwrap();

    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("C4"));
    assert!(content.contains("-->"));
}
