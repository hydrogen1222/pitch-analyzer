use pitch_analyzer_tauri_lib::export::srt::export_srt;
use pitch_analyzer_tauri_lib::lyrics::{bind_pitch_to_tokens, distribute_token_times, parse_lrc};
use pitch_analyzer_tauri_lib::models::{NoteTrackingParams, PitchTrack};
use std::path::PathBuf;

fn mock_track() -> PitchTrack {
    let times: Vec<f32> = (0..1000).map(|i| i as f32 * 0.01).collect(); // 10s @ 100Hz
    let midis: Vec<f32> = (0..1000).map(|_| 60.0).collect(); // C4
    let conf: Vec<f32> = (0..1000).map(|_| 0.9).collect();
    let freq: Vec<f32> = (0..1000).map(|_| 261.63).collect();
    PitchTrack {
        times,
        frequencies: freq,
        confidences: conf,
        midis,
        rms: Vec::new(),
        note_events: Vec::new(),
    }
}

#[test]
fn test_export_srt_with_lyrics() {
    let lrc = "[00:00.50]Hello world\n[00:03.00]再见世界";
    let mut lines = parse_lrc(lrc, Some(8.0));
    distribute_token_times(&mut lines);
    let track = mock_track();
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());

    // 第一行的 tokens 都应该有 pitch
    let first = &lines[0];
    assert!(first.tokens.iter().any(|t| !t.pitch_notes.is_empty()),
            "expected at least one token to have pitch notes");
    // primary_note 应被设置
    assert!(first.tokens.iter().any(|t| t.primary_note.is_some()),
            "expected primary_note to be set");

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

/// 字内错误首音: F5(40ms) + C4(400ms) → SRT 必须显示 C4 而不是 F5
#[test]
fn test_srt_uses_primary_note_not_first() {
    let track = mock_track();
    // 构造 token 绑定：直接手工构造 pitch_notes + primary_note
    let mut lines = parse_lrc("[00:00.50]Test", Some(8.0));
    distribute_token_times(&mut lines);
    // 覆盖 token 的 pitch_notes: 第一个是短 F5，第二个是长 C4
    let f5 = pitch_analyzer_tauri_lib::models::PitchNote {
        start_time: 0.50,
        end_time: 0.54,
        median_midi: 77.0, // F5
        mean_midi: 77.0,
        rounded_midi: 77,
        confidence_mean: 0.8,
        point_count: 4,
    };
    let c4 = pitch_analyzer_tauri_lib::models::PitchNote {
        start_time: 0.54,
        end_time: 0.94,
        median_midi: 60.0, // C4
        mean_midi: 60.0,
        rounded_midi: 60,
        confidence_mean: 0.8,
        point_count: 40,
    };
    for line in lines.iter_mut() {
        for token in line.tokens.iter_mut() {
            token.pitch_notes = vec![f5.clone(), c4.clone()];
        }
    }
    // 手动调用与后端一致的 primary 选择
    for line in lines.iter_mut() {
        for token in line.tokens.iter_mut() {
            token.primary_note =
                pitch_analyzer_tauri_lib::lyrics::select_primary_note(&token.pitch_notes, 0.045, 0.3);
        }
    }

    let out = PathBuf::from("/tmp/test_srt_primary.srt");
    export_srt(&track, &lines, &out).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    println!("SRT primary content:\n{}", content);
    assert!(content.contains("[C4]"), "expected C4 as primary, got:\n{}", content);
    assert!(!content.contains("[F5]"), "F5 must not be the display note");
}
