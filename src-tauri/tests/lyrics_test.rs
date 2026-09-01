use pitch_analyzer_tauri_lib::forced_align::{
    AlignedMora, ForcedAlignBackend, LineAlignmentResult,
};
use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, align_token_times_with_backend, bind_pitch_to_tokens,
    distribute_token_times, parse_lrc, parse_txt, select_primary_note, tokenize,
};
use pitch_analyzer_tauri_lib::models::AlignmentSource;
use pitch_analyzer_tauri_lib::models::{NoteEvent, NoteTrackingParams, PitchNote, PitchTrack};
use pitch_analyzer_tauri_lib::note_engine::{MusicalNoteEvent, MusicalNoteSource};
use std::path::Path;

fn pn(midi: f32, start: f32, end: f32, conf: f32) -> PitchNote {
    PitchNote {
        start_time: start,
        end_time: end,
        median_midi: midi,
        mean_midi: midi,
        rounded_midi: midi.round() as i32,
        confidence_mean: conf,
        point_count: ((end - start) / 0.01).round().max(1.0) as usize,
    }
}

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
fn timed_ruby_txt_uses_lrc_parser_and_builds_kana_pitch_slots() {
    let text = r#"[ar:岡村孝子]
[al:SOLEIL]
[ti:ドラマ]

[00:18.000]冷(つめ)たく微笑(ほほえ)んだ[00:25.020]
[00:18.000]冷冷地泛起微笑[00:25.020]"#;
    let lines = parse_txt(text);
    assert_eq!(
        lines.len(),
        1,
        "metadata and translation must not become lyric lines"
    );
    let line = &lines[0];
    assert_eq!(line.start_time, Some(18.0));
    assert_eq!(line.end_time, Some(25.02));
    assert_eq!(line.primary_text, "冷たく微笑んだ");
    assert_eq!(line.translations, ["冷冷地泛起微笑"]);

    let reading: String = line
        .reading_display_groups
        .iter()
        .map(|group| {
            if group.reading.is_empty() {
                group.surface.as_str()
            } else {
                group.reading.as_str()
            }
        })
        .collect();
    assert_eq!(reading, "つめたくほほえんだ");
    assert_eq!(
        line.reading_display_groups
            .iter()
            .filter(|group| group.phonetic)
            .map(|group| group.reading.as_str())
            .collect::<Vec<_>>(),
        ["つ", "め", "ほ", "ほ", "え"]
    );
    assert!(line
        .reading_display_groups
        .iter()
        .all(|group| group.mora_end - group.mora_start <= 1));
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

// ── primary note 选择 (§14: 字内错误首音 / 一个字两个真音 / 无 voiced frames) ──

#[test]
fn test_primary_note_short_error_first() {
    // 字内错误首音: F5(40ms) + C4(400ms) → 主音必须是 C4
    let notes = vec![pn(77.0, 0.50, 0.54, 0.8), pn(60.0, 0.54, 0.94, 0.8)];
    let primary = select_primary_note(&notes, 0.045, 0.3).unwrap();
    assert_eq!(primary.median_midi, 60.0, "short leading F5 must not win");
}

#[test]
fn test_primary_note_two_real_notes() {
    // 一个字两个真音: C4(180ms) + D4(250ms)，都是真实音 → 取时长更长者 D4
    let notes = vec![pn(60.0, 0.50, 0.68, 0.8), pn(62.0, 0.68, 0.93, 0.8)];
    let primary = select_primary_note(&notes, 0.045, 0.3).unwrap();
    assert_eq!(primary.median_midi, 62.0, "longer real note D4 must win");
}

#[test]
fn test_primary_note_no_voiced() {
    // 无 voiced frames → None
    assert!(select_primary_note(&[], 0.045, 0.3).is_none());
}

#[test]
fn test_primary_note_fallback_to_only_option() {
    // 只有一个极短候选 (低于 min_duration) → 回退仍返回它 (不返回 None)
    let notes = vec![pn(77.0, 0.50, 0.54, 0.8)];
    let primary = select_primary_note(&notes, 0.045, 0.3).unwrap();
    assert_eq!(primary.median_midi, 77.0);
}

#[test]
fn test_primary_note_low_confidence_rejected() {
    // 短且低置信度 vs 长且高置信度 → 取长且高置信度
    let notes = vec![pn(77.0, 0.50, 0.60, 0.2), pn(60.0, 0.60, 1.00, 0.8)];
    let primary = select_primary_note(&notes, 0.045, 0.3).unwrap();
    assert_eq!(primary.median_midi, 60.0);
}

// ── bind 优先使用 NoteEvent (Annotation Track) ──

// ── 字级时间对齐 (§: LRC 字级对齐 / DP) ──

/// 特征对齐: 只有 [0.7, 1.3] 有发声，句 [0.5, 1.5] 的字时间应被钳制到 voiced 窗口 (± margin)
#[test]
fn test_align_clamps_to_voiced_window() {
    let n = 200;
    let mut midis = vec![f32::NAN; n];
    for m in &mut midis[70..130] {
        *m = 60.0; // 0.7s ~ 1.3s voiced
    }
    let track = PitchTrack {
        times: (0..n).map(|i| i as f32 * 0.01).collect(),
        frequencies: vec![261.63; n],
        confidences: vec![0.8; n],
        midis,
        rms: vec![0.0; n],
        note_events: Vec::new(),
        flux: Vec::new(),
        ..Default::default()
    };

    let mut lines = parse_lrc("[00:00.50]你好呀", Some(2.0));
    assert_eq!(lines[0].tokens.len(), 3);
    align_token_times(&mut lines, &track, &Default::default());

    let toks = &lines[0].tokens;
    let starts: Vec<f32> = toks.iter().map(|t| t.start_time.unwrap()).collect();
    let ends: Vec<f32> = toks.iter().map(|t| t.end_time.unwrap()).collect();

    // 对齐窗口 = [max(0.5, 0.7-0.08), min(1.5, 1.3+0.08)] = [0.62, 1.37]
    assert!(
        (starts[0] - 0.62).abs() < 0.02,
        "first token must start at 0.62, got {}",
        starts[0]
    );
    assert!(
        (ends[2] - 1.37).abs() < 0.02,
        "last token must end at 1.37, got {}",
        ends[2]
    );
    // 递增且不重叠
    for i in 0..3 {
        assert!(
            ends[i] > starts[i],
            "token {} must have positive duration",
            i
        );
        if i > 0 {
            assert!(starts[i] >= ends[i - 1], "tokens must not overlap");
        }
    }
    // 每字时长 >= 最小字时长 (60ms)
    for i in 0..3 {
        assert!(
            ends[i] - starts[i] >= 0.06,
            "token {} too short: {}",
            i,
            ends[i] - starts[i]
        );
    }
}

/// 已有逐字时间 (enhanced LRC) → 不重估
#[test]
fn test_align_skips_existing_times() {
    let track = PitchTrack {
        times: vec![0.0, 0.5, 1.0],
        frequencies: vec![261.63; 3],
        confidences: vec![0.8; 3],
        midis: vec![60.0, 60.0, 60.0],
        rms: Vec::new(),
        note_events: Vec::new(),
        flux: Vec::new(),
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.50]你好呀", Some(2.0));
    // 手工设置逐字时间，模拟 enhanced LRC
    let times = [(0.5, 0.8), (0.8, 1.1), (1.1, 1.4)];
    for (tok, (s, e)) in lines[0].tokens.iter_mut().zip(times.iter()) {
        tok.start_time = Some(*s);
        tok.end_time = Some(*e);
    }
    align_token_times(&mut lines, &track, &Default::default());
    assert_eq!(
        lines[0].tokens[0].start_time,
        Some(0.5),
        "existing times must be kept"
    );
    assert_eq!(
        lines[0].tokens[2].end_time,
        Some(1.4),
        "existing times must be kept"
    );
}

#[test]
fn test_bind_uses_note_events() {
    let track = PitchTrack {
        times: vec![1.0, 1.4],
        frequencies: vec![261.63, 261.63],
        confidences: vec![0.9, 0.9],
        midis: vec![60.0, 60.0],
        rms: Vec::new(),
        flux: Vec::new(),
        note_events: vec![NoteEvent {
            start: 1.0,
            end: 1.4,
            midi: 60,
            note_name: "C4".to_string(),
            confidence: 0.9,
            center_midi: Some(60.0),
            stable_duration: 0.4,
            gestures: Vec::new(),
            tracker_version: 2,
        }],
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:01.00]Test", Some(5.0));
    // 手工给 token 时间，避免依赖均匀分配
    for tok in lines[0].tokens.iter_mut() {
        tok.start_time = Some(1.0);
        tok.end_time = Some(1.4);
    }
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let tok = &lines[0].tokens[0];
    assert_eq!(
        tok.pitch_notes.len(),
        1,
        "expected one bound note, got {:?}",
        tok.pitch_notes
    );
    let primary = tok.primary_note.as_ref().expect("primary_note must be set");
    assert_eq!(primary.median_midi, 60.0);
}

// ── LRC 行尾结束时间戳 / 宽松时间戳 / 多时间戳 ──────────────────

/// 扩展行时格式: [start]text[end] — 常见双语 LRC。旧行为会把 end 当成新行起点,
/// 产生"幽灵句"并污染双语合并 (每行主文本显示为上一句残影)。
#[test]
fn test_parse_lrc_trailing_end_tags() {
    let lrc = "[ar:artist]\n\
               [ti:title]\n\
               \n\
               [00:01.000]first line here[00:10.000]\n\
               [00:01.000]第一行歌词[00:10.000]\n\
               [00:10.000]second line[00:18.500]\n\
               [00:10.000]第二行歌词[00:18.500]\n";
    let lines = parse_lrc(lrc, Some(30.0));
    assert_eq!(
        lines.len(),
        2,
        "expected 2 merged lines, got {}: {:?}",
        lines.len(),
        lines.len()
    );

    // 行 1: 双语合并, 显式结束时间
    assert_eq!(lines[0].primary_text, "first line here");
    assert_eq!(lines[0].translations, vec!["第一行歌词"]);
    assert_eq!(lines[0].start_time, Some(1.0));
    assert_eq!(lines[0].end_time, Some(10.0));
    // 行 2
    assert_eq!(lines[1].primary_text, "second line");
    assert_eq!(lines[1].translations, vec!["第二行歌词"]);
    assert_eq!(lines[1].start_time, Some(10.0));
    assert_eq!(lines[1].end_time, Some(18.5));

    // 任何一行的翻译里都不应混入其他行的原文 (幽灵句检查)
    for l in &lines {
        for t in &l.translations {
            assert!(!l.primary_text.is_empty() && t != &l.primary_text);
        }
    }
}

/// 宽松时间戳: [0:01] 无小数、[0:01.5] 一位小数、[00:01.25] 标准都能解析
#[test]
fn test_parse_lrc_lenient_timestamps() {
    let lrc = "[0:01]aaa[0:05]\n[00:05.5]bbb[00:09.25]\n";
    let lines = parse_lrc(lrc, Some(15.0));
    assert_eq!(
        lines.len(),
        2,
        "{:?}",
        lines
            .iter()
            .map(|l| (&l.primary_text, l.start_time, l.end_time))
            .collect::<Vec<_>>()
    );
    assert_eq!(lines[0].start_time, Some(1.0));
    assert_eq!(lines[0].end_time, Some(5.0));
    assert_eq!(lines[1].start_time, Some(5.5));
    assert_eq!(lines[1].end_time, Some(9.25));
}

/// 多时间戳重复段: [t1][t2]同句 → 两个条目 (不能当成行尾时间)
#[test]
fn test_parse_lrc_multi_timestamp_repeat() {
    let lrc = "[00:10.000][00:70.000]chorus line[00:80.000]\n";
    let lines = parse_lrc(lrc, Some(120.0));
    // [00:70.000] 与 [00:80.000] 之间的文本为空? 不 — 文本在最后一个标签前,
    // 标签 t1,t2 相连在开头, end 标签在文本后 → 首尾之间有文本 → 扩展格式
    // starts=[10], end=80... 但 [00:70] 夹在中间与首标签相连:
    // trimmed = "[00:10.000][00:70.000]chorus line[00:80.000]"
    // tags[0]=10, tags[1]=70, tags[2]=80; between tags[0].end..tags[2].start
    //   = "[00:70.000]chorus line" → 去标签后 "chorus line" 非空 → 扩展格式
    // → start=10, end=80。70 被忽略 (罕见格式, 可接受)
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].start_time, Some(10.0));
    assert_eq!(lines[0].end_time, Some(80.0));
}

/// 真正的多时间戳重复: 标签全部连在开头
#[test]
fn test_parse_lrc_repeat_tags_at_line_start() {
    let lrc = "[00:10.000][01:20.000]chorus line\n";
    let lines = parse_lrc(lrc, Some(200.0));
    assert_eq!(lines.len(), 2, "repeated tags should create two entries");
    assert_eq!(lines[0].start_time, Some(10.0));
    assert_eq!(lines[1].start_time, Some(80.0));
    assert_eq!(lines[0].primary_text, "chorus line");
}

// ── 日语莫拉对齐 ──────────────────────────────────────────────

use pitch_analyzer_tauri_lib::lyrics::token_weight;

/// 小書き仮名并入前一拍; 长音符为显示合并但语义上是独立一拍
#[test]
fn test_japanese_attach_char_merge() {
    let toks = tokenize("きゃりーぱみゅぱみゅ");
    assert_eq!(
        toks,
        vec!["きゃ", "りー", "ぱ", "みゅ", "ぱ", "みゅ"],
        "small kana and chōonpu merge into previous display token: {:?}",
        toks
    );
    // 前一 token 不是假名时不合并
    let toks2 = tokenize("AーB");
    assert_ne!(toks2.first().map(|s| s.as_str()), Some("Aー"));
}

/// 拗音 code point 正确性 (P0): ゃゅょ=3083/3085/3087, ャュョ=30E3/30E5/30E7
/// 旧代码把 や(3084)/ヤ(30E4) 当小假名、漏掉 ょ(3087)/ョ(30E7)
#[test]
fn test_youon_codepoints() {
    assert_eq!(tokenize("きょ"), vec!["きょ"], "き+ょ must merge (U+3087)");
    assert_eq!(tokenize("キャ"), vec!["キャ"]);
    assert_eq!(tokenize("キョ"), vec!["キョ"], "キ+ョ must merge (U+30E7)");
    assert_eq!(tokenize("しゃ"), vec!["しゃ"]);
    assert_eq!(tokenize("しゅ"), vec!["しゅ"], "しゅ (U+3085)");
    // 普通大小的や/ヤ絶不附着
    assert_eq!(
        tokenize("てや"),
        vec!["て", "や"],
        "や (U+3084) is a normal kana"
    );
    assert_eq!(
        tokenize("テヤ"),
        vec!["テ", "ヤ"],
        "ヤ (U+30E4) is a normal katakana"
    );
    assert_eq!(tokenize("や"), vec!["や"]);
    assert_eq!(tokenize("ヤ"), vec!["ヤ"]);
}

/// 权重: 汉字 ≈1.8 拍, 假名 =1 拍, 长音符 = 独立 1 拍, 标点 = 0, 拉丁词 = 元音簇数
#[test]
fn test_token_weight_heuristics() {
    assert!((token_weight("心") - 1.8).abs() < 1e-4);
    assert!((token_weight("きゃ") - 1.0).abs() < 1e-4);
    // ー 是独立一拍: スーパー = スー(2) + パー(2) = 4 mora
    assert!(
        (token_weight("スーパー") - 4.0).abs() < 1e-4,
        "ス・ー・パ・ー = 4 mora"
    );
    assert!((token_weight("りー") - 2.0).abs() < 1e-4);
    assert!(
        (token_weight("hello") - 2.0).abs() < 1e-4,
        "he-llo = 2 syllables"
    );
    assert!((token_weight("through") - 1.0).abs() < 1e-4);
    assert!((token_weight("魔法") - 3.6).abs() < 1e-4);
    // 促音/拨音各 1 拍
    assert!(
        (token_weight("がっこう") - 4.0).abs() < 1e-4,
        "が/っ/こ/う = 4"
    );
    // 标点零权重
    assert!(
        (token_weight("た。") - 1.0).abs() < 1e-4,
        "punctuation must be 0 mora"
    );
    assert!((token_weight("あ!") - 1.0).abs() < 1e-4);
}

/// 加权分配: "心を砕いた" (心=1.8, を=1, 砕=1.8, い=1, た=1, 总 6.6)
/// 10s 无特征音轨 → 心 应拿 10*1.8/6.6 = 2.727s, 而不是等分 2s
#[test]
fn test_weighted_distribute() {
    let mut lines = parse_lrc("[00:00.000]心を砕いた[00:10.000]", Some(10.0));
    assert_eq!(
        lines[0].tokens.len(),
        5,
        "{:?}",
        lines[0].tokens.iter().map(|t| &t.text).collect::<Vec<_>>()
    );
    distribute_token_times(&mut lines);
    let durs: Vec<f32> = lines[0]
        .tokens
        .iter()
        .map(|t| t.end_time.unwrap() - t.start_time.unwrap())
        .collect();
    let kanji_dur = durs[0];
    let kana_dur = durs[1];
    assert!(
        (kanji_dur - 10.0 * 1.8 / 6.6).abs() < 0.01,
        "kanji should get 1.8/6.6 of the line, got {:.3}",
        kanji_dur
    );
    assert!(
        (kana_dur - 10.0 * 1.0 / 6.6).abs() < 0.01,
        "kana should get 1.0/6.6 of the line, got {:.3}",
        kana_dur
    );
    assert!((kanji_dur / kana_dur - 1.8).abs() < 0.01);
}

/// 转音 (一字多音) 场景: 一个 token 窗口内 C4(短) → E4(长) → 主音必须是 E4,
/// 且两个音都保留在 pitch_notes 里
#[test]
fn test_melisma_token_binding() {
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let track = PitchTrack {
        times: times.clone(),
        frequencies: vec![261.63; 100],
        confidences: vec![0.9; 100],
        midis: {
            let mut m = vec![60.0; 20]; // C4 0-0.2s (转音起音)
            m.extend(vec![64.0; 80]); // E4 0.2-1.0s (主音)
            m
        },
        rms: Vec::new(),
        note_events: Vec::new(),
        flux: Vec::new(),
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.000]ああ", Some(1.0));
    distribute_token_times(&mut lines);
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let first = &lines[0].tokens[0];
    // 帧 0-20 属于第一个 token (0~0.5s 均匀分配)
    assert!(
        first.pitch_notes.len() >= 2,
        "melisma token must keep all notes: {:?}",
        first.pitch_notes
    );
    let primary = first.primary_note.as_ref().unwrap();
    assert_eq!(
        primary.rounded_midi, 64,
        "primary must be the long E4, not first C4"
    );
}

/// v2 路径的转音绑定: NoteEvent 层已有 C4(短) + E4(长), token 聚合后
/// primary = E4 (覆盖 x 置信 x 稳定), 两个音都在 pitch_notes
#[test]
fn test_melisma_binding_v2_events() {
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let mk = |start: f32, end: f32, midi: i32| NoteEvent {
        start,
        end,
        midi,
        note_name: String::new(),
        confidence: 0.9,
        center_midi: Some(midi as f32),
        stable_duration: end - start,
        gestures: Vec::new(),
        tracker_version: 2,
    };
    let track = PitchTrack {
        times,
        frequencies: vec![261.63; 100],
        confidences: vec![0.9; 100],
        midis: {
            let mut m = vec![60.0; 20];
            m.extend(vec![64.0; 80]);
            m
        },
        rms: Vec::new(),
        flux: Vec::new(),
        note_events: vec![mk(0.0, 0.2, 60), mk(0.2, 1.0, 64)],
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.000]ああ", Some(1.0));
    distribute_token_times(&mut lines);
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let first = &lines[0].tokens[0];
    assert_eq!(
        first.pitch_notes.len(),
        2,
        "both notes admitted: {:?}",
        first.pitch_notes
    );
    let primary = first.primary_note.as_ref().unwrap();
    assert_eq!(primary.rounded_midi, 64, "primary = long E4");
}

/// 零真实重叠的邻近事件不得产生正式 badge (B1 准入硬性验收)
#[test]
fn test_zero_overlap_neighbor_not_bound() {
    let times: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let mk = |start: f32, end: f32, midi: i32| NoteEvent {
        start,
        end,
        midi,
        note_name: String::new(),
        confidence: 0.95,
        center_midi: Some(midi as f32),
        stable_duration: end - start,
        gestures: Vec::new(),
        tracker_version: 2,
    };
    let track = PitchTrack {
        times,
        frequencies: vec![261.63; 100],
        confidences: vec![0.9; 100],
        midis: vec![f32::NAN; 100],
        rms: Vec::new(),
        flux: Vec::new(),
        // 事件在 token 窗口 [0, 0.5] 之外 18ms 处结束 (零重叠)
        note_events: vec![mk(0.0, 0.518, 62)],
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.518]あ", Some(1.0));
    distribute_token_times(&mut lines);
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let tok = &lines[0].tokens[0];
    assert!(
        tok.primary_note.is_none() && tok.pitch_notes.is_empty(),
        "zero-overlap neighbor must NOT become a badge: {:?}",
        tok.pitch_notes
    );
    assert!(
        tok.unpitched_reason.is_some(),
        "unpitched reason must be set"
    );
}

// ── Round3.1: mora 唯一性 + pitch_notes 时序 ─────────────────────

use pitch_analyzer_tauri_lib::lyrics::build_align_units_debug;

/// UniDic coarse span (二人→ふたり) 的 mora 不得按 display token 复制进
/// 对齐序列 (任务书 §23): 每个 mora 恰好出现一次
#[test]
fn round3_align_units_no_duplicate_moras() {
    for text in [
        "愛",
        "心",
        "二人",
        "流れる",
        "群れ",
        "知らない二人にもどるのね",
        "心を引き裂く嵐の中",
        "流れる人の群れ",
    ] {
        let lines = parse_lrc(&format!("[00:00.000]{}", text), Some(10.0));
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        let mora_count = line.moras.len();
        assert!(mora_count > 0, "{} produced no moras", text);
        let units = build_align_units_debug(line);
        let mora_ids: Vec<usize> = units.iter().filter_map(|(m, _, _)| *m).collect();
        let mut sorted_ids = mora_ids.clone();
        sorted_ids.sort_unstable();
        sorted_ids.dedup();
        assert_eq!(
            sorted_ids.len(),
            mora_ids.len(),
            "{}: duplicated mora in align units: {:?}",
            text,
            mora_ids
        );
        assert_eq!(
            sorted_ids,
            (0..mora_count).collect::<Vec<_>>(),
            "{}: mora ids must be 0..n exactly once",
            text
        );
    }
}

/// Binder must consume canonical musical_notes when both canonical and legacy
/// compatibility events are present; a stale legacy event must not win.
#[test]
fn round3_canonical_note_track_is_primary_binding_source() {
    let track = PitchTrack {
        times: (0..100).map(|i| i as f32 * 0.01).collect(),
        frequencies: vec![440.0; 100],
        confidences: vec![0.9; 100],
        midis: vec![69.0; 100],
        musical_notes: vec![MusicalNoteEvent {
            id: 0,
            start: 0.0,
            end: 1.0,
            midi_float: 72.0,
            midi_rounded: 72,
            note_name: "C5".to_string(),
            confidence: 0.95,
            source: MusicalNoteSource::Game,
            model_confidence: None,
            boundary_confidence: Some(0.9),
            is_slur: Some(false),
            evidence: None,
            class: None,
        }],
        note_events: vec![NoteEvent {
            start: 0.0,
            end: 1.0,
            midi: 60,
            note_name: "C4".to_string(),
            confidence: 0.99,
            center_midi: Some(60.0),
            stable_duration: 1.0,
            gestures: Vec::new(),
            tracker_version: 2,
        }],
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.000]あ", Some(1.0));
    distribute_token_times(&mut lines);
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    assert_eq!(
        lines[0].tokens[0]
            .primary_note
            .as_ref()
            .unwrap()
            .rounded_midi,
        72
    );
}

struct TestForcedAlign;

impl ForcedAlignBackend for TestForcedAlign {
    fn name(&self) -> &'static str {
        "test-forced-align"
    }

    fn align_line(
        &self,
        line: &pitch_analyzer_tauri_lib::models::LyricLine,
        _track: &PitchTrack,
        _audio_path: &Path,
        window_start: f32,
        window_end: f32,
    ) -> Result<LineAlignmentResult, String> {
        let unit = (window_end - window_start) / line.moras.len() as f32;
        Ok(LineAlignmentResult {
            line_index: 0,
            source: AlignmentSource::ForcedAlign,
            confidence: 0.88,
            moras: line
                .moras
                .iter()
                .enumerate()
                .map(|(i, m)| AlignedMora {
                    mora_index: i,
                    mora: m.kana.clone(),
                    start: window_start + i as f32 * unit,
                    end: window_start + (i + 1) as f32 * unit,
                    confidence: 0.88,
                })
                .collect(),
        })
    }
}

#[test]
fn round3_forced_align_is_used_before_mora_dp() {
    let mut lines = parse_lrc("[00:00.000]あい", Some(2.0));
    align_token_times_with_backend(
        &mut lines,
        &PitchTrack::default(),
        &pitch_analyzer_tauri_lib::lyrics::TokenAlignParams::default(),
        Some(Path::new("test-audio")),
        Some(&TestForcedAlign),
    );
    assert!(lines[0]
        .tokens
        .iter()
        .all(|token| token.alignment_source == Some(AlignmentSource::ForcedAlign)));
    assert!(lines[0].moras.iter().all(|m| m.start_time.is_some()));
}

/// 详细模式的 pitch_notes 必须按时间升序 (HashMap 迭代无序不得泄漏)
#[test]
fn round3_pitch_notes_chronological() {
    let times: Vec<f32> = (0..200).map(|i| i as f32 * 0.01).collect();
    let mk = |start: f32, end: f32, midi: i32| NoteEvent {
        start,
        end,
        midi,
        note_name: String::new(),
        confidence: 0.9,
        center_midi: Some(midi as f32),
        stable_duration: end - start,
        gestures: Vec::new(),
        tracker_version: 2,
    };
    // 故意乱序构造 note_events (HashMap 迭代顺序不可控, 因此直接乱序输入)
    let track = PitchTrack {
        times,
        frequencies: vec![261.63; 200],
        confidences: vec![0.9; 200],
        midis: vec![f32::NAN; 200],
        rms: Vec::new(),
        flux: Vec::new(),
        note_events: vec![mk(0.6, 1.0, 67), mk(0.0, 0.2, 60), mk(0.2, 0.6, 64)],
        ..Default::default()
    };
    let mut lines = parse_lrc("[00:00.000]ああ", Some(2.0));
    distribute_token_times(&mut lines);
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let tok = &lines[0].tokens[0];
    assert!(
        tok.pitch_notes.len() >= 2,
        "melisma expected: {:?}",
        tok.pitch_notes
    );
    for w in tok.pitch_notes.windows(2) {
        assert!(
            w[0].start_time <= w[1].start_time,
            "pitch_notes must be chronological: {:?}",
            tok.pitch_notes
        );
    }
    // primary 仍应是覆盖最长的 E4 (0.2-0.6 = 0.4s)
    let primary = tok.primary_note.as_ref().unwrap();
    assert_eq!(primary.rounded_midi, 64, "primary must be E4");
}
