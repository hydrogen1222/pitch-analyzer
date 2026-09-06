// Round4 Commit 4: Remove False Precision 回归测试
//
// 任务书 P0-5 / P0-6 / P0-7 / §15.1:
//   1. 200ms phonetic grace (跨 group 借音) 已删除
//   2. MoraDP 0.7/0.3 伪置信度已删除 (0.0 = 无校准概率)
//   3. FA 未覆盖 token 不再获得整行时间
//   4. WeightedFallback 时间 = VisualOnly, 不参与正式 pitch attribution

use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, bind_pitch_to_tokens, parse_lrc_with_language, TokenAlignParams,
};
use pitch_analyzer_tauri_lib::models::{
    AlignmentSource, NoteTrackingParams, PitchTrack, UnpitchedReason,
};
use pitch_analyzer_tauri_lib::note_engine::{MusicalNoteEvent, MusicalNoteSource};

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

fn vo_track(start: f32, end: f32) -> PitchTrack {
    // 1.00-2.00 全程 voiced @ C4, 10ms hop
    let n = ((end - start) / 0.01) as usize;
    PitchTrack {
        times: (0..n).map(|i| start + i as f32 * 0.01).collect(),
        frequencies: vec![261.63; n],
        confidences: vec![0.9; n],
        midis: vec![60.0; n],
        rms: vec![0.1; n],
        flux: Vec::new(),
        ..Default::default()
    }
}

/// P0-5: MoraDP 不再输出伪置信度
#[test]
fn mora_dp_no_longer_reports_fake_confidence() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみと\n[00:03.00]歩くよ",
        Some(5.0),
        pitch_analyzer_tauri_lib::models::ProjectLanguage::Japanese,
    );
    let track = vo_track(1.0, 4.0);
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    let line = &lines[0];
    assert!(!line.moras.is_empty(), "Japanese line must have moras");
    assert!(
        line.moras.iter().all(|m| m.start_time.is_some()),
        "DP must align moras for this fixture"
    );
    assert!(
        line.moras.iter().all(|m| m.confidence == 0.0),
        "MoraDP is a heuristic: mora confidence must be 0.0 (no calibrated probability), got {:?}",
        line.moras.iter().map(|m| m.confidence).collect::<Vec<_>>()
    );
    assert!(line.tokens.iter().all(|t| t.alignment_confidence == 0.0));
    assert!(line
        .tokens
        .iter()
        .all(|t| t.alignment_source == Some(AlignmentSource::MoraDp)));
}

/// §15.1: WeightedFallback 时间是 VisualOnly, 不产生正式 pitch badge
#[test]
fn weighted_fallback_timing_is_visual_only() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみと\n[00:03.00]歩くよ",
        Some(5.0),
        pitch_analyzer_tauri_lib::models::ProjectLanguage::Japanese,
    );
    let track = vo_track(1.0, 4.0);
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    // 强制进入 WeightedFallback 路径: 生产流程中 DP 失败 → 均匀分配,
    // 此时 display group 不会有时间; token 来源统一改写并清空 group 时间
    // 以模拟该真实状态 (排除 DP 残留的声学 group 时间)。
    for token in &mut lines[0].tokens {
        token.alignment_source = Some(AlignmentSource::WeightedFallback);
    }
    for group in &mut lines[0].reading_display_groups {
        group.start_time = None;
        group.end_time = None;
    }
    bind_pitch_to_tokens(
        &mut lines,
        &track,
        0.3,
        &NoteTrackingParams::default(),
    );
    for token in &lines[0].tokens {
        assert!(
            token.pitch_notes.is_empty() && token.primary_note.is_none(),
            "VisualOnly timing must not receive official pitch badges"
        );
        assert_eq!(token.unpitched_reason, Some(UnpitchedReason::VisualTimingOnly));
    }
}

/// 正常 MoraDp (声学启发式) 时间仍然参与绑定, 不受 VisualOnly 误伤
#[test]
fn mora_dp_timing_still_binds_pitch() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみと\n[00:03.00]歩くよ",
        Some(5.0),
        pitch_analyzer_tauri_lib::models::ProjectLanguage::Japanese,
    );
    let mut track = vo_track(1.0, 4.0);
    track.musical_notes = vec![game_event(0, 1.0, 2.0, 60.0)];
    track.musical_note_source = MusicalNoteSource::Game;
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    // 第一行 きみと 完全落在 1.0-2.0 的音符内 → 至少部分 token 有 pitch
    let bound = lines[0]
        .tokens
        .iter()
        .filter(|t| !t.pitch_notes.is_empty())
        .count();
    assert!(bound > 0, "MoraDP (heuristic but acoustic) must still bind pitch");
}

/// P0-6: FA 结果中缺失的 token 不再被赋整行时间 (用 mora 级 FA 单元测试:
/// apply_forced_alignment 只在 mora 全覆盖时接受; 这里验证绑定层对
/// 无时间 token 给出 AlignmentMissing 而不是整行窗口)
#[test]
fn untimed_tokens_report_alignment_missing_not_whole_line() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみと\n[00:03.00]歩くよ",
        Some(5.0),
        pitch_analyzer_tauri_lib::models::ProjectLanguage::Japanese,
    );
    let track = vo_track(1.0, 4.0);
    bind_pitch_to_tokens(
        &mut lines,
        &track,
        0.3,
        &NoteTrackingParams::default(),
    );
    // 未对齐 (parse 后无时间) 的 token: 必须保持无时间且明确 AlignmentMissing
    for token in &lines[0].tokens {
        assert!(token.start_time.is_none() && token.end_time.is_none());
        assert_eq!(token.unpitched_reason, Some(UnpitchedReason::AlignmentMissing));
        assert!(token.pitch_notes.is_empty());
    }
}

/// zero-overlap 邻居不得借音 (P0-7 的另一个面: 20ms 数值容差只能吸收
/// 帧量化误差, 不允许跨音节借音) — 由 round3_2 的
/// explicit_ruby_mora_without_real_overlap_gets_no_pitch 覆盖 group 级;
/// 这里覆盖 token 级
#[test]
fn token_zero_overlap_neighbor_not_claimed() {
    let mut lines = parse_lrc_with_language(
        "[00:01.00]きみと\n[00:03.00]歩くよ",
        Some(5.0),
        pitch_analyzer_tauri_lib::models::ProjectLanguage::Japanese,
    );
    let mut track = vo_track(1.0, 4.0);
    // 音符 1.0-1.5; 第二个 token (预计 1.34-1.67 起点) 只能获得真实重叠部分
    track.musical_notes = vec![game_event(0, 1.0, 1.5, 60.0)];
    track.musical_note_source = MusicalNoteSource::Game;
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let last_token = lines[0].tokens.last().unwrap();
    if let (Some(ts), Some(_te)) = (last_token.start_time, last_token.end_time) {
        if ts >= 1.5 {
            assert!(
                last_token.pitch_notes.is_empty(),
                "token beyond the note window must not claim it"
            );
        }
    }
}
