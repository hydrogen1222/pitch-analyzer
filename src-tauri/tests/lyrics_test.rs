use pitch_analyzer_tauri_lib::lyrics::{parse_lrc, parse_txt, tokenize};

#[test]
fn test_tokenize_mixed() {
    let toks = tokenize("Hello 世界 こんにちは!");
    println!("tokens: {:?}", toks);
    // Hello (英文词), 世 界 (汉字), こ ん に ち は (假名), ! 合并到 は
    assert!(toks.iter().any(|t| t == "Hello"));
    assert!(toks.iter().any(|t| t == "世"));
    assert!(toks.iter().any(|t| t == "界"));
    assert!(toks.iter().any(|t| t == "こ"));
}

#[test]
fn test_parse_txt_simple() {
    let lines = parse_txt("第一行\n第二行\n\n第三行");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text, "第一行");
    assert_eq!(lines[0].tokens.len(), 3); // 第 一 行
}

#[test]
fn test_parse_lrc_simple() {
    let lrc = "[00:01.00]第一行\n[00:05.00]第二行\n[00:09.00]最后一行";
    let lines = parse_lrc(lrc, Some(15.0));
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].start_time, Some(1.0));
    assert_eq!(lines[0].end_time, Some(5.0));
    assert_eq!(lines[1].start_time, Some(5.0));
    assert_eq!(lines[1].end_time, Some(9.0));
    assert_eq!(lines[2].end_time, Some(15.0));
}

#[test]
fn test_parse_lrc_bilingual() {
    // 同一时间戳的两行被合并为双语
    let lrc = "[00:01.00]Hello world\n[00:01.00]你好世界";
    let lines = parse_lrc(lrc, Some(10.0));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].primary_text, "Hello world");
    assert_eq!(lines[0].translations.len(), 1);
    assert_eq!(lines[0].translations[0], "你好世界");
    assert!(lines[0].text.contains("|"));
}
