// ASS / SRT 导出测试 (§: ASS 字幕导出, SRT 使用 primary_note)

use pitch_analyzer_tauri_lib::export::ass::export_ass;
use pitch_analyzer_tauri_lib::export::srt::export_srt;
use pitch_analyzer_tauri_lib::lyrics::{distribute_token_times, parse_lrc, select_primary_note};
use pitch_analyzer_tauri_lib::models::{PitchNote, PitchTrack};
use std::path::PathBuf;

fn track() -> PitchTrack {
    let n = 800;
    let times: Vec<f32> = (0..n).map(|i| i as f32 * 0.01).collect();
    PitchTrack {
        times,
        frequencies: vec![261.63; n],
        confidences: vec![0.9; n],
        midis: vec![60.0; n],
        rms: Vec::new(),
        note_events: Vec::new(),
    }
}

/// 给每个 token 手工设置 primary_note (C4)
fn attach_primary(lines: &mut [pitch_analyzer_tauri_lib::models::LyricLine]) {
    for line in lines.iter_mut() {
        for tok in line.tokens.iter_mut() {
            tok.primary_note = Some(PitchNote {
                start_time: tok.start_time.unwrap_or(0.0),
                end_time: tok.end_time.unwrap_or(1.0),
                median_midi: 60.0,
                mean_midi: 60.0,
                rounded_midi: 60,
                confidence_mean: 0.9,
                point_count: 10,
            });
        }
    }
}

#[test]
fn test_export_ass_karaoke() {
    let mut lines = parse_lrc("[00:01.00]你好世界", Some(10.0));
    distribute_token_times(&mut lines);
    attach_primary(&mut lines);

    let out = PathBuf::from("/tmp/test_export.ass");
    export_ass(&lines, &out, 40, 28).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();

    // 头部样式
    assert!(content.contains("[Script Info]"));
    assert!(content.contains("PlayResX: 1280"));
    assert!(content.contains("Style: Pitch,"));
    assert!(content.contains("Style: Lyric,"));
    // karaoke 标签 + 音高
    assert!(content.contains("{\\k"));
    assert!(content.contains("Dialogue: 0,"));
    assert!(content.contains("C4"), "pitch dialogue must contain note name C4:\n{}", content);
}

#[test]
fn test_export_ass_uses_primary_not_first() {
    // 每个 token 有两个候选 (短 F5 + 长 C4)，但 primary_note 手工设为 C4 → 显示 C4
    let mut lines = parse_lrc("[00:01.00]Test", Some(10.0));
    distribute_token_times(&mut lines);
    attach_primary(&mut lines);
    // 篡改 pitch_notes 为 "短 F5 优先"，但 primary_note 已固定为 C4
    for line in lines.iter_mut() {
        for tok in line.tokens.iter_mut() {
            tok.pitch_notes = vec![
                PitchNote {
                    start_time: 1.0,
                    end_time: 1.04,
                    median_midi: 77.0,
                    mean_midi: 77.0,
                    rounded_midi: 77,
                    confidence_mean: 0.8,
                    point_count: 4,
                },
                PitchNote {
                    start_time: 1.04,
                    end_time: 1.5,
                    median_midi: 60.0,
                    mean_midi: 60.0,
                    rounded_midi: 60,
                    confidence_mean: 0.8,
                    point_count: 46,
                },
            ];
        }
    }

    let out = PathBuf::from("/tmp/test_export_primary.ass");
    export_ass(&lines, &out, 40, 28).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("C4"));
    assert!(!content.contains("F5"), "ASS must display primary_note (C4), not first note F5");
}

#[test]
fn test_export_ass_no_lyrics_error() {
    let out = PathBuf::from("/tmp/test_empty.ass");
    let err = export_ass(&[], &out, 40, 28).unwrap_err();
    assert!(err.contains("没有歌词"), "expected no-lyrics error, got: {}", err);
}

#[test]
fn test_export_srt_primary_fallback() {
    // 旧工程无 primary_note → 按评分回退到 C4
    let track = track();
    let mut lines = parse_lrc("[00:01.00]Test", Some(10.0));
    distribute_token_times(&mut lines);
    for line in lines.iter_mut() {
        for tok in line.tokens.iter_mut() {
            tok.pitch_notes = vec![
                PitchNote {
                    start_time: 1.0,
                    end_time: 1.04,
                    median_midi: 77.0,
                    mean_midi: 77.0,
                    rounded_midi: 77,
                    confidence_mean: 0.8,
                    point_count: 4,
                },
                PitchNote {
                    start_time: 1.04,
                    end_time: 1.5,
                    median_midi: 60.0,
                    mean_midi: 60.0,
                    rounded_midi: 60,
                    confidence_mean: 0.8,
                    point_count: 46,
                },
            ];
            tok.primary_note =
                select_primary_note(&tok.pitch_notes, 0.0, 0.0);
        }
    }
    let out = PathBuf::from("/tmp/test_srt_fallback.srt");
    export_srt(&track, &lines, &out).unwrap();
    let content = std::fs::read_to_string(&out).unwrap();
    assert!(content.contains("[C4]"), "SRT fallback must choose C4:\n{}", content);
    assert!(!content.contains("[F5]"));
}
