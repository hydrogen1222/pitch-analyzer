// 歌词解析: LRC + TXT, tokenizer, 字级对齐, NoteEvent 绑定, primary note 选择

use crate::japanese::reading::JapaneseReadingProvider;
use crate::japanese::{mora, phoneme, KanaOnlyProvider};
use crate::note_engine::NoteWindow;
use crate::models::{
    AlignmentSource, LyricLine, LyricToken, MoraUnit, NoteEvent, NoteTrackingParams, PitchNote,
    PitchTrack, UnpitchedReason,
};
use regex::Regex;
use std::cmp::Ordering;

// ── Tokenizer ──────────────────────────────────────────────

/// 带原文 char 区间的 token (显示 token 可覆盖多个 char: きゃ / りー / 汉字+标点)
#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
}

/// 日文拍合并 (显示层): 小書き与长音符并入前一假名 token。
/// 注意: ー 合并只是显示分组, mora 计数由 token_weight 单独负责。
fn merge_japanese_attach_chars_spans(tokens: &mut Vec<SpannedToken>) {
    let mut i = 1;
    while i < tokens.len() {
        let is_single_attach = {
            let mut chars = tokens[i].text.chars();
            let first = chars.next();
            tokens[i].text.chars().count() == 1
                && first
                    .map(|c| is_japanese_attach_char(c) || is_chionpu(c))
                    .unwrap_or(false)
        };
        let prev_ends_kana = tokens[i - 1]
            .text
            .chars()
            .last()
            .map(|c| mora::is_kana_char(c) || is_chionpu(c))
            .unwrap_or(false);
        if is_single_attach && prev_ends_kana {
            let attach = tokens.remove(i);
            let prev = &mut tokens[i - 1];
            prev.text.push_str(&attach.text);
            prev.char_end = attach.char_end;
        } else {
            i += 1;
        }
    }
}

/// 分词核心: 返回带 char 区间的 token
pub fn tokenize_core(text: &str) -> Vec<SpannedToken> {
    if text.is_empty() {
        return Vec::new();
    }
    let re = Regex::new(
        r"(?x)
        [a-zA-Z0-9]+(?:['\-][a-zA-Z0-9]+)*  # English/Latin words
        |[\x{4e00}-\x{9fff}]                 # Chinese Hanzi
        |[\x{3040}-\x{309f}]                 # Japanese Hiragana
        |[\x{30a0}-\x{30ff}]                 # Japanese Katakana
        |[^\s]                               # Fallback
        ",
    )
    .expect("Invalid tokenizer regex pattern");

    let non_word_re = Regex::new(r"^[^\w\x{4e00}-\x{9fff}\x{3040}-\x{309f}\x{30a0}-\x{30ff}]+$")
        .expect("Invalid non-word regex pattern");

    // byte offset → char offset 查表
    let char_of_byte: Vec<usize> = {
        let mut map = Vec::with_capacity(text.len() + 1);
        for (ci, (b, _)) in text.char_indices().enumerate() {
            map.resize(b, ci);
            map.push(ci);
        }
        map.resize(text.len() + 1, text.chars().count());
        map
    };

    let mut tokens: Vec<SpannedToken> = Vec::new();
    for m in re.find_iter(text) {
        let cs = char_of_byte[m.start()];
        let ce = char_of_byte[m.end()];
        let piece = m.as_str();
        if non_word_re.is_match(piece) && !tokens.is_empty() {
            // 标点并入前一个显示 token (纯显示, 0 莫拉)
            let last = tokens.last_mut().unwrap();
            last.text.push_str(piece);
            last.char_end = ce;
        } else {
            tokens.push(SpannedToken {
                text: piece.to_string(),
                char_start: cs,
                char_end: ce,
            });
        }
    }
    merge_japanese_attach_chars_spans(&mut tokens);
    tokens
}

pub fn tokenize(text: &str) -> Vec<String> {
    tokenize_core(text).into_iter().map(|t| t.text).collect()
}

/// 填充行的日语三层文本结构: reading_spans → moras → token↔span 关联。
/// 优先级: UniDic (Lindera) > KanaOnly (纯假名快速兜底)
fn build_japanese_layers(line: &mut LyricLine) {
    let unidic = crate::japanese::LinderaUnidicProvider;
    let spans = if let Ok(s) = unidic.analyze(&line.primary_text) {
        s
    } else {
        let kana_only = KanaOnlyProvider;
        if let Ok(s) = kana_only.analyze(&line.primary_text) {
            s
        } else {
            return;
        }
    };

    // moras: 必须从读音 (pronunciation 优先, 其次 reading) 展开, 而非 surface
    // (任务书 5.2: surface="愛" reading="あい" 时拿 surface 拆 mora 永远得 0)
    let mut moras: Vec<MoraUnit> = Vec::new();
    for (si, span) in spans.iter().enumerate() {
        let phonetic = if !span.pronunciation.is_empty() {
            span.pronunciation.clone()
        } else {
            span.reading.clone()
        };
        if phonetic.is_empty() {
            continue; // 读音未知 (需词典/override), 不产生 mora
        }
        // reading 与 surface 文本一致 (kana-only) → char 区间 1:1 精确;
        // 词典读音 → mora 继承整个 span 的 surface 区间 (coarse, 不伪造逐字映射)
        let exact = phonetic == span.surface;
        for pm in mora::parse_kana_moras(&phonetic) {
            let (cs, ce) = if exact {
                (
                    span.char_start + pm.char_start,
                    span.char_start + pm.char_end,
                )
            } else {
                (span.char_start, span.char_end) // coarse display span
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
                display_start: span.display_start,
                display_end: span.display_end,
                start_time: None,
                end_time: None,
                confidence: span.confidence,
                note_bindings: Vec::new(),
            });
        }
    }

    // token ↔ reading_span 关联 (char 区间求交)
    let mut tokens = std::mem::take(&mut line.tokens);
    for token in tokens.iter_mut() {
        for (si, span) in spans.iter().enumerate() {
            if token.char_start < span.char_end && span.char_start < token.char_end {
                token.reading_span_ids.push(si);
            }
        }
    }
    line.tokens = tokens;
    line.reading_spans = spans;
    line.moras = moras;
}

// ── 莫拉计数权重 (无词典 fallback; P1 UniDic 后仅保留兜底) ─────────

/// 长音符 ー (U+30FC): UI 上并入前字显示, 但语义上是独立的一拍
fn is_chionpu(c: char) -> bool {
    c == '\u{30FC}'
}

/// 小書き仮名 (显示合并用)
fn is_japanese_attach_char(c: char) -> bool {
    matches!(
        c,
        '\u{3041}'
            | '\u{3043}'
            | '\u{3045}'
            | '\u{3047}'
            | '\u{3049}'
            | '\u{3083}'
            | '\u{3085}'
            | '\u{3087}'
            | '\u{30A1}'
            | '\u{30A3}'
            | '\u{30A5}'
            | '\u{30A7}'
            | '\u{30A9}'
            | '\u{30E3}'
            | '\u{30E5}'
            | '\u{30E7}'
            | '\u{30EE}'
    )
}

pub fn is_kanji_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

/// 拉丁词的音节估计: 元音簇个数 (hello=2, through=1)
fn latin_syllable_count(word: &str) -> f32 {
    let vowels = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
    let mut count = 0f32;
    let mut prev_vowel = false;
    for c in word.to_lowercase().chars() {
        if vowels(c) {
            if !prev_vowel {
                count += 1.0;
            }
            prev_vowel = true;
        } else {
            prev_vowel = false;
        }
    }
    count.max(1.0)
}

/// 估算 token 的莫拉 (拍) 数。
/// 假名部分用真实 mora 解析 (きゃ=1, スーパー=4, がっこう=4);
/// 汉字 ≈1.8 拍 (统计均值, 无词典 fallback); 标点 = 0; 拉丁词 = 元音簇数。
pub fn token_weight(text: &str) -> f32 {
    if text.is_empty() {
        return 1.0;
    }
    let first = text.chars().next().unwrap();
    if first.is_ascii_alphabetic() {
        return latin_syllable_count(text);
    }
    // 假名/长音符/促音按 mora 状态机计数
    let mut w = mora::parse_kana_moras(text).len() as f32;
    for c in text.chars() {
        if is_kanji_char(c) {
            w += 1.8;
        } else if mora::is_kana_char(c) || c == '\u{30FC}' {
            // 已由 mora 解析计入
        } else if !c.is_alphanumeric() {
            // 标点: 纯显示, 0 拍
        } else {
            w += 1.0;
        }
    }
    w.max(1.0)
}

// ── TXT Parser ─────────────────────────────────────────────

pub fn parse_txt(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for line_str in text.lines() {
        let trimmed = line_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut line = LyricLine::new(
            trimmed.to_string(),
            trimmed.to_string(),
            Vec::new(),
            None,
            None,
        );
        line.tokens = tokenize_core(trimmed)
            .into_iter()
            .map(|t| LyricToken::new(t.text, t.char_start, t.char_end))
            .collect();
        build_japanese_layers(&mut line);
        lines.push(line);
    }
    lines
}

// ── LRC Parser ─────────────────────────────────────────────

pub fn parse_lrc(text: &str, audio_duration: Option<f32>) -> Vec<LyricLine> {
    // 宽松时间戳: [mm:ss] / [mm:ss.f] / [mm:ss.ff] / [mm:ss.fff], 分和秒可 1-2 位
    let time_re = Regex::new(r"\[(\d{1,2}):(\d{1,2})(?:\.(\d{1,3}))?\]")
        .expect("Invalid LRC time regex pattern");
    let word_re = Regex::new(r"<(\d{1,3}):(\d{2})(?:\.(\d{1,3}))?>")
        .expect("Invalid enhanced LRC word regex pattern");

    fn parse_tag_time(cap: &regex::Captures) -> f32 {
        let mins: f32 = cap[1].parse().unwrap_or(0.0);
        let s: f32 = cap[2].parse().unwrap_or(0.0);
        let frac = cap.get(3).map(|f| {
            let digits = f.as_str();
            let v: f32 = digits.parse().unwrap_or(0.0);
            v / 10f32.powi(digits.len() as i32)
        });
        mins * 60.0 + s + frac.unwrap_or(0.0)
    }

    #[derive(Debug, Clone)]
    struct RawEntry {
        start_time: f32,
        /// 扩展行时格式 ([start]text[end]) 自带的结束时间, None = 未提供
        end_time: Option<f32>,
        text: String,
        /// enhanced LRC 逐字锚点: (原文 char 位置, 时间) — 权威逐字时间,
        /// 绑定到原文 span 而非某次分词的 token 下标
        anchors: Vec<(usize, f32)>,
    }

    let mut entries: Vec<RawEntry> = Vec::new();
    for line_str in text.lines() {
        let trimmed = line_str.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tags: Vec<(f32, usize, usize)> = time_re
            .captures_iter(trimmed)
            .filter_map(|c| {
                let m = c.get(0)?;
                Some((parse_tag_time(&c), m.start(), m.end()))
            })
            .collect();
        if tags.is_empty() {
            continue;
        }
        let raw_content = time_re.replace_all(trimmed, "").trim().to_string();
        if raw_content.is_empty() {
            continue;
        }

        // 区分两种多标签行:
        //   扩展行时: [start]text[end]  (首尾标签之间夹着文本) → 一个条目, 带结束时间
        //   多时间戳: [t1][t2]text      (标签全部连在开头)     → 每个时间戳一个条目
        let has_text_between = tags.len() >= 2 && {
            let between = &trimmed[tags[0].2..tags[tags.len() - 1].1];
            !time_re.replace_all(between, "").trim().is_empty()
        };
        let (starts, explicit_end): (Vec<f32>, Option<f32>) = if has_text_between {
            (vec![tags[0].0], Some(tags[tags.len() - 1].0))
        } else {
            (tags.iter().map(|(t, _, _)| *t).collect(), None)
        };

        // enhanced LRC 逐字锚点: <00:12.00>如<00:12.50>果
        // 锚点位置 = 标签结束处 (其后紧跟该时间对应的文字), 绑定原文 span
        let mut anchors: Vec<(usize, f32)> = Vec::new();
        for cap in word_re.captures_iter(&raw_content) {
            let m = cap.get(0).unwrap();
            let secs: f32 = {
                let mins: f32 = cap[1].parse().unwrap_or(0.0);
                let sec: f32 = cap[2].parse().unwrap_or(0.0);
                let frac = cap.get(3).map(|f| {
                    let digits = f.as_str();
                    let v: f32 = digits.parse().unwrap_or(0.0);
                    v / 10f32.powi(digits.len() as i32)
                });
                mins * 60.0 + sec + frac.unwrap_or(0.0)
            };
            let char_pos = raw_content[..m.end()].chars().count();
            anchors.push((char_pos, secs));
        }

        let text_clean = word_re.replace_all(&raw_content, "").trim().to_string();
        if text_clean.is_empty() {
            continue;
        }

        for &start in &starts {
            entries.push(RawEntry {
                start_time: start,
                end_time: explicit_end,
                text: text_clean.clone(),
                anchors: anchors.clone(),
            });
        }
    }
    entries.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());

    // 合并双语（相同时间戳 ±50ms）
    struct MergedEntry {
        start_time: f32,
        end_time: Option<f32>,
        text: String,
        translations: Vec<String>,
        anchors: Vec<(usize, f32)>,
    }
    let mut merged: Vec<MergedEntry> = Vec::new();
    for entry in entries {
        if let Some(last) = merged.last_mut() {
            if (last.start_time - entry.start_time).abs() < 0.05 {
                last.translations.push(entry.text);
                // 取更长的结束时间 (双语行通常一致)
                if entry
                    .end_time
                    .is_some_and(|e| last.end_time.is_none_or(|le| e > le))
                {
                    last.end_time = entry.end_time;
                }
                continue;
            }
        }
        merged.push(MergedEntry {
            start_time: entry.start_time,
            end_time: entry.end_time,
            text: entry.text,
            translations: Vec::new(),
            anchors: entry.anchors,
        });
    }

    let mut lines = Vec::new();
    for (i, entry) in merged.iter().enumerate() {
        let start_time = entry.start_time;
        let next_start = merged.get(i + 1).map(|m| m.start_time);
        // 行结束时间: 优先用扩展行时自带的 end (不越过下一行起点);
        // 否则用下一行起点, 最后一行用音频时长
        let end_time = match entry.end_time {
            Some(e) if e > start_time => match next_start {
                Some(ns) if ns > start_time && ns < e => ns,
                _ => e,
            },
            _ => match next_start {
                Some(next) if next > start_time => next,
                _ => audio_duration.unwrap_or(start_time + 0.1),
            },
        };

        let primary_text = entry.text.clone();
        let mut line = LyricLine::new(
            String::new(),
            primary_text.clone(),
            entry.translations.clone(),
            Some(start_time),
            Some(end_time),
        );
        line.text = if entry.translations.is_empty() {
            primary_text.clone()
        } else {
            format!("{} | {}", primary_text, entry.translations.join(" / "))
        };
        line.tokens = tokenize_core(&primary_text)
            .into_iter()
            .map(|t| LyricToken::new(t.text, t.char_start, t.char_end))
            .collect();
        build_japanese_layers(&mut line);

        // enhanced LRC: 歌词自带逐字锚点 (绑定原文 span) → 直接使用, 不做自动估计
        if entry.anchors.len() >= 2 {
            apply_enhanced_anchor_timings(&mut line, &entry.anchors, end_time);
        }

        lines.push(line);
    }
    lines
}

/// enhanced LRC 逐字锚点按原文 span 应用 (不要求分词结果与逐字块一一对应):
/// token 依据 char 区间落入锚点区间, 组内按莫拉权重分配时间。
/// 权威来源标记为 AlignmentSource::EnhancedLrc, 任何重新分词都不会丢失锚点。
fn apply_enhanced_anchor_timings(line: &mut LyricLine, anchors: &[(usize, f32)], line_end: f32) {
    if anchors.len() < 2 || line.tokens.is_empty() {
        return;
    }
    let first_pos = anchors[0].0;
    let line_start = line.start_time.unwrap_or(0.0);
    // 组 0 = 首锚点之前的前导字符, 组 i+1 = 锚点 i
    let mut groups: Vec<usize> = Vec::with_capacity(line.tokens.len());
    for tok in &line.tokens {
        if tok.char_start < first_pos {
            groups.push(0);
        } else {
            let mut g = 1;
            for (i, (pos, _)) in anchors.iter().enumerate() {
                if *pos <= tok.char_start {
                    g = i + 1;
                }
            }
            groups.push(g);
        }
    }
    // 组 i (>=1) 的时间区间 = [t_{i-1}, t_i 或 line_end); 组 0 = [line_start, t_0)
    let mut starts: Vec<f32> = Vec::with_capacity(anchors.len() + 1);
    starts.push(line_start);
    for (_, t) in anchors {
        starts.push(*t);
    }
    let n_groups = starts.len();
    for g in 0..n_groups {
        let g_start = starts[g];
        let g_end = if g + 1 < n_groups {
            starts[g + 1]
        } else {
            line_end
        };
        if g_end <= g_start {
            continue;
        }
        let idxs: Vec<usize> = (0..line.tokens.len()).filter(|&k| groups[k] == g).collect();
        if idxs.is_empty() {
            continue;
        }
        let weights: Vec<f32> = idxs
            .iter()
            .map(|&k| token_weight(&line.tokens[k].text))
            .collect();
        let total: f32 = weights.iter().sum::<f32>().max(1e-3);
        let mut cur = g_start;
        for (&k, &w) in idxs.iter().zip(&weights) {
            let d = (g_end - g_start) * w / total;
            line.tokens[k].start_time = Some(cur);
            line.tokens[k].end_time = Some(cur + d);
            line.tokens[k].alignment_source = Some(AlignmentSource::EnhancedLrc);
            line.tokens[k].alignment_confidence = 1.0;
            cur += d;
        }
    }
}

// ── Token 时间分配 / 对齐 ──────────────────────────────────

/// 字级对齐参数
#[derive(Debug, Clone, Copy)]
pub struct TokenAlignParams {
    /// 最小字时长，防止出现极端短字
    pub min_token_duration_ms: f32,
    /// 对齐窗口在 voiced 区域外扩的 margin
    pub voiced_margin_ms: f32,
}

impl Default for TokenAlignParams {
    fn default() -> Self {
        Self {
            min_token_duration_ms: 60.0,
            voiced_margin_ms: 80.0,
        }
    }
}

/// 均匀分配 token 时间 (作为 fallback，不再是主要算法)
pub fn distribute_token_times(lines: &mut [LyricLine]) {
    for line in lines.iter_mut() {
        let start = match line.start_time {
            Some(t) => t,
            None => continue,
        };
        let end = match line.end_time {
            Some(t) => t,
            None => continue,
        };
        // enhanced LRC 自带的逐字时间不动
        if line
            .tokens
            .iter()
            .all(|t| t.start_time.is_some() && t.end_time.is_some())
            && !line.token_timing_auto
        {
            continue;
        }
        distribute_line_times(line, start, end);
        line.token_timing_auto = true;
    }
}

fn distribute_line_times(line: &mut LyricLine, start: f32, end: f32) {
    if line.tokens.is_empty() {
        return;
    }
    let duration = (end - start).max(0.1);
    // 按莫拉数加权分配 (汉字 ≈1.8 拍, 假名 =1 拍), 而非机械等分
    let weights: Vec<f32> = line.tokens.iter().map(|t| token_weight(&t.text)).collect();
    let total_w: f32 = weights.iter().sum::<f32>().max(1.0);
    let mut current = start;
    for (token, &w) in line.tokens.iter_mut().zip(&weights) {
        token.start_time = Some(current);
        token.end_time = Some(current + duration * w / total_w);
        current += duration * w / total_w;
    }
}

/// 在句级约束内，根据人声音频特征自动分配字级时间。
///
/// 特征: voiced/unvoiced、F0 变化、RMS 能量包络；字符数量作为最终约束；
/// 通过 DP 代价函数选择 N-1 个最佳切分点。无音频特征时回退到均匀分配。
pub fn align_token_times(lines: &mut [LyricLine], track: &PitchTrack, params: &TokenAlignParams) {
    for line in lines.iter_mut() {
        let (start, end) = match (line.start_time, line.end_time) {
            (Some(s), Some(e)) if e > s => (s, e),
            _ => continue,
        };
        if line.tokens.is_empty() {
            continue;
        }
        // 已有逐字时间且非自动估计 (如 enhanced LRC) → 不重新估计;
        // 自动估计过的时间在音轨更新后 (重新分析) 需要重新对齐
        if line
            .tokens
            .iter()
            .all(|t| t.start_time.is_some() && t.end_time.is_some())
            && !line.token_timing_auto
        {
            continue;
        }
        if dp_align_line(line, track, start, end, params) {
            line.token_timing_auto = true;
        } else {
            distribute_line_times(line, start, end);
            line.token_timing_auto = true;
            for token in &mut line.tokens {
                if token.start_time.is_some() {
                    token.alignment_source = Some(AlignmentSource::WeightedFallback);
                }
            }
        }
        // 行首/行尾裁剪: 前奏或间奏很长时, 发声区域之外的时间歌词会长时间
        // 静止挂屏; 发声起点超过行首 2s (行尾超过 1s) 就把显示窗口收紧到演唱附近
        if let Some(first_start) = line.tokens.first().and_then(|t| t.start_time) {
            if let Some(ls) = line.start_time {
                if first_start - ls > 2.0 {
                    line.start_time = Some((first_start - 0.5).max(0.0));
                }
            }
        }
        if let Some(last_end) = line.tokens.last().and_then(|t| t.end_time) {
            if let Some(le) = line.end_time {
                if le - last_end > 1.0 {
                    line.end_time = Some(last_end + 0.3);
                }
            }
        }
    }
}

/// 一个声学对齐单元: 已知 mora (每个 mora 在整行序列中恰好出现一次)
/// 或无读音片段 (汉字无词典读音/拉丁/数字) 的时长先验。
///
/// Round3.1 修复 (任务书 §5-6): 旧实现按 display token overlap 展开 mora,
/// UniDic 的 coarse span (二人→ふたり) 会让同一 mora 相交多个 token 而被
/// 复制多次 (3 mora → 6 unit), 扭曲时长先验与 DP 边界。
/// 现在 mora 是对齐主体; display token 时间在回填阶段按 span 重新聚合。
#[derive(Debug, Clone)]
struct AlignUnit {
    /// 原文排序键 (mora 的 char 起点 / fallback token 的 char 起点)
    sort_char: usize,
    /// 关联 display token (fallback unit 的时间回填目标;
    /// mora unit 的时间回填按 span 相交另行聚合, 不依赖此字段)
    token_idx: usize,
    weight: f32,
    /// 读音已知 (词典/kana mora) = true; 时长先验 = false
    known: bool,
    mora_idx: Option<usize>,
}

/// 从 moras (主体, 每个恰好一次) + 无读音 token (先验) 构建 DP 单元序列,
/// 按原文 char 顺序排序。
fn build_align_units(line: &LyricLine) -> Vec<AlignUnit> {
    let mut units: Vec<AlignUnit> = Vec::new();
    // 1) 已知 mora: 每个恰好一个 unit, 归属到相交的 display token
    //    (coarse span 相交多个 token 时归属第一个; 时间回填按 span 聚合)
    let mut covered = vec![false; line.tokens.len()];
    for (mi, mora) in line.moras.iter().enumerate() {
        let token_idx = line
            .tokens
            .iter()
            .position(|t| mora.char_start < t.char_end && t.char_start < mora.char_end);
        if let Some(ti) = token_idx {
            covered[ti] = true;
        }
        units.push(AlignUnit {
            sort_char: mora.char_start,
            token_idx: token_idx.unwrap_or(usize::MAX),
            weight: 1.0,
            known: true,
            mora_idx: Some(mi),
        });
    }
    // 2) 无 mora 覆盖的 token (读音未知/拉丁/数字): 时长先验 unit
    for (ti, token) in line.tokens.iter().enumerate() {
        if covered[ti] {
            continue;
        }
        units.push(AlignUnit {
            sort_char: token.char_start,
            token_idx: ti,
            weight: token_weight(&token.text),
            known: false,
            mora_idx: None,
        });
    }
    // 3) 原文顺序
    units.sort_by(|a, b| {
        a.sort_char
            .partial_cmp(&b.sort_char)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.mora_idx.unwrap_or(usize::MAX).cmp(&b.mora_idx.unwrap_or(usize::MAX)))
    });
    units
}

/// 测试/调试辅助: 返回 (mora_idx, token_idx, known) 序列 (任务书 §23)
pub fn build_align_units_debug(line: &LyricLine) -> Vec<(Option<usize>, usize, bool)> {
    build_align_units(line)
        .into_iter()
        .map(|u| (u.mora_idx, u.token_idx, u.known))
        .collect()
}

/// 基于声学特征的句内 mora 对齐 (任务书 P2)。
///
///   - 对齐单位是 mora / 先验 unit, 不再强制 "显示字 = 声学段"
///   - 边界证据: 谱通量峰、能量谷/起音、voiced↔unvoiced 转移 (全部与音高无关)
///   - F0 换音不作为歌词边界证据 (转音恰恰证明二者不是同一件事)
///   - 搜索窗口为整行, 不裁剪到 voiced 区域 (无声化/促音也有语言边界信息)
///
/// 返回 false 时由调用方回退加权均匀分配。
#[allow(clippy::needless_range_loop)] // DP 的 (k, j) 二维索引是算法本体
fn dp_align_line(
    line: &mut LyricLine,
    track: &PitchTrack,
    line_start: f32,
    line_end: f32,
    params: &TokenAlignParams,
) -> bool {
    let units = build_align_units(line);
    let n_units = units.len();
    if n_units < 2 || track.times.is_empty() {
        return false;
    }

    // 行内帧索引 (整个行窗口)
    let mut fidx: Vec<usize> = Vec::new();
    for (i, &t) in track.times.iter().enumerate() {
        if t >= line_start && t < line_end {
            fidx.push(i);
        }
    }
    if fidx.len() < 2 {
        return false;
    }

    // 软活动窗口: 有 voicing 或有能量 (RMS) 的帧 ± margin。
    // 不再硬性要求 F0 (无声化/促音闭塞的语言单位 F0 缺失但有能量),
    // 但纯数字静音仍被排除, 避免歌词摊进无声区
    let margin = params.voiced_margin_ms / 1000.0;
    let mut v_first: Option<usize> = None;
    let mut v_last: usize = fidx[0];
    for &fi in &fidx {
        let voiced = !track.midis[fi].is_nan();
        let has_energy = track.rms.get(fi).copied().unwrap_or(0.0) > 0.01;
        if voiced || has_energy {
            if v_first.is_none() {
                v_first = Some(fi);
            }
            v_last = fi;
        }
    }
    let align_start = match v_first {
        Some(fi) => line_start.max(track.times[fi] - margin),
        None => line_start,
    };
    let align_end = match v_first {
        Some(_) => line_end.min(track.times[v_last] + margin),
        None => line_end,
    };
    if align_end <= align_start {
        return false;
    }

    // 窗口内重新取帧
    let fidx: Vec<usize> = fidx
        .into_iter()
        .filter(|&i| track.times[i] >= align_start && track.times[i] < align_end)
        .collect();
    if fidx.len() < 2 {
        return false;
    }
    let total_dur = align_end - align_start;

    // 行内谱通量自适应阈值
    let flux_mean = if !track.flux.is_empty() {
        let sum: f32 = fidx
            .iter()
            .filter_map(|&i| track.flux.get(i).copied())
            .sum();
        sum / fidx.len() as f32
    } else {
        0.0
    };

    let weights: Vec<f32> = units.iter().map(|u| u.weight).collect();
    let total_w: f32 = weights.iter().sum::<f32>().max(1.0);
    let ideal_unit = total_dur / total_w;
    let min_dur = (params.min_token_duration_ms / 1000.0)
        .min(ideal_unit)
        .max(0.02);

    // 边界得分: 纯声学证据, 与音高解耦
    let boundary_score = |fi: usize| -> f32 {
        let mut s = 0.0f32;
        let cur_voiced = fi < track.midis.len() && !track.midis[fi].is_nan();
        let prev_voiced = fi > 0 && !track.midis[fi - 1].is_nan();
        if prev_voiced && !cur_voiced {
            s += 1.5; // 发声结束
        }
        if !prev_voiced && cur_voiced {
            s += 1.0; // 起音
        }
        if fi > 0 && track.rms.len() > fi {
            let r0 = track.rms[fi - 1];
            let r1 = track.rms[fi];
            if r1 < r0 * 0.6 {
                s += 1.0; // 能量谷
            }
            if r1 > r0 * 1.6 {
                s += 0.8; // 能量上升 (新辅音起音)
            }
        }
        if fi > 0 && flux_mean > 0.0 {
            if let Some(&f) = track.flux.get(fi) {
                if f > flux_mean * 1.5 {
                    s += 2.0; // 谱通量峰: 音素/音节转换的强证据
                } else if f > flux_mean {
                    s += 0.8;
                }
            }
        }
        s
    };

    let time_at = |j: usize| track.times[fidx[j]];
    let inf = f32::INFINITY;
    let m = fidx.len();
    let last_k = n_units - 2;

    let mut dp = vec![vec![inf; m]; n_units];
    let mut parent = vec![vec![usize::MAX; m]; n_units];

    let seg_cost = |prev_t: f32, cur_t: f32, score: f32, w: f32| -> f32 {
        let dur = cur_t - prev_t;
        if dur < min_dur {
            return inf;
        }
        let ideal = (w * ideal_unit).max(min_dur);
        let dur_cost = (dur - ideal).abs() / ideal.max(1e-3);
        let bnd_cost = -score * 0.8;
        dur_cost + bnd_cost
    };

    for j in 0..m {
        let t1 = time_at(j);
        if t1 <= align_start {
            continue;
        }
        dp[0][j] = seg_cost(align_start, t1, boundary_score(fidx[j]), weights[0]);
    }

    for k in 1..=last_k {
        for j in k..m {
            let t_j = time_at(j);
            let mut best = inf;
            let mut bp = usize::MAX;
            for p in (k - 1)..j {
                if dp[k - 1][p] == inf {
                    continue;
                }
                let t_p = time_at(p);
                let c = dp[k - 1][p] + seg_cost(t_p, t_j, boundary_score(fidx[j]), weights[k]);
                if c < best {
                    best = c;
                    bp = p;
                }
            }
            dp[k][j] = best;
            parent[k][j] = bp;
        }
    }

    let mut best_cost = inf;
    let mut best_j = usize::MAX;
    for j in last_k..m {
        if dp[last_k][j] == inf {
            continue;
        }
        let t_j = time_at(j);
        let c = dp[last_k][j] + seg_cost(t_j, align_end, 0.0, weights[n_units - 1]);
        if c < best_cost {
            best_cost = c;
            best_j = j;
        }
    }
    if best_j == usize::MAX {
        return false;
    }

    // 回溯 unit 边界
    let mut b_after = vec![usize::MAX; n_units];
    let mut cur = best_j;
    for k in (0..=last_k).rev() {
        b_after[k] = cur;
        cur = parent[k][cur];
    }

    let mut unit_times: Vec<(f32, f32)> = Vec::with_capacity(n_units);
    let mut prev_time = align_start;
    for k in 0..n_units {
        let t_end = if k == n_units - 1 {
            align_end
        } else {
            time_at(b_after[k])
        };
        unit_times.push((prev_time, t_end));
        prev_time = t_end;
    }

    // 回填 mora 时间
    for (u, &(us, ue)) in units.iter().zip(&unit_times) {
        if let Some(mi) = u.mora_idx {
            if let Some(m) = line.moras.get_mut(mi) {
                m.start_time = Some(us);
                m.end_time = Some(ue);
                m.confidence = if u.known { 0.7 } else { 0.3 };
            }
        }
    }
    // 回填 token 时间:
    //   - token 的 char 区间相交的 moras → [min start, max end]
    //     (coarse span 的 moras 会同时覆盖多个 token, 各 token 取自己的区间)
    //   - 无 mora 覆盖的 token → fallback unit 的时间
    let n_tokens = line.tokens.len();
    let mut token_first = vec![None; n_tokens];
    let mut token_last = vec![None; n_tokens];
    let mut token_has_mora = vec![false; n_tokens];
    for (mi, mora) in line.moras.iter().enumerate() {
        let Some(&(us, ue)) = unit_times.get(mi) else {
            continue;
        };
        let _ = mi;
        for (ti, token) in line.tokens.iter().enumerate() {
            if mora.char_start < token.char_end && token.char_start < mora.char_end {
                token_has_mora[ti] = true;
                let f = &mut token_first[ti];
                if f.map_or(true, |v| us < v) {
                    *f = Some(us);
                }
                let l = &mut token_last[ti];
                if l.map_or(true, |v| ue > v) {
                    *l = Some(ue);
                }
            }
        }
    }
    let mut token_known_w = vec![0.0f32; n_tokens];
    let mut token_total_w = vec![0.0f32; n_tokens];
    for (u, _) in units.iter().zip(unit_times.iter()) {
        if u.mora_idx.is_none() && u.token_idx < n_tokens {
            let ti = u.token_idx;
            token_total_w[ti] += u.weight;
        }
    }
    for (u, &(us, ue)) in units.iter().zip(&unit_times) {
        if u.mora_idx.is_none() && u.token_idx < n_tokens && !token_has_mora[u.token_idx] {
            token_known_w[u.token_idx] += u.weight; // unknown 权重, 占位使 confidence=0
        }
    }
    for (ti, token) in line.tokens.iter_mut().enumerate() {
        let times = if token_has_mora[ti] {
            match (token_first[ti], token_last[ti]) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            }
        } else {
            // fallback unit: 从 unit_times 里找该 token 的先验 unit
            units.iter()
                .zip(unit_times.iter())
                .find(|(u, _)| u.mora_idx.is_none() && u.token_idx == ti)
                .map(|(_, &(s, e))| (s, e))
        };
        if let Some((s, e)) = times {
            token.start_time = Some(s);
            token.end_time = Some(e);
            token.alignment_source = Some(AlignmentSource::MoraDp);
            token.alignment_confidence = if token_has_mora[ti] {
                1.0
            } else {
                0.0 // 无读音 → 先验, 置信度 0
            };
        }
    }
    let _ = token_known_w;
    let _ = token_total_w;
    true
}

// ── Token ↔ pitch 绑定 ────────────────────────────────────

const MIN_NOTE_FRAMES: usize = 5;
const DOMINANT_NOTE_RATIO: f32 = 0.65;

/// 绑定 pitch 到每个 token (软评分, many-to-many) 并选 primary_note。
///
/// - 候选生成: 与 token 窗口 (含 30ms 容差, 仅用于发现) 有重叠的 NoteEvent;
/// - 评分: score = 0.55*overlap_ratio_token + 0.30*overlap_ratio_note + 0.15*confidence;
///   评分只使用无容差的真实重叠, 相邻音不会凭空继承;
/// - 全部候选保留在 pitch_notes (转音 melisma 可见), primary_note 仅是紧凑显示的代表;
/// - 无候选时给出 UnpitchedReason (区分物理无声与对齐失败);
/// - 旧工程无 NoteEvent 时回退帧级分段。
///
/// - 候选发现与正式准入拆开: 零真实重叠的邻近事件绝不产生正式 badge;
/// - 动态准入门槛: overlap >= clamp(0.25*min(token,note), 10ms, 30ms)
///   且 (r_token >= 0.12 或 r_note >= 0.12);
/// - 优先在 mora 层 many-to-many 绑定 (NoteBinding 真实落盘), 再聚合到显示 token;
/// - primary = 真实覆盖时长 x 置信度 x 稳定度 最大的已准入事件,
///   不再使用重叠几何软分的第一名;
/// - 无准入候选时给出 UnpitchedReason (区分物理无声与对齐失败);
/// - 旧工程无 NoteEvent 时回退帧级分段。
pub fn bind_pitch_to_tokens(
    lines: &mut [LyricLine],
    pitch_track: &PitchTrack,
    confidence_threshold: f32,
    note_tracking: &NoteTrackingParams,
) {
    if pitch_track.times.is_empty() {
        return;
    }
    let min_dur = note_tracking.min_note_duration_ms / 1000.0;
    let has_events = !pitch_track.note_events.is_empty();

    for line in lines.iter_mut() {
        if !has_events {
            // 旧工程回退: 帧级分段
            for token in &mut line.tokens {
                let (t_start, t_end) = match (token.start_time, token.end_time) {
                    (Some(s), Some(e)) => (s, e),
                    _ => {
                        token.unpitched_reason = Some(UnpitchedReason::AlignmentMissing);
                        continue;
                    }
                };
                token.pitch_notes =
                    frame_segment_notes(pitch_track, t_start, t_end, confidence_threshold);
                token.primary_note =
                    select_primary_note(&token.pitch_notes, min_dur, confidence_threshold);
                if token.primary_note.is_none() {
                    token.unpitched_reason = Some(UnpitchedReason::NoOverlappingNote);
                }
            }
            continue;
        }

        // canonical 优先: GAME/canonical musical_notes; 旧工程回退 legacy note_events
        let mora_timing = line.moras.iter().any(|m| m.start_time.is_some());
        if !pitch_track.musical_notes.is_empty() {
            if mora_timing {
                bind_line_at_mora_level(line, &pitch_track.musical_notes, pitch_track, confidence_threshold);
            } else {
                bind_line_at_token_level(line, &pitch_track.musical_notes, pitch_track, confidence_threshold);
            }
        } else if mora_timing {
            bind_line_at_mora_level(line, &pitch_track.note_events, pitch_track, confidence_threshold);
        } else {
            bind_line_at_token_level(line, &pitch_track.note_events, pitch_track, confidence_threshold);
        }
    }
}

/// 动态准入门槛 (任务书 4.2)
fn admission_min_overlap(window: f32, note: f32) -> f32 {
    (0.25 * window.min(note)).clamp(0.010, 0.030)
}

/// 单窗口的候选准入: 返回 (event_idx, 真实重叠, r_token, r_note, score)。
/// 泛型: canonical MusicalNoteEvent 与 legacy NoteEvent 通用。
fn admit_events_for_window<T: NoteWindow>(
    events: &[T],
    w_start: f32,
    w_end: f32,
) -> Vec<(usize, f32, f32, f32, f32)> {
    let window = (w_end - w_start).max(1e-3);
    let mut out = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        let (ev_start, ev_end) = ev.window();
        let rs = ev_start.max(w_start);
        let re_ = ev_end.min(w_end);
        let ov = (re_ - rs).max(0.0);
        if ov <= 0.0 {
            continue; // 零真实重叠: 不准入 (debug 近邻建议不进入正式绑定)
        }
        let note_dur = (ev_end - ev_start).max(1e-3);
        let r_tok = ov / window;
        let r_note = ov / note_dur;
        let min_ov = admission_min_overlap(window, note_dur);
        if ov < min_ov {
            continue;
        }
        if r_tok < 0.12 && r_note < 0.12 {
            continue;
        }
        let score = 0.55 * r_tok + 0.30 * r_note + 0.15 * ev.confidence();
        out.push((i, ov, r_tok, r_note, score));
    }
    out
}

/// primary 选择: 真实覆盖时长 x 置信度 x 稳定度 (4.4)
fn primary_score<T: NoteWindow>(ev: &T, ov_real: f32) -> f32 {
    ov_real * ev.confidence() * ev.stability()
}

/// mora 层绑定 (优先): mora 时间来自 DP 对齐, 绑定结果落盘到
/// mora.note_bindings, 再聚合出 token 的 pitch_notes / primary_note。
fn bind_line_at_mora_level<T: NoteWindow>(
    line: &mut LyricLine,
    events: &[T],
    pitch_track: &PitchTrack,
    confidence_threshold: f32,
) {
    // 1. 每个 mora 独立准入
    for mora in line.moras.iter_mut() {
        mora.note_bindings.clear();
        let (Some(ms), Some(me)) = (mora.start_time, mora.end_time) else {
            continue;
        };
        let cands = admit_events_for_window(events, ms, me);
        for (idx, ov, r_tok, r_note, score) in cands {
            mora.note_bindings.push(crate::models::NoteBinding {
                note_event_index: idx,
                overlap_ms: ov * 1000.0,
                overlap_ratio_token: r_tok,
                overlap_ratio_note: r_note,
                score,
            });
        }
    }

    // 2. 聚合到 token: 取其 char 区间覆盖的 moras 的绑定并集
    let mora_bindings: Vec<Vec<crate::models::NoteBinding>> =
        line.moras.iter().map(|m| m.note_bindings.clone()).collect();
    let mora_spans: Vec<(usize, usize)> = line
        .moras
        .iter()
        .map(|m| (m.char_start, m.char_end))
        .collect();

    for token in line.tokens.iter_mut() {
        if token.start_time.is_none() || token.end_time.is_none() {
            token.unpitched_reason = Some(UnpitchedReason::AlignmentMissing);
            continue;
        }
        // token 覆盖的 moras
        let mut event_best: std::collections::HashMap<usize, f32> =
            std::collections::HashMap::new();
        for (mi, (ms, me)) in mora_spans.iter().enumerate() {
            if token.char_start >= *me || *ms >= token.char_end {
                continue;
            }
            for b in &mora_bindings[mi] {
                let e = event_best.entry(b.note_event_index).or_insert(0.0);
                *e += b.overlap_ms / 1000.0;
            }
        }
        // 无 mora 覆盖的 token (纯汉字/拉丁) 或部分覆盖: 并入 token 级准入。
        // 同一事件在两层都有时取 max (避免重复计入重叠)。
        let t_start0 = token.start_time.unwrap();
        let t_end0 = token.end_time.unwrap();
        for (idx, ov, _, _, _) in
            admit_events_for_window(events, t_start0, t_end0)
        {
            let e = event_best.entry(idx).or_insert(0.0);
            if *e < ov {
                *e = ov;
            }
        }
        if event_best.is_empty() {
            token.pitch_notes.clear();
            token.primary_note = None;
            token.unpitched_reason = Some(unpitched_reason_for(
                pitch_track,
                token.start_time.unwrap(),
                token.end_time.unwrap(),
                confidence_threshold,
            ));
            continue;
        }
        // 按 token 窗口内的真实覆盖构造 PitchNote, primary 按覆盖x置信x稳定
        let t_start = token.start_time.unwrap();
        let t_end = token.end_time.unwrap();
        let mut notes: Vec<PitchNote> = Vec::new();
        let mut scores: Vec<f32> = Vec::new();
        for (idx, total_ov) in event_best {
            let ev = &events[idx];
            let (ev_start, ev_end) = ev.window();
            let rs = ev_start.max(t_start);
            let re_ = ev_end.min(t_end);
            notes.push(note_window_to_pitch_note(ev, rs, re_));
            scores.push(primary_score(ev, total_ov));
        }
        // 详细模式的 → 箭头必须按时间顺序 (HashMap 迭代无序, 任务书 §9);
        // primary 按覆盖x置信x稳定评分独立重选, 不依赖排序前下标
        let mut order: Vec<usize> = (0..notes.len()).collect();
        order.sort_by(|&a, &b| notes[a].start_time.partial_cmp(&notes[b].start_time).unwrap_or(Ordering::Equal));
        let sorted_notes: Vec<PitchNote> = order.iter().map(|&i| notes[i].clone()).collect();
        let sorted_scores: Vec<f32> = order.iter().map(|&i| scores[i]).collect();
        token.pitch_notes = sorted_notes;
        token.primary_note = sorted_scores
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
            .map(|(i, _)| token.pitch_notes[i].clone());
        token.unpitched_reason = None;
    }
}

/// token 层直接绑定 (无 mora 时间时: 纯汉字/拉丁/中文行)
fn bind_line_at_token_level<T: NoteWindow>(
    line: &mut LyricLine,
    events: &[T],
    pitch_track: &PitchTrack,
    confidence_threshold: f32,
) {
    for token in line.tokens.iter_mut() {
        let (t_start, t_end) = match (token.start_time, token.end_time) {
            (Some(s), Some(e)) => (s, e),
            _ => {
                token.unpitched_reason = Some(UnpitchedReason::AlignmentMissing);
                continue;
            }
        };
        let cands = admit_events_for_window(events, t_start, t_end);
        if cands.is_empty() {
            token.pitch_notes.clear();
            token.primary_note = None;
            token.unpitched_reason = Some(unpitched_reason_for(
                pitch_track,
                t_start,
                t_end,
                confidence_threshold,
            ));
            continue;
        }
        let mut notes: Vec<PitchNote> = Vec::new();
        let mut scores: Vec<f32> = Vec::new();
        for (idx, ov, _, _, _) in &cands {
            let ev = &events[*idx];
            let (ev_start, ev_end) = ev.window();
            let rs = ev_start.max(t_start);
            let re_ = ev_end.min(t_end);
            notes.push(note_window_to_pitch_note(ev, rs, re_));
            scores.push(primary_score(ev, *ov));
        }
        let mut order: Vec<usize> = (0..notes.len()).collect();
        order.sort_by(|&a, &b| notes[a].start_time.partial_cmp(&notes[b].start_time).unwrap_or(Ordering::Equal));
        let sorted_notes: Vec<PitchNote> = order.iter().map(|&i| notes[i].clone()).collect();
        let sorted_scores: Vec<f32> = order.iter().map(|&i| scores[i]).collect();
        token.pitch_notes = sorted_notes;
        token.primary_note = sorted_scores
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(Ordering::Equal))
            .map(|(i, _)| token.pitch_notes[i].clone());
        token.unpitched_reason = None;
    }
}

/// 无音高原因判定: 物理无声 (静音/无声化) 与算法丢失分开
fn unpitched_reason_for(
    track: &PitchTrack,
    t_start: f32,
    t_end: f32,
    confidence_threshold: f32,
) -> UnpitchedReason {
    let mut voiced = 0usize;
    let mut confident = 0usize;
    for (i, &t) in track.times.iter().enumerate() {
        if t < t_start {
            continue;
        }
        if t >= t_end {
            break;
        }
        if !track.midis[i].is_nan() {
            voiced += 1;
            if track.confidences.get(i).copied().unwrap_or(0.0) >= confidence_threshold {
                confident += 1;
            }
        }
    }
    if voiced == 0 {
        UnpitchedReason::NoVoicing
    } else if confident == 0 {
        UnpitchedReason::LowPitchConfidence
    } else {
        UnpitchedReason::NoOverlappingNote
    }
}

/// 泛型音符 → PitchNote (binder 展示层结构)
fn note_window_to_pitch_note<T: NoteWindow>(ev: &T, start: f32, end: f32) -> PitchNote {
    let dur = (end - start).max(0.0);
    let f = ev.midi_float();
    PitchNote {
        start_time: start,
        end_time: end,
        median_midi: f,
        mean_midi: f,
        rounded_midi: ev.midi_rounded(),
        confidence_mean: ev.confidence(),
        point_count: (dur / 0.01).round().max(1.0) as usize,
    }
}

fn note_event_to_pitch_note(ev: &NoteEvent, start: f32, end: f32) -> PitchNote {
    let dur = (end - start).max(0.0);
    PitchNote {
        start_time: start,
        end_time: end,
        median_midi: ev.midi as f32,
        mean_midi: ev.midi as f32,
        rounded_midi: ev.midi,
        confidence_mean: ev.confidence,
        point_count: (dur / 0.01).round().max(1.0) as usize,
    }
}

/// 帧级分段 (无 NoteEvent 时的回退路径)
fn frame_segment_notes(
    pitch_track: &PitchTrack,
    c_start: f32,
    c_end: f32,
    confidence_threshold: f32,
) -> Vec<PitchNote> {
    let mut seg_times = Vec::new();
    let mut seg_midis = Vec::new();
    let mut seg_conf = Vec::new();
    for i in 0..pitch_track.times.len() {
        let t = pitch_track.times[i];
        if t < c_start {
            continue;
        }
        if t >= c_end {
            break;
        }
        let m = pitch_track.midis[i];
        let c = pitch_track.confidences[i];
        if c >= confidence_threshold && m.is_finite() && !m.is_nan() {
            seg_times.push(t);
            seg_midis.push(m);
            seg_conf.push(c);
        }
    }
    segment_pitch_notes(&seg_times, &seg_midis, &seg_conf, c_start, c_end)
}

/// 从 NoteEvent / 帧分段结果中选择主音:
/// score = voiced_duration × mean_confidence，剔除非常短、低 confidence 的候选。
pub fn select_primary_note(
    notes: &[PitchNote],
    min_duration: f32,
    min_confidence: f32,
) -> Option<PitchNote> {
    let filtered = notes
        .iter()
        .filter(|n| {
            (n.end_time - n.start_time) >= min_duration && n.confidence_mean >= min_confidence
        })
        .max_by(|a, b| score_cmp(a, b));
    match filtered {
        Some(n) => Some(n.clone()),
        None => notes.iter().max_by(|a, b| score_cmp(a, b)).cloned(),
    }
}

fn score_cmp(a: &PitchNote, b: &PitchNote) -> Ordering {
    let sa = (a.end_time - a.start_time) * a.confidence_mean;
    let sb = (b.end_time - b.start_time) * b.confidence_mean;
    sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
}

fn segment_pitch_notes(
    times: &[f32],
    midis: &[f32],
    confidences: &[f32],
    token_start: f32,
    token_end: f32,
) -> Vec<PitchNote> {
    if midis.len() < 2 {
        return Vec::new();
    }

    // 检查 dominant note
    let labels: Vec<i32> = midis.iter().map(|&m| m.round() as i32).collect();
    let mut label_counts: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for &l in &labels {
        *label_counts.entry(l).or_insert(0) += 1;
    }
    let (dominant_label, dominant_count) = label_counts
        .iter()
        .max_by_key(|(_, &c)| c)
        .map(|(&l, &c)| (l, c))
        .unwrap();
    let dominant_ratio = dominant_count as f32 / labels.len() as f32;

    if dominant_ratio >= DOMINANT_NOTE_RATIO {
        let core_midis: Vec<f32> = midis
            .iter()
            .filter(|&&m| (m - dominant_label as f32).abs() <= 0.75)
            .copied()
            .collect();
        let core_conf: Vec<f32> = midis
            .iter()
            .zip(confidences.iter())
            .filter(|(m, _)| (**m - dominant_label as f32).abs() <= 0.75)
            .map(|(_, c)| *c)
            .collect();
        let core_midis = if core_midis.is_empty() {
            midis.to_vec()
        } else {
            core_midis
        };
        let core_conf = if core_conf.is_empty() {
            confidences.to_vec()
        } else {
            core_conf
        };
        return vec![make_pitch_note(
            token_start,
            token_end,
            &core_midis,
            &core_conf,
            core_midis.len(),
        )];
    }

    // 按 label run 分段
    let cleaned_labels = remove_short_label_runs(&labels);
    let runs = label_runs(&cleaned_labels);
    let hop = infer_hop_seconds(times);
    let mut notes = Vec::new();
    for (start_idx, end_idx, _label) in &runs {
        let run_midis = &midis[*start_idx..*end_idx];
        let run_conf = &confidences[*start_idx..*end_idx];
        if run_midis.len() < 2 {
            continue;
        }
        let note_start = token_start.max(times[*start_idx]);
        let note_end = token_end.min(times[*end_idx - 1] + hop);
        notes.push(make_pitch_note(
            note_start,
            note_end,
            run_midis,
            run_conf,
            run_midis.len(),
        ));
    }
    merge_adjacent_notes(notes)
}

#[allow(clippy::needless_range_loop)] // run 填充按索引定位
fn remove_short_label_runs(labels: &[i32]) -> Vec<i32> {
    let mut cleaned = labels.to_vec();
    let runs = label_runs(&cleaned);
    for i in 0..runs.len() {
        let (start, end, _label) = runs[i];
        if end - start >= MIN_NOTE_FRAMES {
            continue;
        }
        let prev_label = if i > 0 { Some(runs[i - 1].2) } else { None };
        let next_label = if i + 1 < runs.len() {
            Some(runs[i + 1].2)
        } else {
            None
        };
        let fill = match (prev_label, next_label) {
            (Some(p), Some(n)) if p == n => p,
            (Some(p), Some(n)) => {
                let prev_len = runs[i - 1].1 - runs[i - 1].0;
                let next_len = runs[i + 1].1 - runs[i + 1].0;
                if prev_len >= next_len {
                    p
                } else {
                    n
                }
            }
            (Some(p), None) => p,
            (None, Some(n)) => n,
            _ => _label,
        };
        for idx in start..end {
            cleaned[idx] = fill;
        }
    }
    cleaned
}

fn label_runs(labels: &[i32]) -> Vec<(usize, usize, i32)> {
    if labels.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for i in 1..=labels.len() {
        if i == labels.len() || labels[i] != labels[start] {
            runs.push((start, i, labels[start]));
            start = i;
        }
    }
    runs
}

fn infer_hop_seconds(times: &[f32]) -> f32 {
    if times.len() < 2 {
        return 0.01;
    }
    let mut diffs: Vec<f32> = times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|&d| d > 0.0)
        .collect();
    if diffs.is_empty() {
        return 0.01;
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    diffs[diffs.len() / 2]
}

fn make_pitch_note(
    start_time: f32,
    end_time: f32,
    midis: &[f32],
    confidences: &[f32],
    point_count: usize,
) -> PitchNote {
    let filtered = iqr_filter(midis);
    let median_m = median(&filtered);
    let mean_m = mean(&filtered);
    PitchNote {
        start_time,
        end_time,
        median_midi: median_m,
        mean_midi: mean_m,
        rounded_midi: median_m.round() as i32,
        confidence_mean: mean(confidences),
        point_count,
    }
}

fn iqr_filter(values: &[f32]) -> Vec<f32> {
    if values.len() < 4 {
        return values.to_vec();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p25_idx = (sorted.len() as f32 * 0.25) as usize;
    let p75_idx = (sorted.len() as f32 * 0.75) as usize;
    let p25 = sorted[p25_idx];
    let p75 = sorted[p75_idx];
    let iqr = p75 - p25;
    if iqr == 0.0 {
        return values.to_vec();
    }
    let lo = p25 - 0.5 * iqr;
    let hi = p75 + 0.5 * iqr;
    let filtered: Vec<f32> = values
        .iter()
        .filter(|&&v| v >= lo && v <= hi)
        .copied()
        .collect();
    if filtered.is_empty() {
        values.to_vec()
    } else {
        filtered
    }
}

fn merge_adjacent_notes(notes: Vec<PitchNote>) -> Vec<PitchNote> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut merged = vec![notes[0].clone()];
    for note in notes.into_iter().skip(1) {
        let prev = merged.last_mut().unwrap();
        if prev.rounded_midi == note.rounded_midi {
            let total = prev.point_count + note.point_count;
            let w1 = prev.point_count as f32 / total as f32;
            let w2 = note.point_count as f32 / total as f32;
            prev.end_time = note.end_time;
            prev.median_midi = prev.median_midi * w1 + note.median_midi * w2;
            prev.mean_midi = prev.mean_midi * w1 + note.mean_midi * w2;
            prev.confidence_mean = prev.confidence_mean * w1 + note.confidence_mean * w2;
            prev.rounded_midi = prev.median_midi.round() as i32;
            prev.point_count = total;
        } else {
            merged.push(note);
        }
    }
    merged
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut s = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    if n.is_multiple_of(2) {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    } else {
        s[n / 2]
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}
