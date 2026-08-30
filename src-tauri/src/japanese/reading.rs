// 日语读音 provider 抽象
//
// 优先级 (任务书 §2.4):
//   用户 override > ruby/furigana > UniDic 词典 > KanaOnly > heuristic
//
// Lindera + UniDic 接入说明 (P1):
//   - 当前版本通过 lindera 的 embed-unidic 特性直接提供词典读音
//   - JapaneseReadingProvider 返回 ReadingSpan，所有下游坐标统一为 display char
//   - 歌曲特殊读法 (運命→さだめ) 通过 ReadingOverride 接口覆盖词典结果
//   - Ruby 注音在 mora 展开前进入 ReadingOverride，优先级高于词典

use crate::japanese::mora;
use crate::models::ReadingSpan;

/// Ruby 注音解析结果。所有 display_* 坐标均以去掉注音标记后的 display_text
/// 为基准；raw_* 坐标仅用于诊断原始标记。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RubyAnnotation {
    pub surface: String,
    pub reading: String,
    pub display_start: usize,
    pub display_end: usize,
    pub raw_start: usize,
    pub raw_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParsedRubyText {
    pub raw_text: String,
    pub display_text: String,
    pub annotations: Vec<RubyAnnotation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RubyParseError {
    InvalidInput(String),
}

fn is_kanji(c: char) -> bool {
    ('\u{3400}'..='\u{4DBF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)
}

fn is_ruby_reading_char(c: char) -> bool {
    mora::is_kana_char(c) || c == '\u{30FC}' || c == '\u{30FB}'
}

fn is_escaped(chars: &[char], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut i = index;
    while i > 0 && chars[i - 1] == '\\' {
        backslashes += 1;
        i -= 1;
    }
    backslashes % 2 == 1
}

/// 解析 `漢字(かな)` 注音标记。
///
/// 不符合 Ruby 规则的括号按普通歌词文本保留；反斜杠只用于转义括号，
/// 不会进入 display_text。这使 malformed Ruby 不会破坏歌词或触发 panic。
pub fn parse_ruby_text(raw: &str) -> Result<ParsedRubyText, RubyParseError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut display = String::new();
    let mut annotations = Vec::new();
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '(' | ')') {
            display.push(chars[i + 1]);
            i += 2;
            continue;
        }

        if chars[i] == '(' && !is_escaped(&chars, i) && i > 0 && is_kanji(chars[i - 1]) {
            let surface_start = (0..i)
                .rev()
                .find(|&j| !is_kanji(chars[j]))
                .map(|j| j + 1)
                .unwrap_or(0);
            let close = ((i + 1)..chars.len()).find(|&j| chars[j] == ')' && !is_escaped(&chars, j));
            if let Some(close) = close {
                let reading_chars = &chars[i + 1..close];
                if !reading_chars.is_empty()
                    && reading_chars.iter().all(|&c| is_ruby_reading_char(c))
                {
                    let surface: String = chars[surface_start..i].iter().collect();
                    let reading: String = reading_chars.iter().collect();
                    let surface_len = surface.chars().count();
                    let display_end = display.chars().count();
                    let display_start = display_end.saturating_sub(surface_len);
                    annotations.push(RubyAnnotation {
                        surface,
                        reading,
                        display_start,
                        display_end,
                        raw_start: surface_start,
                        raw_end: close + 1,
                    });
                    i = close + 1;
                    continue;
                }
            }
        }

        display.push(chars[i]);
        i += 1;
    }

    Ok(ParsedRubyText {
        raw_text: raw.to_string(),
        display_text: display,
        annotations,
    })
}

/// 将一个相对于去 timing-tag 文本的 raw char offset 映射到 display offset。
/// Enhanced-LRC 的 `<time>` 标签应先被移除，再应用 Ruby 删除映射。
pub fn ruby_display_offset(parsed: &ParsedRubyText, raw_position: usize) -> usize {
    for annotation in &parsed.annotations {
        if raw_position < annotation.raw_start {
            break;
        }
        if raw_position < annotation.raw_end {
            let within_surface = raw_position.saturating_sub(annotation.raw_start);
            return annotation
                .display_start
                .saturating_add(within_surface.min(annotation.surface.chars().count()));
        }
    }
    let removed: usize = parsed
        .annotations
        .iter()
        .filter(|annotation| annotation.raw_end <= raw_position)
        .map(|annotation| {
            annotation
                .raw_end
                .saturating_sub(annotation.raw_start)
                .saturating_sub(annotation.surface.chars().count())
        })
        .sum();
    raw_position.saturating_sub(removed)
}

/// 日语读音分析器抽象。实现必须无锁可共享 (Send + Sync)。
pub trait JapaneseReadingProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// 把一行歌词原文解析为 ReadingSpan 序列。
    /// span 按原文顺序排列, char 区间覆盖整个原文 (包括非假名片段)。
    /// 假名片段 reading = 原文; 未知读音片段 reading = 空 (confidence = 0)。
    fn analyze(&self, text: &str) -> Result<Vec<ReadingSpan>, String>;
}

/// 歌曲特殊读音 override (運命 → さだめ)
#[derive(Debug, Clone)]
pub struct ReadingOverride {
    /// 原文 char 区间
    pub char_start: usize,
    pub char_end: usize,
    /// 规范化假名读音
    pub reading: String,
}

/// 纯假名快速路径 / 词典缺失时的兜底:
/// 假名片段自带读音; 汉字/拉丁片段读音未知 (由上层用时长先验兜底)
pub struct KanaOnlyProvider;

/// 一个连续同类片段 (假名 / 非假名)
struct RawFragment {
    text: String,
    char_start: usize,
    char_end: usize,
    is_kana: bool,
}

fn split_fragments(text: &str) -> Vec<RawFragment> {
    let mut fragments: Vec<RawFragment> = Vec::new();
    for (ci, c) in text.chars().enumerate() {
        let is_kana = mora::is_kana_char(c);
        let is_chionpu = c == '\u{30FC}';
        let treat_as_kana = is_kana || is_chionpu;
        if let Some(last) = fragments.last_mut() {
            if last.is_kana == treat_as_kana {
                last.text.push(c);
                last.char_end = ci + 1;
                continue;
            }
        }
        fragments.push(RawFragment {
            text: c.to_string(),
            char_start: ci,
            char_end: ci + 1,
            is_kana: treat_as_kana,
        });
    }
    fragments
}

impl JapaneseReadingProvider for KanaOnlyProvider {
    fn name(&self) -> &'static str {
        "kana-only"
    }

    fn analyze(&self, text: &str) -> Result<Vec<ReadingSpan>, String> {
        let fragments = split_fragments(text);
        let mut spans: Vec<ReadingSpan> = Vec::new();
        let mut mora_offset = 0usize;
        for frag in fragments {
            let (reading, confidence) = if frag.is_kana {
                (frag.text.clone(), 1.0)
            } else {
                (String::new(), 0.0)
            };
            let mora_start = mora_offset;
            if frag.is_kana {
                mora_offset += mora::parse_kana_moras(&frag.text).len();
            }
            spans.push(ReadingSpan {
                surface: frag.text.clone(),
                pronunciation: reading.clone(),
                reading,
                char_start: frag.char_start,
                char_end: frag.char_end,
                // display span 与原文一致 (kana-only 不做跨字合并)
                display_start: frag.char_start,
                display_end: frag.char_end,
                mora_start,
                mora_end: mora_offset,
                confidence,
            });
        }
        Ok(spans)
    }
}

/// 应用 override: 命中 char 区间的 span 更新读音并提升 confidence,
/// 然后按新读音重算所有 span 的 mora 区间 (override 可能改变拍数)
pub fn apply_reading_overrides(
    spans: &mut Vec<ReadingSpan>,
    overrides: &[ReadingOverride],
    text: &str,
) {
    // A Ruby span is the language truth even if the dictionary segmented the
    // surface into several morphemes. Replace the complete intersecting range
    // with one span so `二人(ふたり)` can never duplicate all three moras onto
    // both glyphs.
    for ov in overrides {
        let first = spans
            .iter()
            .position(|span| ov.char_start < span.char_end && span.char_start < ov.char_end);
        let Some(first) = first else { continue };
        let last = spans
            .iter()
            .rposition(|span| ov.char_start < span.char_end && span.char_start < ov.char_end)
            .unwrap_or(first);
        let char_start = ov.char_start;
        let char_end = ov.char_end;
        let surface: String = text
            .chars()
            .skip(char_start)
            .take(char_end.saturating_sub(char_start))
            .collect();
        let display_start = spans[first].display_start.min(char_start);
        let display_end = spans[last].display_end.max(char_end);
        spans.splice(
            first..=last,
            [ReadingSpan {
                surface,
                pronunciation: ov.reading.clone(),
                reading: ov.reading.clone(),
                char_start,
                char_end,
                display_start,
                display_end,
                mora_start: 0,
                mora_end: 0,
                confidence: 1.0,
            }],
        );
    }
    // 顺序重算 mora 区间: 假名 span 按读音解析, 非假名 span 0 拍
    let mut offset = 0usize;
    for span in spans.iter_mut() {
        span.mora_start = offset;
        if !span.reading.is_empty() {
            offset += mora::parse_kana_moras(&span.reading).len();
        }
        span.mora_end = offset;
    }
}
