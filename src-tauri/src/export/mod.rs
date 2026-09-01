pub mod ass;
pub mod srt;

use crate::lyrics::select_primary_note;
use crate::models::{LyricLine, LyricToken, PitchNote};

const DISPLAY_NOTE_MIN_DURATION: f32 = 0.045;
const DISPLAY_NOTE_MIN_CONFIDENCE: f32 = 0.3;

/// A source-line presentation unit. It is one lyric/display group, not one
/// subtitle cue: several acoustic notes in the same group become one chain
/// such as `D4-C4`.
#[derive(Debug, Clone)]
pub(crate) struct PresentationSegment {
    pub token_indices: Vec<usize>,
    pub start_time: f32,
    pub end_time: f32,
    pub note_text: Option<String>,
    /// Text directly under the pitch badge. For explicit ruby this is one
    /// kana mora; otherwise it is the source display text.
    pub display_text: String,
    pub phonetic: bool,
}

/// Build the visual units used by both SRT and ASS exporters.
///
/// ReadingDisplayGroup is preferred because it is the canonical display
/// boundary for cases such as `二人 -> ふたり`. For old projects without
/// groups, the token itself remains the compatibility boundary.
pub(crate) fn presentation_segments(line: &LyricLine) -> Vec<PresentationSegment> {
    let timed_indices: Vec<usize> = line
        .tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            token
                .start_time
                .zip(token.end_time)
                .filter(|(start, end)| start.is_finite() && end.is_finite() && end >= start)
                .map(|_| index)
        })
        .collect();
    let mut segments = Vec::new();
    let mut used = std::collections::HashSet::new();
    let phonetic_layout = line
        .reading_display_groups
        .iter()
        .any(|group| group.phonetic);

    for group in &line.reading_display_groups {
        let token_indices: Vec<usize> = timed_indices
            .iter()
            .copied()
            .filter(|&index| {
                let token = &line.tokens[index];
                token.char_start < group.char_end && group.char_start < token.char_end
            })
            .collect();
        if token_indices.is_empty() {
            continue;
        }
        used.extend(token_indices.iter().copied());

        let (token_start, token_end) = token_time_bounds(line, &token_indices)
            .expect("timed presentation group must have timed tokens");
        let start_time = group
            .start_time
            .filter(|time| time.is_finite())
            .unwrap_or(token_start);
        let end_time = group
            .end_time
            .filter(|time| time.is_finite())
            .unwrap_or(token_end);
        let note_text = if !group.pitch_notes.is_empty() || group.primary_note.is_some() {
            format_note_chain(&group.pitch_notes, group.primary_note.as_ref())
        } else {
            first_token_note_chain(line, &token_indices)
        };
        segments.push(PresentationSegment {
            token_indices,
            start_time,
            end_time: end_time.max(start_time),
            note_text,
            display_text: if phonetic_layout {
                if group.reading.is_empty() {
                    group.surface.clone()
                } else {
                    group.reading.clone()
                }
            } else {
                group.surface.clone()
            },
            phonetic: phonetic_layout,
        });
    }

    // Include compatibility tokens and tokens not covered by a display group.
    for index in timed_indices {
        if used.contains(&index) {
            continue;
        }
        let token = &line.tokens[index];
        let (start_time, end_time) = token
            .start_time
            .zip(token.end_time)
            .expect("timed token index must have timing");
        segments.push(PresentationSegment {
            token_indices: vec![index],
            start_time,
            end_time: end_time.max(start_time),
            note_text: format_note_chain(&token.pitch_notes, token.primary_note.as_ref()),
            display_text: visible_token_text(token).to_string(),
            phonetic: false,
        });
    }

    segments.sort_by_key(|segment| segment.token_indices[0]);
    segments
}

/// Kana reading row used by SRT/ASS when explicit `漢字(かな)` is present.
pub(crate) fn phonetic_line_text(
    line: &LyricLine,
    segments: &[PresentationSegment],
) -> Option<String> {
    line.reading_display_groups
        .iter()
        .any(|group| group.phonetic)
        .then(|| {
            segments
                .iter()
                .map(|segment| segment.display_text.as_str())
                .collect::<String>()
        })
}

/// Return the source text without translation columns. This keeps the output
/// as one source lyric line even when the input contains `|` translations.
pub(crate) fn source_line_text(line: &LyricLine) -> String {
    if !line.primary_text.is_empty() {
        return line.primary_text.clone();
    }
    line.tokens
        .iter()
        .map(visible_token_text)
        .collect::<String>()
}

pub(crate) fn format_note_chain(
    notes: &[PitchNote],
    primary: Option<&PitchNote>,
) -> Option<String> {
    // Remove short/low-confidence candidates from the export chain. This
    // preserves the existing primary-note safety rule while keeping genuine
    // multi-note turns visible.
    let significant: Vec<&PitchNote> = notes
        .iter()
        .filter(|note| {
            note.end_time - note.start_time >= DISPLAY_NOTE_MIN_DURATION
                && note.confidence_mean >= DISPLAY_NOTE_MIN_CONFIDENCE
        })
        .collect();

    let mut names = Vec::new();
    if significant.is_empty() {
        if let Some(note) = primary.cloned().or_else(|| {
            select_primary_note(
                notes,
                DISPLAY_NOTE_MIN_DURATION,
                DISPLAY_NOTE_MIN_CONFIDENCE,
            )
        }) {
            append_note_name(&mut names, &note);
        }
    } else {
        for note in significant {
            append_note_name(&mut names, note);
        }
    }
    (!names.is_empty()).then(|| names.join("-"))
}

fn append_note_name(names: &mut Vec<String>, note: &PitchNote) {
    let name = crate::models::midi_to_note_name(note.median_midi);
    if name != "---" && names.last() != Some(&name) {
        names.push(name);
    }
}

fn first_token_note_chain(line: &LyricLine, token_indices: &[usize]) -> Option<String> {
    token_indices.iter().find_map(|&index| {
        let token = &line.tokens[index];
        format_note_chain(&token.pitch_notes, token.primary_note.as_ref())
    })
}

fn token_time_bounds(line: &LyricLine, token_indices: &[usize]) -> Option<(f32, f32)> {
    let mut bounds: Option<(f32, f32)> = None;
    for &index in token_indices {
        let token = &line.tokens[index];
        let (start, end) = token.start_time.zip(token.end_time)?;
        bounds = Some(match bounds {
            Some((min_start, max_end)) => (min_start.min(start), max_end.max(end)),
            None => (start, end),
        });
    }
    bounds
}

fn visible_token_text(token: &LyricToken) -> &str {
    token.text.split('|').next().unwrap_or(&token.text)
}
