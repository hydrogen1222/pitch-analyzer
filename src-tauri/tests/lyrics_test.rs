use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, bind_pitch_to_tokens, parse_lrc, parse_txt, select_primary_note, tokenize,
};
use pitch_analyzer_tauri_lib::models::{NoteEvent, NoteTrackingParams, PitchNote, PitchTrack};

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
    for i in 70..130 {
        midis[i] = 60.0; // 0.7s ~ 1.3s voiced
    }
    let track = PitchTrack {
        times: (0..n).map(|i| i as f32 * 0.01).collect(),
        frequencies: vec![261.63; n],
        confidences: vec![0.8; n],
        midis,
        rms: vec![0.0; n],
        note_events: Vec::new(),
    };

    let mut lines = parse_lrc("[00:00.50]你好呀", Some(2.0));
    assert_eq!(lines[0].tokens.len(), 3);
    align_token_times(&mut lines, &track, &Default::default());

    let toks = &lines[0].tokens;
    let starts: Vec<f32> = toks.iter().map(|t| t.start_time.unwrap()).collect();
    let ends: Vec<f32> = toks.iter().map(|t| t.end_time.unwrap()).collect();

    // 对齐窗口 = [max(0.5, 0.7-0.08), min(1.5, 1.3+0.08)] = [0.62, 1.37]
    assert!((starts[0] - 0.62).abs() < 0.02, "first token must start at 0.62, got {}", starts[0]);
    assert!((ends[2] - 1.37).abs() < 0.02, "last token must end at 1.37, got {}", ends[2]);
    // 递增且不重叠
    for i in 0..3 {
        assert!(ends[i] > starts[i], "token {} must have positive duration", i);
        if i > 0 {
            assert!(starts[i] >= ends[i - 1], "tokens must not overlap");
        }
    }
    // 每字时长 >= 最小字时长 (60ms)
    for i in 0..3 {
        assert!(ends[i] - starts[i] >= 0.06, "token {} too short: {}", i, ends[i] - starts[i]);
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
    };
    let mut lines = parse_lrc("[00:00.50]你好呀", Some(2.0));
    // 手工设置逐字时间，模拟 enhanced LRC
    let times = [(0.5, 0.8), (0.8, 1.1), (1.1, 1.4)];
    for (tok, (s, e)) in lines[0].tokens.iter_mut().zip(times.iter()) {
        tok.start_time = Some(*s);
        tok.end_time = Some(*e);
    }
    align_token_times(&mut lines, &track, &Default::default());
    assert_eq!(lines[0].tokens[0].start_time, Some(0.5), "existing times must be kept");
    assert_eq!(lines[0].tokens[2].end_time, Some(1.4), "existing times must be kept");
}

#[test]
fn test_bind_uses_note_events() {
    let track = PitchTrack {
        times: vec![1.0, 1.4],
        frequencies: vec![261.63, 261.63],
        confidences: vec![0.9, 0.9],
        midis: vec![60.0, 60.0],
        rms: Vec::new(),
        note_events: vec![NoteEvent {
            start: 1.0,
            end: 1.4,
            midi: 60,
            note_name: "C4".to_string(),
            confidence: 0.9,
        }],
    };
    let mut lines = parse_lrc("[00:01.00]Test", Some(5.0));
    // 手工给 token 时间，避免依赖均匀分配
    for tok in lines[0].tokens.iter_mut() {
        tok.start_time = Some(1.0);
        tok.end_time = Some(1.4);
    }
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    let tok = &lines[0].tokens[0];
    assert_eq!(tok.pitch_notes.len(), 1, "expected one bound note, got {:?}", tok.pitch_notes);
    let primary = tok.primary_note.as_ref().expect("primary_note must be set");
    assert_eq!(primary.median_midi, 60.0);
}
