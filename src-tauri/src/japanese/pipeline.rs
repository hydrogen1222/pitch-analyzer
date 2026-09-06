// Japanese pipeline (Round4 任务书 §8)
//
// 从 lyrics.rs 搬入的日语三层文本结构构建 (Commit 6):
//   raw lyrics → ruby parse → UniDic contextual reading → ReadingSpan
//   → normalized kana pronunciation → Mora → Phoneme → pitch-bearing nucleus
//
// Reading 优先级 (§8.1): UserRuby > UniDic contextual > Kana surface > Unknown

use crate::japanese::reading::{apply_reading_overrides, JapaneseReadingProvider, ReadingOverride};
use crate::japanese::{mora, phoneme, KanaOnlyProvider, LinderaUnidicProvider};
use crate::models::{LyricLine, MoraUnit, ReadingDisplayGroup};

/// 填充行的日语三层文本结构: reading_spans → moras → token↔span 关联。
/// 优先级: UserRuby > UniDic (Lindera) > KanaOnly (纯假名快速兜底)
pub fn build_japanese_layers(line: &mut LyricLine) {
    let unidic = LinderaUnidicProvider;
    let mut spans = if let Ok(s) = unidic.analyze(&line.primary_text) {
        s
    } else {
        let kana_only = KanaOnlyProvider;
        if let Ok(s) = kana_only.analyze(&line.primary_text) {
            s
        } else {
            return;
        }
    };

    // Explicit Ruby is the highest-priority reading truth. Applying it before
    // mora expansion also collapses a multi-glyph word into one span.
    if !line.ruby_annotations.is_empty() {
        let overrides: Vec<ReadingOverride> = line
            .ruby_annotations
            .iter()
            .map(|annotation| ReadingOverride {
                char_start: annotation.display_start,
                char_end: annotation.display_end,
                reading: annotation.reading.clone(),
            })
            .collect();
        apply_reading_overrides(&mut spans, &overrides, &line.primary_text);
    }

    // moras: 必须从读音 (pronunciation 优先, 其次 reading) 展开, 而非 surface
    // (任务书 5.2: surface="愛" reading="あい" 时拿 surface 拆 mora 永远得 0)
    let mut moras: Vec<MoraUnit> = Vec::new();
    let mut reading_display_groups: Vec<ReadingDisplayGroup> = Vec::new();
    for (si, span) in spans.iter().enumerate() {
        let phonetic = if !span.pronunciation.is_empty() {
            span.pronunciation.clone()
        } else {
            span.reading.clone()
        };
        if phonetic.is_empty() {
            let id = reading_display_groups.len();
            reading_display_groups.push(ReadingDisplayGroup {
                id,
                char_start: span.display_start,
                char_end: span.display_end,
                reading_span_id: si,
                mora_start: span.mora_start,
                mora_end: span.mora_end,
                surface: span.surface.clone(),
                reading: String::new(),
                phonetic: false,
                start_time: None,
                end_time: None,
                pitch_notes: Vec::new(),
                primary_note: None,
                unpitched_reason: None,
            });
            continue; // 读音未知 (需词典/override), 不产生 mora
        }
        // reading 与 surface 文本一致 (kana-only) → char 区间 1:1 精确;
        // 词典读音 → mora 继承整个 span 的 surface 区间 (coarse, 不伪造逐字映射)
        // 比较在规范化域进行 (半角/全角片假名等价, 任务书 §8.4)
        let normalized_surface = mora::normalize_kana(&span.surface);
        let exact = phonetic == span.surface
            || phonetic == normalized_surface
            || phonetic == crate::japanese::unidic::katakana_to_hiragana(&normalized_surface);
        // For explicit 漢字(かな), the reading is authoritative and is also the
        // requested pitch display axis. Split it into one acoustic/display slot
        // per mora instead of attaching one coarse badge to the source kanji.
        let explicit_phonetic = line.ruby_annotations.iter().any(|annotation| {
            annotation.display_start == span.display_start
                && annotation.display_end == span.display_end
                && annotation.reading == phonetic
        });
        let split_per_mora = exact || explicit_phonetic;
        let parsed_moras = mora::parse_kana_moras(&phonetic);
        let coarse_group_id = if split_per_mora {
            None
        } else {
            let id = reading_display_groups.len();
            reading_display_groups.push(ReadingDisplayGroup {
                id,
                char_start: span.display_start,
                char_end: span.display_end,
                reading_span_id: si,
                mora_start: span.mora_start,
                mora_end: span.mora_end,
                surface: span.surface.clone(),
                reading: phonetic.clone(),
                phonetic: false,
                start_time: None,
                end_time: None,
                pitch_notes: Vec::new(),
                primary_note: None,
                unpitched_reason: None,
            });
            Some(id)
        };
        for (local_mora_index, pm) in parsed_moras.into_iter().enumerate() {
            let (cs, ce) = if exact {
                (
                    span.char_start + pm.char_start,
                    span.char_start + pm.char_end,
                )
            } else {
                (span.char_start, span.char_end) // coarse display span
            };
            let display_group_id = if let Some(id) = coarse_group_id {
                id
            } else {
                let id = reading_display_groups.len();
                let surface: String = if exact {
                    line.primary_text
                        .chars()
                        .skip(cs)
                        .take(ce.saturating_sub(cs))
                        .collect()
                } else {
                    span.surface.clone()
                };
                reading_display_groups.push(ReadingDisplayGroup {
                    id,
                    char_start: cs,
                    char_end: ce,
                    reading_span_id: si,
                    mora_start: span.mora_start + local_mora_index,
                    mora_end: span.mora_start + local_mora_index + 1,
                    surface,
                    reading: pm.kana.clone(),
                    phonetic: explicit_phonetic,
                    start_time: None,
                    end_time: None,
                    pitch_notes: Vec::new(),
                    primary_note: None,
                    unpitched_reason: None,
                });
                id
            };
            let phonemes = phoneme::mora_to_phonemes(&pm.kana);
            moras.push(MoraUnit {
                kana: pm.kana,
                phonemes,
                reading_span_id: si,
                char_start: cs,
                char_end: ce,
                reading_offset_start: pm.char_start,
                reading_offset_end: pm.char_end,
                display_start: if exact { cs } else { span.display_start },
                display_end: if exact { ce } else { span.display_end },
                display_group_id,
                start_time: None,
                end_time: None,
                confidence: span.confidence,
                note_bindings: Vec::new(),
            });
        }
    }

    // token ↔ reading_span/display_group 关联 (canonical display char offset)
    let mut tokens = std::mem::take(&mut line.tokens);
    for token in tokens.iter_mut() {
        for (si, span) in spans.iter().enumerate() {
            if token.char_start < span.char_end && span.char_start < token.char_end {
                token.reading_span_ids.push(si);
            }
        }
        for group in &reading_display_groups {
            if token.char_start < group.char_end && group.char_start < token.char_end {
                token.reading_group_ids.push(group.id);
            }
        }
    }
    line.tokens = tokens;
    line.reading_spans = spans;
    line.reading_display_groups = reading_display_groups;
    line.moras = moras;
}
