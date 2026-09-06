// Mandarin pipeline (Round4 Commit 7)
//
// 把 Mandarin 行的 provider 读音接入歌词三层文本结构:
//   LyricLine.reading_spans (pinyin reading)
//   LyricLine.moras (每字一个音节单元 — 供 DP 对齐/binder)
//   LyricLine.reading_display_groups (1:1 每字一组)
//
// 1 汉字 = 1 音节 (§9.1); 读音未知的字仍有音节结构 (pinyin 空)。
// v4 PronunciationUnit 的 MandarinSyllable kind 由投影层生成。

use crate::japanese::mora;
use crate::mandarin::overrides::{apply_mandarin_overrides, MandarinOverride};
use crate::mandarin::reading::{MandarinReadingProvider, ReadingQuality};
use crate::models::{LyricLine, MoraUnit, ReadingDisplayGroup, ReadingSpan};

/// 构建 Mandarin 行的读音层。`overrides` 来自用户 (最高优先级)。
pub fn build_mandarin_layers(
    line: &mut LyricLine,
    provider: &dyn MandarinReadingProvider,
    overrides: &[MandarinOverride],
) {
    let Ok(mut spans_raw) = provider.analyze(&line.primary_text) else {
        // provider 失败: 不伪造读音, 退回纯显示层
        crate::lyrics::build_unknown_display_layers_pub(line);
        return;
    };
    apply_mandarin_overrides(&mut spans_raw, overrides);

    let mut reading_spans: Vec<ReadingSpan> = Vec::new();
    let mut moras: Vec<MoraUnit> = Vec::new();
    let mut groups: Vec<ReadingDisplayGroup> = Vec::new();

    for (i, syllable) in spans_raw.iter().enumerate() {
        let confidence = match syllable.quality {
            ReadingQuality::Contextual => 1.0,
            ReadingQuality::Dictionary => 0.95,
            ReadingQuality::Ambiguous => 0.5,
            ReadingQuality::Unknown => 0.0,
        };
        reading_spans.push(ReadingSpan {
            surface: syllable.surface.clone(),
            reading: syllable.pinyin.clone(),
            pronunciation: syllable.pinyin.clone(),
            char_start: syllable.char_start,
            char_end: syllable.char_end,
            display_start: syllable.char_start,
            display_end: syllable.char_end,
            mora_start: i,
            mora_end: i + 1,
            confidence,
        });

        let id = groups.len();
        groups.push(ReadingDisplayGroup {
            id,
            char_start: syllable.char_start,
            char_end: syllable.char_end,
            reading_span_id: i,
            mora_start: i,
            mora_end: i + 1,
            surface: syllable.surface.clone(),
            reading: syllable.pinyin.clone(),
            phonetic: false,
            start_time: None,
            end_time: None,
            pitch_notes: Vec::new(),
            primary_note: None,
            unpitched_reason: None,
        });

        moras.push(MoraUnit {
            // 兼容层: MoraUnit.kana 字段承载音节文本 (拼音)
            kana: if syllable.pinyin.is_empty() {
                syllable.surface.clone()
            } else {
                syllable.pinyin.clone()
            },
            phonemes: syllable.phonemes.clone(),
            reading_span_id: i,
            char_start: syllable.char_start,
            char_end: syllable.char_end,
            reading_offset_start: 0,
            reading_offset_end: mora::normalize_kana(&syllable.pinyin).chars().count(),
            display_start: syllable.char_start,
            display_end: syllable.char_end,
            display_group_id: id,
            start_time: None,
            end_time: None,
            confidence,
            note_bindings: Vec::new(),
        });
    }

    // 非汉字片段的 1:1 显示组 (读音为空, 承载显示时序)
    let covered: Vec<(usize, usize)> = spans_raw
        .iter()
        .map(|s| (s.char_start, s.char_end))
        .collect();
    for token in line.tokens.iter() {
        let overlaps = covered
            .iter()
            .any(|(cs, ce)| token.char_start < *ce && *cs < token.char_end);
        if !overlaps {
            let id = groups.len();
            groups.push(ReadingDisplayGroup {
                id,
                char_start: token.char_start,
                char_end: token.char_end,
                reading_span_id: usize::MAX,
                mora_start: 0,
                mora_end: 0,
                surface: token.text.clone(),
                reading: String::new(),
                phonetic: false,
                start_time: None,
                end_time: None,
                pitch_notes: Vec::new(),
                primary_note: None,
                unpitched_reason: None,
            });
        }
    }

    // token ↔ group/span 关联 (与日语路径一致)
    let mut tokens = std::mem::take(&mut line.tokens);
    for token in tokens.iter_mut() {
        for (si, span) in reading_spans.iter().enumerate() {
            if token.char_start < span.char_end && span.char_start < token.char_end {
                token.reading_span_ids.push(si);
            }
        }
        for group in &groups {
            if token.char_start < group.char_end && group.char_start < token.char_end {
                token.reading_group_ids.push(group.id);
            }
        }
    }
    line.tokens = tokens;
    line.reading_spans = reading_spans;
    line.reading_display_groups = groups;
    line.moras = moras;
}
