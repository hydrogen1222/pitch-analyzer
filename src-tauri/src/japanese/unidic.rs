// UniDic 日语形态与发音分析实现
//
// 使用 Lindera + UniDic 词典提取日语句子的词素、汉字真实读音/发音 (pronunciation)。
//
// 特性:
//   - 优先提取发音 (pronunciation / 発音) 字段; 若缺失则使用读音 (reading / 読み)
//   - 片假名自动转为平假名以匹配下游 mora 解析器
//   - byte offset 转换为 char offset, 构建精确定位的 ReadingSpan
//   - 结合 ReadingOverride: 歌曲特殊读音 (如 運命→さだめ) 优先覆盖词典结果

use crate::japanese::mora;
use crate::japanese::reading::JapaneseReadingProvider;
use crate::models::ReadingSpan;
use lindera::dictionary::{resolve_embedded_loader, DictionaryKind};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use std::sync::OnceLock;

/// UniDic 词典全局单例 Segmenter
static UNIDIC_SEGMENTER: OnceLock<Result<Segmenter, String>> = OnceLock::new();

fn get_segmenter() -> Result<&'static Segmenter, &'static str> {
    let res = UNIDIC_SEGMENTER.get_or_init(|| {
        let loader = resolve_embedded_loader(DictionaryKind::UniDic)
            .map_err(|e| format!("获取 UniDic 字典加载器失败: {}", e))?;
        let dictionary = loader
            .load()
            .map_err(|e| format!("加载 UniDic 字典数据失败: {}", e))?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        Ok(segmenter)
    });
    match res {
        Ok(s) => Ok(s),
        Err(e) => Err(e.as_str()),
    }
}

/// 片假名转平假名 (U+30A1..=U+30F6 -> U+3041..=U+3096)
pub fn katakana_to_hiragana(text: &str) -> String {
    text.chars()
        .map(|c| {
            if ('\u{30A1}'..='\u{30F6}').contains(&c) {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

pub struct LinderaUnidicProvider;

impl JapaneseReadingProvider for LinderaUnidicProvider {
    fn name(&self) -> &'static str {
        "lindera-unidic"
    }

    fn analyze(&self, text: &str) -> Result<Vec<ReadingSpan>, String> {
        let segmenter = get_segmenter().map_err(|e| e.to_string())?;

        let mut tokens = segmenter
            .segment(std::borrow::Cow::Borrowed(text))
            .map_err(|e| format!("UniDic 分词失败: {}", e))?;

        // byte offset -> char offset 查表
        let char_of_byte: Vec<usize> = {
            let mut map = Vec::with_capacity(text.len() + 1);
            for (ci, (b, _)) in text.char_indices().enumerate() {
                map.resize(b, ci);
                map.push(ci);
            }
            map.resize(text.len() + 1, text.chars().count());
            map
        };

        let mut spans = Vec::new();
        let mut mora_offset = 0usize;

        for token in &mut tokens {
            let surface = token.surface.to_string();
            let surface_is_kana = surface
                .chars()
                .all(|c| mora::is_kana_char(c) || c == '\u{30FC}');
            let byte_start = token.byte_start;
            let byte_end = token.byte_end;
            let char_start = char_of_byte.get(byte_start).copied().unwrap_or(0);
            let char_end = char_of_byte
                .get(byte_end)
                .copied()
                .unwrap_or(char_start + surface.chars().count());

            // UniDic features:
            // POS(0..5), lemma_reading(6), lemma(7), reading(8/11), pronunciation(9), ...
            let feats = token.details();
            let raw_pron = feats.get(9).copied().unwrap_or("*");
            let raw_read = feats
                .get(6)
                .or_else(|| feats.get(11))
                .or_else(|| feats.get(8))
                .copied()
                .unwrap_or("*");

            // UniDic can expose the lemma reading for inflected kana tokens
            // (`し` -> `する`, `て` -> `てる`). For an all-kana surface the
            // visible text itself is the exact pronunciation and must win.
            let pronunciation_str = if surface_is_kana {
                katakana_to_hiragana(&surface)
            } else if raw_pron != "*" && !raw_pron.is_empty() {
                katakana_to_hiragana(raw_pron)
            } else if raw_read != "*" && !raw_read.is_empty() {
                katakana_to_hiragana(raw_read)
            } else {
                String::new()
            };

            let reading_str = if surface_is_kana {
                katakana_to_hiragana(&surface)
            } else if raw_read != "*" && !raw_read.is_empty() {
                katakana_to_hiragana(raw_read)
            } else if !pronunciation_str.is_empty() {
                pronunciation_str.clone()
            } else {
                String::new()
            };

            let confidence = if surface_is_kana {
                1.0
            } else if !pronunciation_str.is_empty() {
                0.95
            } else {
                0.0
            };

            let mora_start = mora_offset;
            let phonetic = if !pronunciation_str.is_empty() {
                &pronunciation_str
            } else {
                &reading_str
            };
            if !phonetic.is_empty() {
                mora_offset += mora::parse_kana_moras(phonetic).len();
            }
            let mora_end = mora_offset;

            spans.push(ReadingSpan {
                surface: surface.clone(),
                reading: reading_str,
                pronunciation: pronunciation_str,
                char_start,
                char_end,
                display_start: char_start,
                display_end: char_end,
                mora_start,
                mora_end,
                confidence,
            });
        }

        Ok(spans)
    }
}
