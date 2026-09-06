// mora → phoneme 序列
//
// 为 P3 forced alignment 提供统一输入 (constrained CTC/Viterbi 的已知序列)。
// 规则表覆盖: 清/浊/半浊音、拗音 (きゃ=k y a)、外来音组合 (ファ=f a, ティ=t i)、
// 促音 (っ=cl)、拨音 (ん=N)、长音符 (ー=:)。
// 这不是完整的音系同化模型, 但足够作为 forced aligner 的标签序列。

use crate::japanese::mora;

/// 单个假名 (或假名组合) 的辅音部分
fn consonant_of(base: char) -> Option<&'static str> {
    Some(match base {
        'か' | 'き' | 'く' | 'け' | 'こ' => "k",
        'が' | 'ぎ' | 'ぐ' | 'げ' | 'ご' => "g",
        'さ' | 'す' | 'せ' | 'そ' => "s",
        'し' => "sh",
        'ざ' | 'ず' | 'ぜ' | 'ぞ' => "z",
        'じ' => "j",
        'た' | 'て' | 'と' => "t",
        'ち' => "ch",
        'つ' => "ts",
        'だ' | 'で' | 'ど' => "d",
        'な' | 'に' | 'ぬ' | 'ね' | 'の' => "n",
        'は' | 'へ' | 'ほ' => "h",
        'ひ' => "h",
        'ふ' => "f",
        'ば' | 'び' | 'ぶ' | 'べ' | 'ぼ' => "b",
        'ぱ' | 'ぴ' | 'ぷ' | 'ぺ' | 'ぽ' => "p",
        'ま' | 'み' | 'む' | 'め' | 'も' => "m",
        'や' | 'ゆ' | 'よ' => "y",
        'ら' | 'り' | 'る' | 'れ' | 'ろ' => "r",
        'わ' => "w",
        'ヴ' => "v",
        _ => return None,
    })
}

fn vowel_of(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' | 'か' | 'さ' | 'た' | 'な' | 'は' | 'ま' | 'や' | 'ら' | 'わ' | 'が' | 'ざ'
        | 'だ' | 'ば' | 'ぱ' | 'ヴ' => "a",
        'い' | 'き' | 'し' | 'ち' | 'に' | 'ひ' | 'み' | 'り' | 'ぎ' | 'じ' | 'ぢ' | 'び'
        | 'ぴ' => "i",
        'う' | 'く' | 'す' | 'つ' | 'ぬ' | 'ふ' | 'む' | 'ゆ' | 'る' | 'ぐ' | 'ず' | 'づ'
        | 'ぶ' | 'ぷ' => "u",
        'え' | 'け' | 'せ' | 'て' | 'ね' | 'へ' | 'め' | 'れ' | 'げ' | 'ぜ' | 'で' | 'べ'
        | 'ぺ' => "e",
        'お' | 'こ' | 'そ' | 'と' | 'の' | 'ほ' | 'も' | 'よ' | 'ろ' | 'ご' | 'ぞ' | 'ど'
        | 'ぼ' | 'ぽ' => "o",
        _ => return None,
    })
}

/// 小假名 → 母音 (ぁ=a ゃ=a ... ェ=e ォ=o)
fn small_vowel(c: char) -> Option<&'static str> {
    Some(match c {
        'ぁ' | 'ャ' | 'ゃ' => "a",
        'ぃ' | 'ィ' => "i",
        'ぅ' | 'ュ' | 'ゅ' => "u",
        'ぇ' | 'ェ' => "e",
        'ぉ' | 'ョ' | 'ょ' => "o",
        _ => return None,
    })
}

/// 片假名 → 平假名 (用于查表)
fn katakana_to_hiragana(c: char) -> char {
    if ('\u{30A1}'..='\u{30F6}').contains(&c) {
        char::from_u32(c as u32 - 0x60).unwrap_or(c)
    } else {
        c
    }
}

/// 一个 mora 的假名 → phoneme 序列
pub fn mora_to_phonemes(kana: &str) -> Vec<String> {
    let chars: Vec<char> = kana.chars().map(katakana_to_hiragana).collect();
    match chars.as_slice() {
        [c] => {
            if *c == 'ー' {
                return vec![":".to_string()]; // 长音符: 延续前一母音
            }
            if *c == 'っ' {
                return vec!["cl".to_string()]; // 促音闭塞
            }
            if *c == 'ん' {
                return vec!["N".to_string()]; // 拨音
            }
            single_kana_phonemes(*c)
        }
        [base, attach] => {
            // base + 小書き: 拗音 (きゃ = k y a) 或外来音 (ファ = f a, ティ = t i)
            if let Some(v) = small_vowel(*attach) {
                match consonant_of(*base) {
                    Some(cons) => {
                        if is_y_small_raw(*attach) {
                            vec![cons.to_string(), "y".to_string(), v.to_string()]
                        } else {
                            // 外来音直母音: ファ = f a (不加 y)
                            vec![cons.to_string(), v.to_string()]
                        }
                    }
                    None => {
                        // 母音底 + 小書き (ウェ/ウォ 等): う行底用 w 起音,
                        // 其余 (ぁ行底) 退化为单母音核
                        if *base == 'う' || *base == 'ヴ' {
                            vec!["w".to_string(), v.to_string()]
                        } else {
                            vec![v.to_string()]
                        }
                    }
                }
            } else {
                match consonant_of(*base) {
                    Some(cons) => {
                        vec![cons.to_string(), vowel_of(*base).unwrap_or("u").to_string()]
                    }
                    None => single_kana_phonemes(*base),
                }
            }
        }
        _ => vec![],
    }
}

fn is_y_small_raw(c: char) -> bool {
    matches!(
        c,
        '\u{3083}' | '\u{3085}' | '\u{3087}' | '\u{30E3}' | '\u{30E5}' | '\u{30E7}'
    )
}

/// 单个普通假名 → 辅音 + 母音; 元音直用假名 (あ行) 只有母音
fn single_kana_phonemes(c: char) -> Vec<String> {
    match (consonant_of(c), vowel_of(c)) {
        (Some(cons), Some(v)) => vec![cons.to_string(), v.to_string()],
        (None, Some(v)) => vec![v.to_string()],
        _ => vec![],
    }
}

// ── Phonemizer abstraction (Round4 任务书 §8.5) ────────────────────
//
// 核心 alignment 不得依赖当前规则表的具体符号: 通过 trait 包装,
// 以后可替换为 OpenJTalk 兼容音素或 singing FA 自带 dictionary。

/// 日语读音 → 音素序列的抽象。实现必须无锁可共享 (Send + Sync)。
pub trait JapanesePhonemizer: Send + Sync {
    fn name(&self) -> &'static str;

    /// 把一个发音单位的读音 (假名) 展开为音素序列。
    /// 未知假名返回空序列 (调用方据此保持 Unknown, 不伪造音素)。
    fn phonemize(&self, pronunciation: &str) -> Result<Vec<PhonemeSpec>, String>;
}

/// 音素结果: 符号 + 分类 + 是否 pitch-bearing (由 pitch_policy 统一判定)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhonemeSpec {
    pub symbol: String,
    pub class: crate::models::PhonemeClass,
    pub pitch_bearing: bool,
}

/// 当前规则表实现 (短期保留; §8.5: 不要继续无限手写扩展)
pub struct RuleBasedPhonemizer;

impl JapanesePhonemizer for RuleBasedPhonemizer {
    fn name(&self) -> &'static str {
        "rule-based-phonemizer"
    }

    fn phonemize(&self, pronunciation: &str) -> Result<Vec<PhonemeSpec>, String> {
        Ok(mora::parse_kana_moras(pronunciation)
            .iter()
            .flat_map(|m| mora_to_phonemes(&m.kana))
            .map(|symbol| {
                let class = crate::japanese::pitch_policy::classify_phoneme_class(&symbol);
                PhonemeSpec {
                    symbol,
                    class,
                    pitch_bearing: crate::japanese::pitch_policy::phoneme_pitch_bearing(class),
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::super::mora;
    use super::*;

    #[test]
    fn test_basic_moras() {
        assert_eq!(mora_to_phonemes("か"), vec!["k", "a"]);
        assert_eq!(mora_to_phonemes("し"), vec!["sh", "i"]);
        assert_eq!(mora_to_phonemes("ん"), vec!["N"]);
        assert_eq!(mora_to_phonemes("っ"), vec!["cl"]);
        assert_eq!(mora_to_phonemes("ー"), vec![":"]);
    }

    #[test]
    fn test_yoon_and_foreign() {
        assert_eq!(mora_to_phonemes("きゃ"), vec!["k", "y", "a"]);
        assert_eq!(mora_to_phonemes("しゃ"), vec!["sh", "y", "a"]);
        assert_eq!(mora_to_phonemes("ファ"), vec!["f", "a"]);
        assert_eq!(mora_to_phonemes("ティ"), vec!["t", "i"]);
        assert_eq!(mora_to_phonemes("シェ"), vec!["sh", "e"]);
    }

    #[test]
    fn test_mora_sequence() {
        let seq: Vec<String> = mora::parse_kana_moras("きゃっと")
            .iter()
            .flat_map(|m| mora_to_phonemes(&m.kana))
            .collect();
        assert_eq!(seq, vec!["k", "y", "a", "cl", "t", "o"]);
    }
}
