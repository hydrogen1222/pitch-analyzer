// NoteTracker v2 合成回归测试 (任务书 §3.4 / §10.1)
//
// 全部输入为程序生成的连续 MIDI 轮廓 (无版权):
//   synth helpers: flat / vibrato / glide / noise / unvoiced gap
//
// 基线: v1 (round-first 分段) 在 vibrato/glide/jitter/gap 场景过分割;
//       v2 (stable plateau + cents hysteresis) 必须全部通过。

use pitch_analyzer_tauri_lib::models::NoteTrackingParams;
use pitch_analyzer_tauri_lib::note_tracker::build_note_events;

const HOP: f32 = 0.01;

// ── 合成器 ─────────────────────────────────────────────────

fn flat(midi: f32, dur_s: f32) -> Vec<f32> {
    vec![midi; (dur_s / HOP).round() as usize]
}

/// 确定性噪声 (LCG), ±cents 范围
fn add_noise(v: Vec<f32>, cents: f32, seed: u64) -> Vec<f32> {
    let mut state = seed | 1;
    v.into_iter()
        .map(|x| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let r = ((state >> 33) as i64 as f32 / (1i64 << 31) as f32 - 1.0) * cents;
            x + r / 100.0
        })
        .collect()
}

/// 围绕中心的正弦颤音 (depth 为峰峰值的一半, cents)
fn vibrato(center: f32, depth_cents: f32, rate_hz: f32, dur_s: f32) -> Vec<f32> {
    let n = (dur_s / HOP).round() as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 * HOP;
            center + (depth_cents / 100.0) * (2.0 * std::f32::consts::PI * rate_hz * t).sin()
        })
        .collect()
}

/// 线性滑音
fn glide(from: f32, to: f32, dur_s: f32) -> Vec<f32> {
    let n = (dur_s / HOP).round() as usize;
    (0..n)
        .map(|i| from + (to - from) * (i as f32 / (n - 1).max(1) as f32))
        .collect()
}

/// 插入无声 (NaN) 间隙
fn unvoiced_gap(v: Vec<f32>, dur_s: f32) -> Vec<f32> {
    let mut out = v;
    out.extend(vec![f32::NAN; (dur_s / HOP).round() as usize]);
    out
}

// ── 运行器 ─────────────────────────────────────────────────

fn run(midis: &[f32]) -> Vec<(i32, f32, f32)> {
    let n = midis.len();
    let times: Vec<f32> = (0..n).map(|i| i as f32 * HOP).collect();
    let conf: Vec<f32> = vec![0.8; n];
    let params = NoteTrackingParams::default();
    build_note_events(&times, midis, &conf, &params)
        .into_iter()
        .map(|ev| (ev.midi, ev.start, ev.end))
        .collect()
}

fn seq(parts: Vec<Vec<f32>>) -> Vec<f32> {
    parts.concat()
}

// ── Test 1: 纯稳音 ──────────────────────────────────────────

#[test]
fn v2_test1_stable_note() {
    let midis = add_noise(flat(69.0, 1.0), 10.0, 1);
    let events = run(&midis);
    assert_eq!(events.len(), 1, "stable note must be 1 event: {:?}", events);
    assert_eq!(events[0].0, 69, "A4");
}

// ── Test 2: 跨 round 边界的颤音 (v1 的致命场景) ──────────────

#[test]
fn v2_test2_vibrato_crossing_round_boundary() {
    let midis = vibrato(69.0, 55.0, 6.0, 1.0);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "vibrato must be ONE event, got {:?}",
        events.iter().map(|e| e.0).collect::<Vec<_>>()
    );
    assert_eq!(events[0].0, 69);
}

// ── Test 3: 稳定音贴近半音边界抖动 ───────────────────────────

#[test]
fn v2_test3_jitter_near_semitone_boundary() {
    let midis = seq(vec![
        flat(69.45, 0.25),
        flat(69.55, 0.25),
        flat(69.45, 0.25),
        flat(69.55, 0.25),
    ]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "boundary jitter must not flicker, got {:?}",
        events.iter().map(|e| e.0).collect::<Vec<_>>()
    );
    assert!(
        events[0].0 == 69 || events[0].0 == 70,
        "center should resolve near the boundary, got {}",
        events[0].0
    );
}

// ── Test 4: glide 不得产生中间半音 ───────────────────────────

#[test]
fn v2_test4_glide_no_intermediate_notes() {
    let midis = seq(vec![
        flat(69.0, 0.3),
        glide(69.0, 71.0, 0.12),
        flat(71.0, 0.3),
    ]);
    let events = run(&midis);
    let midis_out: Vec<i32> = events.iter().map(|e| e.0).collect();
    assert_eq!(
        midis_out,
        vec![69, 71],
        "glide must yield [A4, B4] without A#4, got {:?}",
        events
    );
}

// ── Test 5: 真 melisma 必须保留 ──────────────────────────────

#[test]
fn v2_test5_true_melisma_kept() {
    let midis = seq(vec![flat(69.0, 0.15), flat(71.0, 0.15), flat(69.0, 0.18)]);
    let events = run(&midis);
    let midis_out: Vec<i32> = events.iter().map(|e| e.0).collect();
    assert_eq!(
        midis_out,
        vec![69, 71, 69],
        "true melisma must be preserved, got {:?}",
        events
    );
}

// ── Test 6: 短倚音 (45~70ms, 有证据) 保留 ────────────────────

#[test]
fn v2_test6_short_appoggiatura_kept() {
    let midis = seq(vec![flat(67.0, 0.06), flat(69.0, 0.4)]);
    let events = run(&midis);
    let midis_out: Vec<i32> = events.iter().map(|e| e.0).collect();
    assert_eq!(
        midis_out,
        vec![67, 69],
        "short appoggiatura with evidence must be kept, got {:?}",
        events
    );
}

// ── Test 7: 假 flicker (20~30ms excursion) 不成事件 ──────────

#[test]
fn v2_test7_flicker_not_an_event() {
    let midis = seq(vec![flat(69.0, 0.3), flat(70.0, 0.025), flat(69.0, 0.3)]);
    let events = run(&midis);
    assert_eq!(events.len(), 1, "flicker must be absorbed: {:?}", events);
    assert_eq!(events[0].0, 69);
}

// ── Test 8: 八度 glitch 不成事件 ─────────────────────────────

#[test]
fn v2_test8_octave_glitch_not_an_event() {
    let midis = seq(vec![flat(69.0, 0.3), flat(81.0, 0.03), flat(69.0, 0.3)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "octave glitch must be absorbed: {:?}",
        events
    );
    assert_eq!(events[0].0, 69);
}

// ── Test 9: 短静音桥接 (两侧同 target) ───────────────────────

#[test]
fn v2_test9_micro_gap_bridged() {
    let midis = unvoiced_gap(seq(vec![flat(69.0, 0.3)]), 0.02);
    let midis = seq(vec![midis, flat(69.0, 0.3)]);
    let events = run(&midis);
    assert_eq!(
        events.len(),
        1,
        "20ms gap with same target on both sides must bridge: {:?}",
        events
    );
}

// ── Test 10: v2 字段存在性 ───────────────────────────────────

#[test]
fn v2_test10_tracker_version_and_center() {
    let midis = flat(69.0, 0.5);
    let n = midis.len();
    let times: Vec<f32> = (0..n).map(|i| i as f32 * HOP).collect();
    let conf: Vec<f32> = vec![0.8; n];
    let params = NoteTrackingParams::default();
    let events = build_note_events(&times, &midis, &conf, &params);
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.tracker_version, 2, "must be v2");
    assert!(
        ev.center_midi
            .map(|c| (c - 69.0).abs() < 0.3)
            .unwrap_or(false),
        "center_midi must be near 69, got {:?}",
        ev.center_midi
    );
}
