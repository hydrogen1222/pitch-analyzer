// kana → mora 状态机
//
// 莫拉规则 (与显示分组无关):
//   base + 小書き (ゃゅょ / ぁぃぅぇぉ) = 1 mora   (きゃ / ファ / ティ / シェ)
//   ー = 独立 1 mora                               (スーパー = ス・ー・パ・ー)
//   っ = 独立 1 mora                               (がっこう = が・っ・こ・う)
//   ん = 独立 1 mora
//   其他假名 = 各 1 mora
//   非假名 (汉字/拉丁/标点) 不产生 mora, 原样跳过 (由上层做 unknown-mora 处理)

/// 文本侧 mora 解析结果 (char 下标相对于传入的 text)
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMora {
    pub kana: String,
    pub char_start: usize,
    pub char_end: usize,
}

fn is_small_vowel(c: char) -> bool {
    matches!(
        c,
        '\u{3041}' | '\u{3043}' | '\u{3045}' | '\u{3047}' | '\u{3049}' // ぁぃぅぇぉ
            | '\u{30A1}' | '\u{30A3}' | '\u{30A5}' | '\u{30A7}' | '\u{30A9}' // ァィゥェォ
            | '\u{FF67}'..='\u{FF6B}' // 半角ｧｨｩｪｫ
    )
}

fn is_y_small(c: char) -> bool {
    matches!(
        c,
        '\u{3083}' | '\u{3085}' | '\u{3087}' | '\u{30E3}' | '\u{30E5}' | '\u{30E7}' | '\u{FF6C}'
            ..='\u{FF6E}' // 半角ｬｭｮ
    )
}

fn is_combining_kana(c: char) -> bool {
    is_small_vowel(c) || is_y_small(c)
}

pub fn is_kana_char(c: char) -> bool {
    ('\u{3041}'..='\u{309F}').contains(&c)
        || ('\u{30A0}'..='\u{30FF}').contains(&c)
        // 半角片假名 (U+FF66..U+FF9F, 含半角长音符 ｰ U+FF70):
        // 歌词站点常输出半角片假名, 语言检测/分词必须与全角等价 (任务书 §8.4)
        || ('\u{FF66}'..='\u{FF9F}').contains(&c)
}

fn is_chionpu(c: char) -> bool {
    c == '\u{30FC}' || c == '\u{FF70}'
}

/// Unicode 规范化 (任务书 §8.4):
///   - 半角片假名 → 全角 (ｽｰﾊﾟｰ → スーパー)
///   - combining dakuten → 合成浊音 (か+U+3099 → が)
/// 使用 NFKC; 读产生点 (provider / override / ruby) 统一调用,
/// 保证 reading 文本与 mora char offset 一致。
pub fn normalize_kana(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.chars().nfkc().collect()
}

/// 把假名文本解析成 mora 序列。非假名字符被跳过 (不产生 mora)。
pub fn parse_kana_moras(text: &str) -> Vec<ParsedMora> {
    let char_list: Vec<char> = text.chars().collect();
    let mut moras: Vec<ParsedMora> = Vec::new();
    let mut ci = 0usize;
    while ci < char_list.len() {
        let c = char_list[ci];
        if !is_kana_char(c) && !is_chionpu(c) {
            ci += 1;
            continue;
        }
        let start = ci;
        let mut end = ci + 1;
        // base + 小書き (ゃゅょ/ぁぃぅぇぉ) → 1 mora
        if (is_kana_char(c) && !is_combining_kana(c) && c != '\u{3063}') // っ自身独立
            && end < char_list.len()
            && is_combining_kana(char_list[end])
        {
            end += 1;
        }
        let kana: String = char_list[start..end].iter().collect();
        moras.push(ParsedMora {
            kana,
            char_start: start,
            char_end: end,
        });
        ci = end;
    }
    moras
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kanas(text: &str) -> Vec<String> {
        parse_kana_moras(text).into_iter().map(|m| m.kana).collect()
    }

    #[test]
    fn test_yoon_one_mora() {
        assert_eq!(kanas("きゃ"), vec!["きゃ"]);
        assert_eq!(kanas("キョ"), vec!["キョ"]);
        assert_eq!(kanas("しゃししゅしょ"), vec!["しゃ", "し", "しゅ", "しょ"]);
    }

    #[test]
    fn test_chionpu_own_mora() {
        assert_eq!(kanas("スーパー"), vec!["ス", "ー", "パ", "ー"]);
        assert_eq!(kanas("りー"), vec!["り", "ー"]);
    }

    #[test]
    fn test_sokuon_and_hatsuon() {
        assert_eq!(kanas("がっこう"), vec!["が", "っ", "こ", "う"]);
        assert_eq!(kanas("さんぽ"), vec!["さ", "ん", "ぽ"]);
    }

    #[test]
    fn test_foreign_combinations() {
        assert_eq!(
            kanas("ファティシェチェウィ"),
            vec!["ファ", "ティ", "シェ", "チェ", "ウィ"]
        );
    }

    /// 任务书 §8.4: 外来音组合必须各自为 1 mora
    #[test]
    fn test_foreign_mora_list_round4() {
        for word in [
            "ヴァ", "ウェ", "ウォ", "ツァ", "ツィ", "ツェ", "ツォ", "クァ", "グァ", "シェ", "ジェ",
            "チェ", "ディ", "デュ", "トゥ", "ドゥ",
        ] {
            let parsed = parse_kana_moras(word);
            assert_eq!(parsed.len(), 1, "{} must be exactly 1 mora", word);
            assert_eq!(parsed[0].kana, word);
        }
    }

    /// 任务书 §8.4: Unicode 规范化 (半角片假名 / combining dakuten / NFKC)。
    /// 规范化在读取产生点调用; mora 解析消费规范化后的文本。
    #[test]
    fn test_unicode_normalization() {
        // 半角片假名长音符
        assert_eq!(kanas(&normalize_kana("ｽｰﾊﾟｰ")), vec!["ス", "ー", "パ", "ー"]);
        // combining dakuten: か + U+3099 → が
        let combined = "か\u{3099}";
        assert_eq!(normalize_kana(combined), "が");
        assert_eq!(kanas(&normalize_kana(combined)), vec!["が"]);
        // 半角濁音 ﾞ (U+FF9E): NFKC 合成
        assert_eq!(normalize_kana("ｶﾞ"), "ガ");
        assert_eq!(kanas(&normalize_kana("ｶﾞ")), vec!["ガ"]);
        // NFKC 幂等
        assert_eq!(
            normalize_kana(&normalize_kana("ｷｮｳ")),
            normalize_kana("ｷｮｳ")
        );
    }

    #[test]
    fn test_half_width_kana_is_kana_char() {
        assert!(is_kana_char('\u{FF71}')); // ｱ
        assert!(is_kana_char('\u{FF70}')); // ｰ
        assert!(is_kana_char('\u{FF9E}')); // ﾞ
        assert!(!is_kana_char('A'));
    }

    #[test]
    fn test_non_kana_skipped() {
        // 汉字/标点不产生 mora, 送り仮名正常解析
        assert_eq!(kanas("砕いた。"), vec!["い", "た"]);
        assert_eq!(kanas("位に"), vec!["に"]);
    }

    #[test]
    fn test_char_spans() {
        let moras = parse_kana_moras("きゃっと");
        assert_eq!(moras.len(), 3);
        assert_eq!((moras[0].char_start, moras[0].char_end), (0, 2));
        assert_eq!((moras[1].char_start, moras[1].char_end), (2, 3));
        assert_eq!((moras[2].char_start, moras[2].char_end), (3, 4));
    }
}
