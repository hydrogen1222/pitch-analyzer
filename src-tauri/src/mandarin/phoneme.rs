// Pinyin → initial/final phoneme 序列 (Round4 Commit 7)
//
// FA 标签序列: [initial, final] 或单 final (零声母)。
// tone 数字只是 G2P 信息, 不参与 phoneme 化; ü 记作 v。
// final 是韵母, 即音节的 pitch-bearing nucleus。

/// 把一个带调拼音音节 (如 "wo3" / "lv4") 拆成音素序列。
/// 返回空 vec 表示无法解析 (调用方保持 Unknown)。
pub fn pinyin_to_phonemes(syllable: &str) -> Vec<String> {
    let base: String = syllable
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect::<Vec<_>>()
        .iter()
        .map(|c| {
            if *c == 'ü' {
                'v'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    let syl = base.as_str();
    if syl.is_empty() {
        return Vec::new();
    }
    // 双字母声母优先
    for double in ["zh", "ch", "sh"] {
        if syl.starts_with(double) && syl.len() > 2 {
            return vec![double.to_string(), syl[2..].to_string()];
        }
    }
    // 单字母声母
    let initials = [
        "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r", "z", "c", "s",
        "y", "w",
    ];
    for initial in initials {
        if syl.starts_with(initial) && syl.len() > initial.len() {
            return vec![initial.to_string(), syl[initial.len()..].to_string()];
        }
    }
    // 零声母: 整体为 final
    vec![syl.to_string()]
}

/// final (韵母) 的 phoneme 下标: 约定为最后一个元素。
pub fn final_phoneme_index(phonemes: &[String]) -> Option<usize> {
    if phonemes.is_empty() {
        None
    } else {
        Some(phonemes.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_initial_and_final() {
        assert_eq!(pinyin_to_phonemes("wo3"), vec!["w", "o"]);
        assert_eq!(pinyin_to_phonemes("ai4"), vec!["ai"]);
        assert_eq!(pinyin_to_phonemes("zhong1"), vec!["zh", "ong"]);
        assert_eq!(pinyin_to_phonemes("lv4"), vec!["l", "v"]);
        assert_eq!(pinyin_to_phonemes("shi3"), vec!["sh", "i"]);
        assert_eq!(pinyin_to_phonemes("yi1"), vec!["y", "i"]);
        assert_eq!(pinyin_to_phonemes("e5"), vec!["e"]);
    }

    #[test]
    fn final_is_pitch_bearing_nucleus() {
        for syl in ["wo3", "ai4", "zhong1", "e5"] {
            let ph = pinyin_to_phonemes(syl);
            let idx = final_phoneme_index(&ph).unwrap();
            assert_eq!(idx, ph.len() - 1);
        }
    }

    #[test]
    fn invalid_input_is_empty() {
        assert!(pinyin_to_phonemes("").is_empty());
    }
}
