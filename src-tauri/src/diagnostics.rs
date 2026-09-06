// Pipeline Diagnostics (Round4 Commit 1)
//
// 在不改任何算法的前提下, 对当前实际运行的对齐来源 / 语言特征 / 回退状态
// 做机器可读观测。本模块只读数据、不参与任何决策, 为 Round4 的
// Language Router / False Precision 清理提供 baseline 证据:
//
//   - summarize()          → 结构化 PipelineSummary (测试快照 / benchmark 报告)
//   - log_summary()        → stderr 一行 JSON (运行期观测)
//
// 语言观测只做脚本特征统计 (kana / 纯汉字 / 拉丁), 不是语言判定;
// 真正的 Language Router 在 Round4 Commit 2 落地。

use crate::models::{AlignmentSource, LyricLine};
use serde::Serialize;

/// 逐 token 对齐来源分布。口径: 只统计 primary_text 的 display tokens。
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct AlignmentSourceCounts {
    pub enhanced_lrc: usize,
    pub forced_align: usize,
    pub mora_dp: usize,
    pub weighted_fallback: usize,
    /// 有时间但无来源标签 (均匀分配遗留 / 旧工程数据)
    pub timed_without_source: usize,
    /// 完全没有时间的 token (对齐从未到达)
    pub unaligned: usize,
}

impl AlignmentSourceCounts {
    fn record(&mut self, source: Option<AlignmentSource>, timed: bool) {
        match source {
            Some(AlignmentSource::EnhancedLrc) => self.enhanced_lrc += 1,
            Some(AlignmentSource::ForcedAlign) => self.forced_align += 1,
            Some(AlignmentSource::MoraDp) => self.mora_dp += 1,
            Some(AlignmentSource::WeightedFallback) => self.weighted_fallback += 1,
            None => {
                if timed {
                    self.timed_without_source += 1;
                } else {
                    self.unaligned += 1;
                }
            }
        }
    }
}

/// 脚本特征观测 (不是语言判定)。Commit 2 的 Language Router 将替代此统计。
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LanguageObservation {
    /// 至少含一个假名 (平/片假名或长音符) 的行
    pub lines_with_kana: usize,
    /// 含汉字但完全无假名的行 (ambiguous: 可能是日语也可能是中文)
    pub han_only_lines: usize,
    /// 只含拉丁/数字/标点的行
    pub latin_only_lines: usize,
    /// 无可见字符的行
    pub empty_lines: usize,
}

impl LanguageObservation {
    fn record(&mut self, primary_text: &str) {
        let mut has_kana = false;
        let mut has_han = false;
        let mut has_latin = false;
        let mut has_visible = false;
        for c in primary_text.chars() {
            if c.is_whitespace() {
                continue;
            }
            has_visible = true;
            if crate::japanese::mora::is_kana_char(c) || c == '\u{30FC}' {
                has_kana = true;
            } else if is_han_char(c) {
                has_han = true;
            } else if c.is_ascii_alphanumeric() {
                has_latin = true;
            }
        }
        if !has_visible {
            self.empty_lines += 1;
        } else if has_kana {
            self.lines_with_kana += 1;
        } else if has_han {
            self.han_only_lines += 1;
        } else if has_latin {
            self.latin_only_lines += 1;
        }
    }
}

fn is_han_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

/// 当前 pipeline 的整体快照。字段全部为计数, 可直接做测试断言与 JSON 报告。
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct PipelineSummary {
    pub line_count: usize,
    pub token_count: usize,
    pub mora_count: usize,
    pub display_group_count: usize,
    pub ruby_annotation_count: usize,
    /// char 区间与任何 mora 都不相交的 token (读音未知 → 无声学单元)
    pub tokens_without_pronunciation: usize,
    pub sources: AlignmentSourceCounts,
    pub language_observation: LanguageObservation,
}

/// 汇总当前歌词状态。只读, 无副作用。
pub fn summarize(lines: &[LyricLine]) -> PipelineSummary {
    let mut summary = PipelineSummary::default();
    for line in lines {
        summary.line_count += 1;
        summary.token_count += line.tokens.len();
        summary.mora_count += line.moras.len();
        summary.display_group_count += line.reading_display_groups.len();
        summary.ruby_annotation_count += line.ruby_annotations.len();
        for token in &line.tokens {
            summary
                .sources
                .record(token.alignment_source, token.start_time.is_some());
            let covered = line
                .moras
                .iter()
                .any(|mora| token.char_start < mora.char_end && mora.char_start < token.char_end);
            if !covered {
                summary.tokens_without_pronunciation += 1;
            }
        }
        summary.language_observation.record(&line.primary_text);
    }
    summary
}

/// stderr 一行 JSON 观测。backend 传当前实际对齐后端 (None = 未配置)。
pub fn log_summary(stage: &str, backend: Option<&str>, lines: &[LyricLine]) {
    let summary = summarize(lines);
    let backend = backend.unwrap_or("mora_dp/weighted_fallback (no FA backend)");
    let json = serde_json::to_string(&summary).unwrap_or_else(|_| "{}".to_string());
    eprintln!("[pipeline-summary] stage={stage} backend={backend} {json}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{LyricLine, LyricToken};

    fn line_with_tokens(text: &str, source: Option<AlignmentSource>, timed: bool) -> LyricLine {
        let mut line = LyricLine::new(
            text.to_string(),
            text.to_string(),
            Vec::new(),
            Some(1.0),
            Some(2.0),
        );
        for (i, ch) in text.chars().enumerate() {
            let mut token = LyricToken::new(ch.to_string(), i, i + 1);
            if timed {
                token.start_time = Some(1.0);
                token.end_time = Some(2.0);
            }
            token.alignment_source = source;
            line.tokens.push(token);
        }
        line
    }

    #[test]
    fn source_counts_classify_each_state() {
        let lines = vec![
            line_with_tokens("え", Some(AlignmentSource::EnhancedLrc), true),
            line_with_tokens("ふ", Some(AlignmentSource::ForcedAlign), true),
            line_with_tokens("ぁ", Some(AlignmentSource::MoraDp), true),
            line_with_tokens("れ", Some(AlignmentSource::WeightedFallback), true),
            line_with_tokens("ん", None, true),
            line_with_tokens("て", None, false),
        ];
        let counts = summarize(&lines).sources;
        assert_eq!(counts.enhanced_lrc, 1);
        assert_eq!(counts.forced_align, 1);
        assert_eq!(counts.mora_dp, 1);
        assert_eq!(counts.weighted_fallback, 1);
        assert_eq!(counts.timed_without_source, 1);
        assert_eq!(counts.unaligned, 1);
    }

    #[test]
    fn language_observation_separates_kana_han_latin() {
        let lines = vec![
            line_with_tokens("きみ", None, false),
            line_with_tokens("未来", None, false),
            line_with_tokens("Hello", None, false),
            line_with_tokens("　", None, false),
        ];
        let obs = summarize(&lines).language_observation;
        assert_eq!(obs.lines_with_kana, 1);
        assert_eq!(obs.han_only_lines, 1);
        assert_eq!(obs.latin_only_lines, 1);
        assert_eq!(obs.empty_lines, 1);
    }
}
