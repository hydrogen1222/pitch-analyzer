// LyricNoteMatcher v2 (Round4 Commit 9, 任务书 §12)
//
// 输入: PronunciationUnit (pitch-bearing nucleus span) + canonical GAME notes
//       + FCPE F0 track
// 输出: PitchAttachment (many-to-many)
//
// 硬规则 (§12.2): 候选只能来自真实重叠
//   intersection(nucleus span, note span) 超过动态准入门槛;
//   容差 (frame_tolerance) 只用于帧量化/浮点误差, 不是跨音节借音。
//
// 评分 (§12.3) 可解释, 不称 confidence:
//   score = w1*nucleus_overlap_ratio + w2*note_overlap_ratio
//         + w3*FCPE_voiced_coverage + w4*GAME↔FCPE_pitch_agreement
//
// Ambiguity (§12.4): 两个 note 得分接近 → status=Ambiguous, primary=None,
//   全部候选保留在 note_ids。

use crate::models::{
    F0Summary, PitchAttachment, PitchAttachmentStatus, PitchBearingPolicy, PitchTrack,
    PronunciationUnit,
};
use crate::note_engine::MusicalNoteEvent;

/// 匹配参数集中管理 (§12.3: 不散落 magic numbers)
#[derive(Debug, Clone, Copy)]
pub struct LyricNoteMatchParams {
    /// w1: 重叠占 nucleus 的比例
    pub nucleus_overlap_weight: f32,
    /// w2: 重叠占 note 的比例
    pub note_overlap_weight: f32,
    /// w3: 核内 FCPE voiced coverage
    pub voiced_coverage_weight: f32,
    /// w4: GAME↔FCPE 音高一致性
    pub pitch_agreement_weight: f32,
    /// 候选准入: 重叠 >= clamp(0.2*min(unit,note), 10ms, 30ms)
    pub admission_ratio: f32,
    pub admission_floor_secs: f32,
    pub admission_ceiling_secs: f32,
    /// 帧量化容差 (§12.2): 只吸收边界浮点误差
    pub frame_tolerance_secs: f32,
    /// 前两名得分比 >= margin → Ambiguous
    pub ambiguity_margin: f32,
    /// GAME↔FCPE 一致性低于此 → LowCrossModelAgreement
    pub max_agreement_delta_cents: f32,
    pub fcpe_confidence_threshold: f32,
}

impl Default for LyricNoteMatchParams {
    fn default() -> Self {
        Self {
            nucleus_overlap_weight: 0.40,
            note_overlap_weight: 0.25,
            voiced_coverage_weight: 0.20,
            pitch_agreement_weight: 0.15,
            admission_ratio: 0.2,
            admission_floor_secs: 0.010,
            admission_ceiling_secs: 0.030,
            frame_tolerance_secs: 0.015,
            ambiguity_margin: 0.9,
            max_agreement_delta_cents: 180.0,
            fcpe_confidence_threshold: 0.3,
        }
    }
}

/// 一个 unit 的声学输入 (由上层从对齐结果装配)
#[derive(Debug, Clone)]
pub struct UnitAcoustics {
    pub unit_id: usize,
    /// 音节/mora 的声学 span
    pub span: (f32, f32),
    /// pitch-bearing nucleus (母音核); 无 phoneme FA 时 = span 本身
    pub nucleus: (f32, f32),
}

/// 匹配结果行: unit_id → attachment
pub struct MatcherOutput {
    pub attachments: Vec<PitchAttachment>,
}

/// 核心 matcher: many-to-many, 真实重叠准入, 可解释评分。
///
/// `units` 提供文本/发音层语义 (pitch_bearing policy);
/// `acoustics` 与 units 等长 (无对齐的 unit 填 None);
/// `notes` 是 canonical GAME notes (或 legacy events);
/// `track` 提供独立 FCPE 证据。
pub fn match_units_to_notes(
    units: &[PronunciationUnit],
    acoustics: &[Option<UnitAcoustics>],
    notes: &[MusicalNoteEvent],
    track: &PitchTrack,
    params: &LyricNoteMatchParams,
) -> MatcherOutput {
    debug_assert_eq!(units.len(), acoustics.len());
    let mut attachments = Vec::with_capacity(units.len());
    for (unit, ac) in units.iter().zip(acoustics.iter()) {
        attachments.push(match_one(unit, ac.as_ref(), notes, track, params));
    }
    MatcherOutput { attachments }
}

fn match_one(
    unit: &PronunciationUnit,
    acoustics: Option<&UnitAcoustics>,
    notes: &[MusicalNoteEvent],
    track: &PitchTrack,
    params: &LyricNoteMatchParams,
) -> PitchAttachment {
    let unit_id = unit.unit_id_of();
    // 1. 物理无音高单位: 促音/标点等 (不猜)
    if unit.pitch_bearing == PitchBearingPolicy::None {
        return PitchAttachment {
            pronunciation_unit_id: unit_id,
            nucleus_span: None,
            note_ids: Vec::new(),
            primary_note_id: None,
            f0_summary: None,
            status: PitchAttachmentStatus::NonPitchBearing,
        };
    }
    let Some(ac) = acoustics else {
        return PitchAttachment {
            pronunciation_unit_id: unit_id,
            nucleus_span: None,
            note_ids: Vec::new(),
            primary_note_id: None,
            f0_summary: None,
            status: PitchAttachmentStatus::AlignmentMissing,
        };
    };
    let (n0, n1) = ac.nucleus;
    let f0 = f0_summary(track, n0, n1, params.fcpe_confidence_threshold);

    // 2. 核内无 voicing → NoVoicing
    if f0.voiced_coverage <= 0.0 {
        return PitchAttachment {
            pronunciation_unit_id: unit_id,
            nucleus_span: Some((n0, n1)),
            note_ids: Vec::new(),
            primary_note_id: None,
            f0_summary: Some(f0),
            status: PitchAttachmentStatus::NoVoicing,
        };
    }

    // 3. 真实重叠候选 (§12.2)
    let unit_dur = (n1 - n0).max(1e-3);
    let mut candidates: Vec<(usize, f32, f32)> = Vec::new(); // (note_idx, overlap, score)
    for (note_idx, note) in notes.iter().enumerate() {
        let (s0, s1) = (note.start, note.end);
        let overlap = (s1.min(n1) - s0.max(n0) + params.frame_tolerance_secs).max(0.0);
        // 容差只用于把"恰好相邻"的帧量化边界收进来, 不制造跨音节重叠
        if overlap <= params.frame_tolerance_secs {
            continue;
        }
        let note_dur = (s1 - s0).max(1e-3);
        let min_overlap = (params.admission_ratio * unit_dur.min(note_dur))
            .clamp(params.admission_floor_secs, params.admission_ceiling_secs);
        if overlap < min_overlap {
            continue;
        }
        let r_nucleus = (overlap / unit_dur).clamp(0.0, 1.0);
        let r_note = (overlap / note_dur).clamp(0.0, 1.0);
        let intersection = (s1.min(n1) - s0.max(n0)).max(0.0);
        let coverage = f0.voiced_coverage;
        let agreement = pitch_agreement(track, s0.max(n0), s1.min(n1), note.midi_float, params);
        let score = params.nucleus_overlap_weight * r_nucleus
            + params.note_overlap_weight * r_note
            + params.voiced_coverage_weight * coverage
            + params.pitch_agreement_weight * agreement;
        candidates.push((note_idx, overlap, score));
    }
    candidates.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                notes[a.0]
                    .start
                    .partial_cmp(&notes[b.0].start)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    if candidates.is_empty() {
        return PitchAttachment {
            pronunciation_unit_id: unit_id,
            nucleus_span: Some((n0, n1)),
            note_ids: Vec::new(),
            primary_note_id: None,
            f0_summary: Some(f0),
            status: PitchAttachmentStatus::NoOverlappingNote,
        };
    }

    // 4. GAME↔FCPE 一致性门槛: 最优候选也不一致 → 显式标记
    let best = candidates[0];
    let best_note = &notes[best.0];
    let best_agreement = pitch_agreement(
        track,
        best_note.start.max(n0),
        best_note.end.min(n1),
        best_note.midi_float,
        params,
    );
    if best_agreement <= 0.0 {
        return PitchAttachment {
            pronunciation_unit_id: unit_id,
            nucleus_span: Some((n0, n1)),
            note_ids: candidates
                .iter()
                .map(|(idx, _, _)| notes[*idx].id)
                .collect(),
            primary_note_id: None,
            f0_summary: Some(f0),
            status: PitchAttachmentStatus::LowCrossModelAgreement,
        };
    }

    // 5. Ambiguity (§12.4): 第二名得分接近、音名不同、且两者在时间上
    //    并发竞争 (同一区域两个候选) → 不强行挑一个。
    //    顺序覆盖不同时间段的候选是 melisma (1 unit → N notes), 不是歧义;
    //    primary 取得分最高者 (平分取最早)。
    let note_ids: Vec<u32> = candidates
        .iter()
        .map(|(idx, _, _)| notes[*idx].id)
        .collect();
    let (status, primary) = if candidates.len() > 1 {
        let second = candidates[1];
        let second_note = &notes[second.0];
        let concurrent_overlap =
            (best_note.end.min(second_note.end) - best_note.start.max(second_note.start)).max(0.0);
        let competing = concurrent_overlap > params.frame_tolerance_secs;
        if competing
            && best.2 > 0.0
            && second.2 / best.2 >= params.ambiguity_margin
            && best_note.midi_rounded != second_note.midi_rounded
        {
            (PitchAttachmentStatus::Ambiguous, None)
        } else {
            (PitchAttachmentStatus::Reliable, Some(best_note.id))
        }
    } else {
        (PitchAttachmentStatus::Reliable, Some(best_note.id))
    };

    PitchAttachment {
        pronunciation_unit_id: unit_id,
        nucleus_span: Some((n0, n1)),
        note_ids,
        primary_note_id: primary,
        f0_summary: Some(f0),
        status,
    }
}

/// Pipeline 入口 (Commit 9): 对整批歌词计算 v4 attachments。
/// - unit 序列 = 行 mora 序 (投影顺序一致, id = mora 下标)
/// - acoustics: mora 的 DP/FA 时间作为 nucleus 代理 (无 phoneme FA 时
///   这是最诚实的上界; FA 落地后由 phoneme 核替换)
/// - canonical GAME notes 优先, legacy NoteEvent 兼容映射
pub fn attach_lines(
    lines: &mut [crate::models::LyricLine],
    track: &crate::models::PitchTrack,
    confidence_threshold: f32,
) {
    if track.times.is_empty() {
        return;
    }
    let params = LyricNoteMatchParams {
        fcpe_confidence_threshold: confidence_threshold,
        ..Default::default()
    };
    // GAME 交叉验证失败态: canonical 明确拒绝, attachments 不伪造
    let game_rejected = track.musical_note_source == crate::note_engine::MusicalNoteSource::Game
        && track.musical_notes.is_empty()
        && !track.raw_game_notes.is_empty();
    let use_musical = !track.musical_notes.is_empty();
    let use_legacy = track.musical_notes.is_empty() && !track.note_events.is_empty();
    if game_rejected || (!use_musical && !use_legacy) {
        for line in lines {
            line.pitch_attachments.clear();
        }
        return;
    }
    let notes: Vec<MusicalNoteEvent> = if use_musical {
        track.musical_notes.clone()
    } else {
        track
            .note_events
            .iter()
            .enumerate()
            .map(|(id, ev)| MusicalNoteEvent {
                id: id as u32,
                start: ev.start,
                end: ev.end,
                midi_float: ev.center_midi.unwrap_or(ev.midi as f32),
                midi_rounded: ev.midi,
                note_name: ev.note_name.clone(),
                confidence: ev.confidence,
                source: crate::note_engine::MusicalNoteSource::LegacyFcpeTracker,
                model_confidence: Some(ev.confidence),
                boundary_confidence: None,
                is_slur: None,
                evidence: None,
                class: None,
            })
            .collect()
    };

    for line in lines {
        let language = line
            .language
            .unwrap_or(crate::models::SingingLanguage::Unknown);
        let layers = crate::alignment::projection::project_line_v4(line, language);
        let acoustics: Vec<Option<UnitAcoustics>> = line
            .moras
            .iter()
            .enumerate()
            .map(|(unit_id, mora)| match (mora.start_time, mora.end_time) {
                (Some(start), Some(end)) if end > start => Some(UnitAcoustics {
                    unit_id,
                    span: (start, end),
                    nucleus: (start, end),
                }),
                _ => None,
            })
            .collect();
        let output = match_units_to_notes(
            &layers.pronunciation_units,
            &acoustics,
            &notes,
            track,
            &params,
        );
        line.pitch_attachments = output.attachments;
    }
}

trait UnitId {
    fn unit_id_of(&self) -> usize;
}

impl UnitId for PronunciationUnit {
    fn unit_id_of(&self) -> usize {
        self.id
    }
}

/// GAME↔FCPE pitch agreement (可解释证据, 不是概率):
/// 核∩音符窗口内的 FCPE 中位 midi 与 GAME midi 的偏差,
/// 0 cents → 1.0; ≥ max_agreement_delta_cents → 0 (不一致)。
fn pitch_agreement(
    track: &PitchTrack,
    start: f32,
    end: f32,
    game_midi: f32,
    params: &LyricNoteMatchParams,
) -> f32 {
    let Some(median) = median_midi_in(track, start, end, params.fcpe_confidence_threshold) else {
        return 0.0;
    };
    let delta_cents = ((median - game_midi).abs()) * 100.0;
    (1.0 - delta_cents / params.max_agreement_delta_cents).clamp(0.0, 1.0)
}

/// 核内 F0 稳健统计 (§13.2): 不使用"整个 span 简单平均"
pub fn f0_summary(
    track: &PitchTrack,
    start: f32,
    end: f32,
    confidence_threshold: f32,
) -> F0Summary {
    let mut midis: Vec<f32> = Vec::new();
    let mut total_frames = 0usize;
    for (i, &t) in track.times.iter().enumerate() {
        if t < start {
            continue;
        }
        if t >= end {
            break;
        }
        total_frames += 1;
        let midi = track.midis.get(i).copied().unwrap_or(f32::NAN);
        let conf = track.confidences.get(i).copied().unwrap_or(0.0);
        if midi.is_finite() && conf >= confidence_threshold {
            midis.push(midi);
        }
    }
    let voiced_coverage = if total_frames == 0 {
        0.0
    } else {
        midis.len() as f32 / total_frames as f32
    };
    if midis.is_empty() {
        return F0Summary {
            voiced_coverage,
            median_midi: None,
            mad_cents: None,
            p10_midi: None,
            p90_midi: None,
            slope_cents_per_sec: None,
        };
    }
    let mut sorted = midis.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = percentile(&sorted, 0.5);
    let mad = {
        let mut dev: Vec<f32> = midis.iter().map(|m| (m - median).abs() * 100.0).collect();
        dev.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        percentile(&dev, 0.5)
    };
    let p10 = percentile(&sorted, 0.10);
    let p90 = percentile(&sorted, 0.90);
    F0Summary {
        voiced_coverage,
        median_midi: Some(median),
        mad_cents: Some(mad),
        p10_midi: Some(p10),
        p90_midi: Some(p90),
        slope_cents_per_sec: None,
    }
}

fn median_midi_in(track: &PitchTrack, start: f32, end: f32, threshold: f32) -> Option<f32> {
    let summary = f0_summary(track, start, end, threshold);
    summary.median_midi
}

fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    let idx = (q * (sorted.len() - 1) as f32).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_engine::MusicalNoteSource;

    fn note(id: u32, start: f32, end: f32, midi: f32) -> MusicalNoteEvent {
        MusicalNoteEvent {
            id,
            start,
            end,
            midi_float: midi,
            midi_rounded: midi.round() as i32,
            note_name: crate::models::midi_to_note_name(midi),
            confidence: 1.0,
            source: MusicalNoteSource::Game,
            model_confidence: None,
            boundary_confidence: None,
            is_slur: None,
            evidence: None,
            class: None,
        }
    }

    fn track(notes: &[(f32, f32, f32)]) -> PitchTrack {
        // notes: (start, end, midi); 其余帧 NaN
        let end = notes.iter().map(|n| n.1).fold(0.0f32, f32::max);
        let n = ((end + 0.1) / 0.01) as usize;
        let mut track = PitchTrack {
            times: (0..n).map(|i| i as f32 * 0.01).collect(),
            ..Default::default()
        };
        track.frequencies = vec![440.0; n];
        track.confidences = vec![0.9; n];
        track.midis = vec![f32::NAN; n];
        for (s, e, m) in notes {
            for i in 0..n {
                let t = i as f32 * 0.01;
                if t >= *s && t < *e {
                    track.midis[i] = *m;
                }
            }
        }
        track
    }

    fn unit(id: usize, policy: PitchBearingPolicy) -> PronunciationUnit {
        PronunciationUnit {
            id,
            language: crate::models::SingingLanguage::Japanese,
            text: "あ".to_string(),
            normalized: "あ".to_string(),
            kind: crate::models::PronunciationUnitKind::JapaneseMora,
            source_span_ids: Vec::new(),
            phoneme_ids: Vec::new(),
            pitch_bearing: policy,
        }
    }

    fn acoustics(unit_id: usize, span: (f32, f32)) -> Option<UnitAcoustics> {
        Some(UnitAcoustics {
            unit_id,
            span,
            nucleus: span,
        })
    }

    fn default_track() -> PitchTrack {
        track(&[(0.0, 2.0, 60.0)])
    }

    /// §22: exact overlap → Reliable
    #[test]
    fn exact_overlap_is_reliable() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 1.5))];
        let notes = vec![note(10, 1.0, 1.5, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert_eq!(out.attachments[0].status, PitchAttachmentStatus::Reliable);
        assert_eq!(out.attachments[0].note_ids, vec![10]);
        assert_eq!(out.attachments[0].primary_note_id, Some(10));
    }

    /// §22 / §12.2: zero overlap → 不准入 (不借音)
    #[test]
    fn zero_overlap_neighbor_not_claimed() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 1.5))];
        let notes = vec![note(10, 1.5, 2.0, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert_eq!(
            out.attachments[0].status,
            PitchAttachmentStatus::NoOverlappingNote
        );
        assert!(out.attachments[0].note_ids.is_empty());
    }

    /// §22: 5ms rounding tolerance → 仍可准入 (帧量化误差)
    #[test]
    fn tiny_frame_quantization_overlap_admitted() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        // 音符比核晚 5ms 开始 (一帧)
        let ac = vec![acoustics(0, (1.0, 1.5))];
        let notes = vec![note(10, 1.005, 2.0, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert_eq!(out.attachments[0].status, PitchAttachmentStatus::Reliable);
        assert_eq!(out.attachments[0].note_ids, vec![10]);
    }

    /// §22: 1 unit → 3 notes (melisma), primary = 重叠最大者
    #[test]
    fn melisma_one_unit_many_notes() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 1.6))];
        let notes = vec![
            note(1, 1.0, 1.2, 60.0),
            note(2, 1.2, 1.4, 62.0),
            note(3, 1.4, 1.6, 64.0),
        ];
        let mut melodic = track(&[(1.0, 1.2, 60.0), (1.2, 1.4, 62.0), (1.4, 1.6, 64.0)]);
        melodic.times = (0..melodic.times.len()).map(|i| i as f32 * 0.01).collect();
        let out = match_units_to_notes(&units, &ac, &notes, &melodic, &Default::default());
        let attachment = &out.attachments[0];
        assert_eq!(attachment.note_ids, vec![1, 2, 3]);
        assert_eq!(attachment.status, PitchAttachmentStatus::Reliable);
        assert_eq!(attachment.primary_note_id, Some(1));
    }

    /// §22: 2 units → 1 note (连唱共享)
    #[test]
    fn two_units_share_one_note() {
        let units = vec![
            unit(0, PitchBearingPolicy::Nucleus),
            unit(1, PitchBearingPolicy::Nucleus),
        ];
        let ac = vec![acoustics(0, (1.0, 1.3)), acoustics(1, (1.3, 1.6))];
        let notes = vec![note(10, 1.0, 1.6, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert!(out.attachments.iter().all(|a| a.note_ids == vec![10]));
        assert!(out
            .attachments
            .iter()
            .all(|a| a.status == PitchAttachmentStatus::Reliable));
    }

    /// §12.4: 同一时间区域的竞争候选得分接近 → Ambiguous + primary=None
    #[test]
    fn equal_scores_are_ambiguous() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 2.0))];
        // 两个候选同时覆盖核的中段 (GAME 边界不确定的典型形态),
        // FCPE 中位介于两者之间 → 分数接近 → 歧义, 不强行挑一个
        let notes = vec![note(1, 1.0, 1.6, 60.0), note(2, 1.4, 2.0, 62.0)];
        let contested = track(&[(1.0, 2.0, 61.0)]);
        let out = match_units_to_notes(&units, &ac, &notes, &contested, &Default::default());
        let attachment = &out.attachments[0];
        assert_eq!(attachment.status, PitchAttachmentStatus::Ambiguous);
        assert_eq!(attachment.primary_note_id, None);
        assert_eq!(attachment.note_ids.len(), 2);
    }

    /// §22: closure (っ) → NonPitchBearing, 即使音符完全重叠也不给 pitch
    #[test]
    fn non_pitch_bearing_unit_never_gets_note() {
        let units = vec![unit(0, PitchBearingPolicy::None)];
        let ac = vec![acoustics(0, (1.0, 1.5))];
        let notes = vec![note(10, 1.0, 1.5, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert_eq!(
            out.attachments[0].status,
            PitchAttachmentStatus::NonPitchBearing
        );
        assert!(out.attachments[0].note_ids.is_empty());
        assert!(out.attachments[0].f0_summary.is_none());
    }

    /// 无 voicing → NoVoicing
    #[test]
    fn unvoiced_nucleus_is_no_voicing() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 1.5))];
        let silent = track(&[]);
        let notes = vec![note(10, 1.0, 1.5, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &silent, &Default::default());
        assert_eq!(out.attachments[0].status, PitchAttachmentStatus::NoVoicing);
    }

    /// GAME↔FCPE 大偏差 → LowCrossModelAgreement
    #[test]
    fn cross_model_disagreement_is_explicit() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![acoustics(0, (1.0, 1.5))];
        // GAME 说 C4 (60), FCPE 实测 E4 (64) → 400 cents
        let notes = vec![note(10, 1.0, 1.5, 60.0)];
        let mismatched = track(&[(1.0, 1.5, 64.0)]);
        let out = match_units_to_notes(&units, &ac, &notes, &mismatched, &Default::default());
        assert_eq!(
            out.attachments[0].status,
            PitchAttachmentStatus::LowCrossModelAgreement
        );
        assert_eq!(out.attachments[0].primary_note_id, None);
    }

    /// 无对齐 → AlignmentMissing
    #[test]
    fn missing_alignment_is_explicit() {
        let units = vec![unit(0, PitchBearingPolicy::Nucleus)];
        let ac = vec![None];
        let notes = vec![note(10, 1.0, 1.5, 60.0)];
        let out = match_units_to_notes(&units, &ac, &notes, &default_track(), &Default::default());
        assert_eq!(
            out.attachments[0].status,
            PitchAttachmentStatus::AlignmentMissing
        );
    }
}
