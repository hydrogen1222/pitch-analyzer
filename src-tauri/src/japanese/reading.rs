// 日语读音 provider 抽象
//
// 优先级 (任务书 §2.4):
//   用户 override > ruby/furigana (未支持) > UniDic 词典 (P1, 未接入) > KanaOnly > heuristic
//
// Lindera + UniDic 接入说明 (P1):
//   - lindera (MIT) + UniDic 词典资源, 词典作为可选外部资源打包 (不要默认塞进主 exe,
//     见任务书 §10), 运行时通过 app_config_dir / 资源目录加载, 带 hash/版本校验
//   - 实现 JapaneseReadingProvider, 返回的 ReadingSpan 需带 byte span 与读音
//   - 歌曲特殊读法 (運命→さだめ) 通过 ReadingOverride 接口覆盖词典结果
//   - 本文件预留 override 接口与 provider trait, 词典接入不改变下游数据模型

use crate::japanese::mora;
use crate::models::ReadingSpan;

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
    spans: &mut [ReadingSpan],
    overrides: &[ReadingOverride],
    _text: &str,
) {
    for ov in overrides {
        for span in spans.iter_mut() {
            let overlaps = ov.char_start < span.char_end && span.char_start < ov.char_end;
            if overlaps {
                span.reading = ov.reading.clone();
                span.pronunciation = ov.reading.clone();
                span.confidence = 1.0;
            }
        }
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
