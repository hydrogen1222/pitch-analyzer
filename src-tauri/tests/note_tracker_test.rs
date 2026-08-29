// Note Event Tracker 回归测试 (任务书 §14 场景)
//
// 统一输入: 10ms hop 的 clean midi 序列 (DSP 已输出，故这里直接用稳定值)
// 默认参数: min_note_duration_ms=45 → 5 frames; switch_confirm_ms=60 → 6 frames;
//           octave_error_max_ms=80 → 8 frames; hysteresis=25 cents

use pitch_analyzer_tauri_lib::models::NoteTrackingParams;
use pitch_analyzer_tauri_lib::note_tracker::build_note_events;

fn run(midis: &[f32]) -> Vec<(i32, f32, f32)> {
    let n = midis.len();
    let times: Vec<f32> = (0..n).map(|i| i as f32 * 0.01).collect();
    let conf: Vec<f32> = vec![0.8; n];
    let params = NoteTrackingParams::default();
    build_note_events(&times, midis, &conf, &params)
        .into_iter()
        .map(|ev| (ev.midi, ev.start, ev.end))
        .collect()
}

fn midis_from(vals: &[(f32, usize)]) -> Vec<f32> {
    vals.iter().flat_map(|&(m, len)| vec![m; len]).collect()
}

/// 短暂八度错误: C4 C4 C5(20ms) C4 C4 → C5 被并入 C4，只剩一个 C4 事件
#[test]
fn test_short_octave_error_merged() {
    let midis = midis_from(&[(60.0, 10), (72.0, 2), (60.0, 10)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "expected a single C4 event, got {:?}",
        events
    );
    assert_eq!(events[0].0, 60);
    // 事件应覆盖整个音区
    assert!(
        events[0].2 - events[0].1 > 0.18,
        "event too short: {:?}",
        events[0]
    );
}

/// 句首八度错误: C5(30ms) → C4(500ms) → 首部 C5 应被丢弃，主音为 C4
#[test]
fn test_sentence_start_octave_dropped() {
    let midis = midis_from(&[(72.0, 3), (60.0, 50)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "expected a single C4 event, got {:?}",
        events
    );
    assert_eq!(events[0].0, 60, "sentence-start C5 must be dropped");
    assert!(
        events[0].2 - events[0].1 > 0.4,
        "C4 event too short: {:?}",
        events[0]
    );
}

/// 真实倚音: D4(100ms) → E4(400ms) → 两个真实音符都应保留
#[test]
fn test_real_appoggiatura_kept() {
    let midis = midis_from(&[(62.0, 10), (64.0, 40)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        2,
        "real appoggiatura must be kept, got {:?}",
        events
    );
    assert_eq!(events[0].0, 62); // D4
    assert_eq!(events[1].0, 64); // E4
}

/// 半音抖动: C4 内夹两段 C#4(20ms) → 全部吸收进 C4，只产生一个事件
#[test]
fn test_semitone_jitter_absorbed() {
    let midis = midis_from(&[(60.0, 20), (61.0, 2), (60.0, 20), (61.0, 2), (60.0, 20)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "semitone jitter must be absorbed, got {:?}",
        events
    );
    assert_eq!(events[0].0, 60);
}

/// 真正换音: C4(200ms) → G4(200ms) → 两个事件
#[test]
fn test_real_note_change_two_events() {
    let midis = midis_from(&[(60.0, 20), (67.0, 20)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        2,
        "real note change must yield 2 events, got {:?}",
        events
    );
    assert_eq!(events[0].0, 60);
    assert_eq!(events[1].0, 67);
}

/// 极短噪声: C4 中间 1 帧 G5(10ms) → 被并入 C4
#[test]
fn test_very_short_noise_dropped() {
    let midis = midis_from(&[(60.0, 10), (79.0, 1), (60.0, 10)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "10ms noise must be dropped, got {:?}",
        events
    );
    assert_eq!(events[0].0, 60);
}

/// 尾音错误: E4(500ms) → E5(20ms) → 尾部 E5 丢弃，保留 E4
#[test]
fn test_tail_octave_error_dropped() {
    let midis = midis_from(&[(64.0, 50), (76.0, 2)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "tail octave error must be dropped, got {:?}",
        events
    );
    assert_eq!(events[0].0, 64, "expected E4, got {}", events[0].0);
}

/// 空输入不 panic
#[test]
fn test_empty_input() {
    let events = run(&[]);
    assert!(events.is_empty());
}

/// 连续同音长段只产生一个事件
#[test]
fn test_long_stable_note_single_event() {
    let midis = midis_from(&[(60.0, 200)]);
    let events = run(&midis);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, 60);
    assert!(
        (events[0].2 - events[0].1 - 2.0).abs() < 0.1,
        "duration wrong: {:?}",
        events[0]
    );
}
