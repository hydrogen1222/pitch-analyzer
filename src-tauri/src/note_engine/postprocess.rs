//! Evidence fusion and conservative canonical note decisions.
//!
//! GAME supplies discrete regions; FCPE supplies an independent continuous
//! F0 observation. This module is deliberately independent of lyric timing so
//! it can be tested with synthetic tracks and reused by the production path.

use crate::models::{CanonicalSungNote, NoteEvidence, PitchTrack, SungNoteClass};
use crate::note_engine::{MusicalNoteEvent, MusicalNoteSource};

#[derive(Debug, Clone, Copy)]
pub struct CanonicalNotePostProcessor {
    pub fcpe_confidence_threshold: f32,
    pub minimum_support: f32,
    pub maximum_pitch_delta_cents: f32,
}

impl Default for CanonicalNotePostProcessor {
    fn default() -> Self {
        Self {
            fcpe_confidence_threshold: 0.30,
            minimum_support: 0.42,
            maximum_pitch_delta_cents: 180.0,
        }
    }
}

impl CanonicalNotePostProcessor {
    pub fn new(fcpe_confidence_threshold: f32) -> Self {
        Self {
            fcpe_confidence_threshold,
            ..Self::default()
        }
    }

    /// Build evidence-backed notes. Uncertain candidates are retained in the
    /// result for debug explanation but must not be used as production notes.
    pub fn process(
        &self,
        game_events: &[MusicalNoteEvent],
        track: &PitchTrack,
    ) -> Vec<CanonicalSungNote> {
        let mut notes: Vec<CanonicalSungNote> = game_events
            .iter()
            .filter(|event| event.source == MusicalNoteSource::Game)
            .map(|event| self.make_candidate(event, track))
            .collect();
        notes.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.end
                        .partial_cmp(&b.end)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        self.consolidate(notes)
    }

    /// Convert accepted canonical notes into the legacy-compatible event
    /// shape consumed by the binder. The event's confidence is now a real
    /// FCPE-backed decision, never a hard-coded GAME constant.
    pub fn accepted_events(&self, notes: &[CanonicalSungNote]) -> Vec<MusicalNoteEvent> {
        notes
            .iter()
            .filter(|note| {
                note.class != SungNoteClass::Uncertain && note.confidence >= self.minimum_support
            })
            .enumerate()
            .map(|(id, note)| MusicalNoteEvent {
                id: id as u32,
                start: note.start,
                end: note.end,
                midi_float: note.display_midi,
                midi_rounded: note.display_midi.round() as i32,
                note_name: crate::models::midi_to_note_name(note.display_midi),
                confidence: note.confidence,
                source: MusicalNoteSource::Game,
                model_confidence: None,
                boundary_confidence: None,
                is_slur: None,
                evidence: Some(note.evidence.clone()),
                class: Some(note.class),
            })
            .collect()
    }

    fn make_candidate(&self, event: &MusicalNoteEvent, track: &PitchTrack) -> CanonicalSungNote {
        let evidence = self.measure(event, track);
        let delta = evidence.pitch_delta_cents.unwrap_or(f32::INFINITY).abs();
        let class = if evidence.fcpe_frame_count == 0
            || evidence.voiced_coverage < 0.20
            || delta > self.maximum_pitch_delta_cents
            || evidence.support_score < self.minimum_support
        {
            SungNoteClass::Uncertain
        } else if event.duration() < 0.10 {
            SungNoteClass::Ornament
        } else {
            SungNoteClass::Stable
        };
        let display_midi = if class == SungNoteClass::Uncertain {
            event.midi_float
        } else {
            evidence.median_midi.unwrap_or(event.midi_float)
        };
        CanonicalSungNote {
            event_ids: vec![event.id],
            start: event.start,
            end: event.end,
            game_midi: event.midi_float,
            fcpe_median_midi: evidence.median_midi,
            display_midi,
            voiced_coverage: evidence.voiced_coverage,
            fcpe_support: evidence.support_score,
            confidence: evidence.support_score,
            class,
            evidence,
        }
    }

    fn measure(&self, event: &MusicalNoteEvent, track: &PitchTrack) -> NoteEvidence {
        let duration = event.duration().max(0.001);
        let hop = infer_hop(&track.times);
        let mut values = Vec::new();
        let mut voiced_seconds = 0.0f32;
        for (i, &time) in track.times.iter().enumerate() {
            if time < event.start {
                continue;
            }
            if time >= event.end {
                break;
            }
            let midi = track.midis.get(i).copied().unwrap_or(f32::NAN);
            let confidence = track.confidences.get(i).copied().unwrap_or(0.0);
            if midi.is_finite() && confidence >= self.fcpe_confidence_threshold {
                values.push((midi, confidence));
                voiced_seconds += hop.min((event.end - time).max(0.0));
            }
        }
        let frame_count = values.len();
        let coverage = (voiced_seconds / duration).clamp(0.0, 1.0);
        let median_midi = if values.is_empty() {
            None
        } else {
            Some(weighted_median(&values))
        };
        let mad_cents = median_midi.map(|median| {
            let deviations: Vec<f32> = values
                .iter()
                .map(|(midi, _)| (midi - median).abs() * 100.0)
                .collect();
            median_of(&deviations)
        });
        let delta_cents = median_midi.map(|median| (median - event.midi_float) * 100.0);
        let coverage_score = (coverage / 0.65).clamp(0.0, 1.0);
        let pitch_support = delta_cents
            .map(|delta| (1.0 - delta.abs() / 240.0).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let stability_support = mad_cents
            .map(|mad| (1.0 - mad / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        // Multiplicative support intentionally makes missing evidence visible;
        // a short, high-confidence-looking GAME region cannot pass by itself.
        let support = (coverage_score * pitch_support * stability_support).sqrt();
        NoteEvidence {
            fcpe_frame_count: frame_count,
            voiced_coverage: coverage,
            median_midi,
            midi_mad_cents: mad_cents,
            pitch_delta_cents: delta_cents,
            support_score: support,
        }
    }

    fn consolidate(&self, mut notes: Vec<CanonicalSungNote>) -> Vec<CanonicalSungNote> {
        if notes.len() < 2 {
            return notes;
        }
        let durations: Vec<f32> = notes.iter().map(|n| (n.end - n.start).max(0.0)).collect();
        let typical = median_of(&durations).max(0.04);
        let merge_gap = (typical * 0.18).clamp(0.015, 0.06);
        let mut output: Vec<CanonicalSungNote> = Vec::new();
        let mut current = notes.remove(0);
        for next in notes {
            let same_note = current.display_midi.round() == next.display_midi.round();
            let vibrato_fragment = (current.display_midi - next.display_midi).abs() <= 0.70
                && current.end + merge_gap >= next.start
                && (current.end - current.start) <= typical * 1.15
                && (next.end - next.start) <= typical * 1.15;
            if current.end + merge_gap >= next.start && (same_note || vibrato_fragment) {
                merge_into(&mut current, next);
            } else {
                current.class = classify_transition(current.class, current.display_midi, &output);
                output.push(current);
                current = next;
            }
        }
        current.class = classify_transition(current.class, current.display_midi, &output);
        output.push(current);
        output
    }
}

fn classify_transition(
    class: SungNoteClass,
    current_midi: f32,
    previous: &[CanonicalSungNote],
) -> SungNoteClass {
    if class == SungNoteClass::Stable
        && previous.last().is_some_and(|previous| {
            previous.class != SungNoteClass::Uncertain
                && (previous.display_midi.round() - current_midi.round()).abs() >= 1.0
        })
    {
        SungNoteClass::Transition
    } else {
        class
    }
}

fn merge_into(left: &mut CanonicalSungNote, right: CanonicalSungNote) {
    let left_duration = (left.end - left.start).max(0.001);
    let right_duration = (right.end - right.start).max(0.001);
    let total = left_duration + right_duration;
    left.end = left.end.max(right.end);
    left.event_ids.extend(right.event_ids);
    left.game_midi = (left.game_midi * left_duration + right.game_midi * right_duration) / total;
    left.display_midi =
        (left.display_midi * left_duration + right.display_midi * right_duration) / total;
    left.fcpe_median_midi = match (left.fcpe_median_midi, right.fcpe_median_midi) {
        (Some(a), Some(b)) => Some((a * left_duration + b * right_duration) / total),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    left.voiced_coverage =
        (left.voiced_coverage * left_duration + right.voiced_coverage * right_duration) / total;
    left.fcpe_support =
        (left.fcpe_support * left_duration + right.fcpe_support * right_duration) / total;
    left.confidence = (left.confidence * left_duration + right.confidence * right_duration) / total;
    left.class =
        if left.class == SungNoteClass::Uncertain || right.class == SungNoteClass::Uncertain {
            SungNoteClass::Uncertain
        } else {
            SungNoteClass::Stable
        };
    left.evidence.fcpe_frame_count += right.evidence.fcpe_frame_count;
    left.evidence.voiced_coverage = left.voiced_coverage;
    left.evidence.median_midi = left.fcpe_median_midi;
    left.evidence.midi_mad_cents =
        match (left.evidence.midi_mad_cents, right.evidence.midi_mad_cents) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
    left.evidence.pitch_delta_cents = left.fcpe_median_midi.map(|m| (m - left.game_midi) * 100.0);
    left.evidence.support_score = left.fcpe_support;
}

fn infer_hop(times: &[f32]) -> f32 {
    let mut diffs: Vec<f32> = times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| d.is_finite() && *d > 0.0)
        .collect();
    if diffs.is_empty() {
        return 0.01;
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    diffs[diffs.len() / 2]
}

fn weighted_median(values: &[(f32, f32)]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let total: f32 = sorted.iter().map(|(_, weight)| weight.max(0.001)).sum();
    let mut cumulative = 0.0;
    for (value, weight) in sorted {
        cumulative += weight.max(0.001);
        if cumulative >= total * 0.5 {
            return value;
        }
    }
    values.last().map(|(value, _)| *value).unwrap_or(0.0)
}

fn median_of(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) * 0.5
    } else {
        sorted[mid]
    }
}
