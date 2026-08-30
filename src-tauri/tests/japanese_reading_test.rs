use pitch_analyzer_tauri_lib::japanese::mora::parse_kana_moras;
use pitch_analyzer_tauri_lib::japanese::reading::{
    apply_reading_overrides, JapaneseReadingProvider, ReadingOverride,
};
use pitch_analyzer_tauri_lib::japanese::unidic::LinderaUnidicProvider;

fn get_reading_and_moras(text: &str) -> (String, Vec<String>) {
    let provider = LinderaUnidicProvider;
    let spans = provider.analyze(text).expect("analyze failed");
    let full_reading: String = spans.iter().map(|s| s.reading.as_str()).collect();
    let phonetic: String = spans
        .iter()
        .map(|s| {
            if !s.pronunciation.is_empty() {
                s.pronunciation.as_str()
            } else {
                s.reading.as_str()
            }
        })
        .collect();
    let moras: Vec<String> = parse_kana_moras(&phonetic)
        .into_iter()
        .map(|m| m.kana)
        .collect();
    (full_reading, moras)
}

#[test]
fn test_unidic_readings_regression() {
    // 1. 愛 → あい (2 moras)
    let (_, moras) = get_reading_and_moras("愛");
    assert_eq!(moras, vec!["あ", "い"], "愛 should be 2 moras [あ, い]");

    // 2. 二人 → ふたり (3 moras)
    let (_, moras) = get_reading_and_moras("二人");
    assert_eq!(
        moras,
        vec!["ふ", "た", "り"],
        "二人 should be 3 moras [ふ, た, り]"
    );

    // 3. 心 → こころ (3 moras)
    let (_, moras) = get_reading_and_moras("心");
    assert_eq!(
        moras,
        vec!["こ", "こ", "ろ"],
        "心 should be 3 moras [こ, こ, ろ]"
    );

    // 4. 嵐 → あらし (3 moras)
    let (_, moras) = get_reading_and_moras("嵐");
    assert_eq!(
        moras,
        vec!["あ", "ら", "し"],
        "嵐 should be 3 moras [あ, ら, し]"
    );

    // 5. 流れる → ながれる (4 moras)
    let (_, moras) = get_reading_and_moras("流れる");
    assert_eq!(
        moras,
        vec!["な", "が", "れ", "る"],
        "流れる should be 4 moras [な, が, れ, る]"
    );

    // 6. 群れ → むれ (2 moras)
    let (_, moras) = get_reading_and_moras("群れ");
    assert_eq!(moras, vec!["む", "れ"], "群れ should be 2 moras [む, れ]");

    // 7. スーパー → す・ー・ぱ・ー (4 moras)
    let (_, moras) = get_reading_and_moras("スーパー");
    assert_eq!(
        moras,
        vec!["す", "ー", "ぱ", "ー"],
        "スーパー should be 4 moras"
    );

    // 8. きょう → 2 moras (きょ・ー / きょ・う)
    let (_, moras) = get_reading_and_moras("きょう");
    assert_eq!(moras.len(), 2, "きょう should be 2 moras");
    assert_eq!(moras[0], "きょ");

    // 9. がっこう → 4 moras (が・っ・こ・ー / が・っ・こ・う)
    let (_, moras) = get_reading_and_moras("がっこう");
    assert_eq!(moras.len(), 4, "がっこう should be 4 moras");
    assert_eq!(moras[0], "が");
    assert_eq!(moras[1], "っ");
    assert_eq!(moras[2], "こ");
}

#[test]
fn test_user_reading_override() {
    let provider = LinderaUnidicProvider;
    let mut spans = provider.analyze("運命の扉").expect("analyze failed");

    // Default UniDic reading for 運命 is うんめい (4 moras)
    let initial_moras: Vec<String> = spans
        .iter()
        .flat_map(|s| parse_kana_moras(&s.reading))
        .map(|m| m.kana)
        .collect();
    assert_eq!(&initial_moras[..4], &["う", "ん", "め", "い"]);

    // Override 運命 (chars 0..2) -> さだめ (3 moras)
    let overrides = vec![ReadingOverride {
        char_start: 0,
        char_end: 2,
        reading: "さだめ".to_string(),
    }];
    apply_reading_overrides(&mut spans, &overrides, "運命の扉");

    let overridden_moras: Vec<String> = spans
        .iter()
        .flat_map(|s| parse_kana_moras(&s.reading))
        .map(|m| m.kana)
        .collect();
    assert_eq!(&overridden_moras[..3], &["さ", "だ", "め"]);
    assert_eq!(overridden_moras[3], "の");
}
