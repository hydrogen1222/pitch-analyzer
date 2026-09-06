// Round4 Commit 12: Gold benchmark gate (Tier D)
//
// 对仓库内合成 fixture 跑完整 fallback 链路 (语言路由 → MoraDP →
// legacy note engine → binder + LyricNoteMatcher v2), 评估 §20.4 指标,
// 并执行 baseline 回归门 (§22)。
//
// 真实模型基准 (GAME / singing FA / 真人音频) 通过 Tier C 环境变量
// 加载私有 fixture, 不在本测试中下载模型。

use pitch_analyzer_tauri_lib::alignment::benchmark::{
    check_regression, evaluate_line, project_language_of, synthesize_track_from_gold,
    BenchmarkReport, GoldSample, RegressionTolerance,
};
use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, bind_pitch_to_tokens, parse_lrc_with_language, TokenAlignParams,
};
use pitch_analyzer_tauri_lib::models::{NoteTrackingParams, PitchTrack, ProjectLanguage};
use pitch_analyzer_tauri_lib::note_engine::legacy_fcpe::LegacyFcpeNoteTracker;
use pitch_analyzer_tauri_lib::note_engine::MusicalNoteSource;

const GOLD_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../testdata/alignment_gold");
const SAMPLES: &[(&str, &str)] = &[
    ("ja/sample_001.json", "今日も (boundary evidence)"),
    ("ja/sample_002.json", "あー (melisma)"),
    ("zh/sample_001.json", "我爱你 (mandarin syllables)"),
];

fn load_gold(rel: &str) -> GoldSample {
    let path = std::path::Path::new(GOLD_DIR).join(rel);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("gold fixture missing: {}: {e}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("invalid gold JSON {}: {e}", path.display()))
}

/// 在合成 track 上跑 fallback 链路并评估
fn run_benchmark(rel: &str) -> BenchmarkReport {
    let gold = load_gold(rel);
    let track: PitchTrack = synthesize_track_from_gold(&gold, 0.01);

    // legacy note engine 从合成 F0 转写音符 (无 GAME 模型环境)
    let tracker = LegacyFcpeNoteTracker::default();
    let notes = tracker.transcribe_from_track(&track.times, &track.midis, &track.confidences);
    let mut track = track;
    track.musical_notes = notes;
    track.musical_note_source = MusicalNoteSource::LegacyFcpeTracker;

    // 歌词: timed LRC + 语言路由
    let lrc = format!(
        "[{:02}:{:06.3}]{}\n",
        (gold.line_start as i32) / 60,
        gold.line_start % 60.0,
        gold.text
    );
    let mut lines =
        parse_lrc_with_language(&lrc, Some(gold.line_end + 1.0), project_language_of(&gold));
    assert_eq!(lines.len(), 1, "{}: fixture must be a single line", rel);

    // MoraDP 对齐 (fallback 层, FA 后端缺省)
    align_token_times(&mut lines, &track, &TokenAlignParams::default());
    // binder + matcher v2
    bind_pitch_to_tokens(&mut lines, &track, 0.3, &NoteTrackingParams::default());
    pitch_analyzer_tauri_lib::alignment::note_matcher::attach_lines(&mut lines, &track, 0.3);

    let notes = track.musical_notes.clone();
    evaluate_line(rel, &lines[0], &gold, &notes)
}

/// Tier D 回归门: 与 baseline_report.json 比较, 首跑写入基线
#[test]
fn benchmark_regression_gate() {
    let mut reports = Vec::new();
    for (rel, _) in SAMPLES {
        let report = run_benchmark(rel);
        println!(
            "benchmark {rel}: {}",
            serde_json::to_string(&report.summary()).unwrap()
        );
        let opt = |v: Option<f32>| v.map(|x| format!("{x:.2}")).unwrap_or("-".into());
        println!(
            "  onset median={}ms P90={}ms within100={}",
            opt(report.onset_median_ms),
            opt(report.onset_p90_ms),
            opt(report.onset_within_100ms)
        );
        println!(
            "  note P/R/F1={}/{}/{}",
            opt(report.note_precision),
            opt(report.note_recall),
            opt(report.note_f1)
        );
        println!(
            "  unpitched P/R={}/{}",
            opt(report.unpitched_precision),
            opt(report.unpitched_recall)
        );
        println!(
            "  fallback mora_dp={:.2} unaligned={:.2}",
            report.fallback_mora_dp, report.fallback_unaligned
        );
        reports.push(report);
    }

    // ── 工程级 sanity gate (不是论文级保证, 只拦截明显退化) ──
    for report in &reports {
        assert_eq!(
            report.paired_count, report.unit_count,
            "{}: gold/pipeline segmentation must pair",
            report.sample
        );
        let onset_mae = report
            .onset_mae_ms
            .expect("onset MAE must exist for fixture");
        assert!(
            onset_mae < 300.0,
            "{}: onset MAE sanity failed ({onset_mae:.1}ms)",
            report.sample
        );
        assert!(
            report.fallback_unaligned < 0.5,
            "{}: too many unaligned tokens ({:.2})",
            report.sample,
            report.fallback_unaligned
        );
    }

    // ── baseline 回归门 ──
    let baseline_path = std::path::Path::new(GOLD_DIR).join("baseline_report.json");
    if baseline_path.exists() {
        let baseline: Vec<BenchmarkReport> =
            serde_json::from_str(&std::fs::read_to_string(&baseline_path).expect("read baseline"))
                .expect("parse baseline");
        let tol = RegressionTolerance::default();
        for current in &reports {
            let Some(base) = baseline.iter().find(|b| b.sample == current.sample) else {
                continue;
            };
            check_regression(current, base, &tol)
                .unwrap_or_else(|e| panic!("{}: {e}", current.sample));
        }
        println!(
            "regression gate passed against baseline ({} samples)",
            baseline.len()
        );
    } else {
        std::fs::write(
            &baseline_path,
            serde_json::to_string_pretty(&reports).unwrap(),
        )
        .expect("write baseline");
        println!("baseline written (first run): {}", baseline_path.display());
    }

    // machine-readable report 输出 (§22 Tier D)
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/round4-benchmark");
    let _ = std::fs::create_dir_all(&out_dir);
    let summaries: Vec<_> = reports.iter().map(|r| r.summary()).collect();
    std::fs::write(
        out_dir.join("report.json"),
        serde_json::to_string_pretty(&summaries).unwrap(),
    )
    .expect("write report");
}

/// 配置一致性: Mandarin fixture 必须经 Mandarin pipeline (拼音音节), 日语必须经日语链路
#[test]
fn fixtures_route_to_expected_pipelines() {
    let gold = load_gold("zh/sample_001.json");
    let lrc = "[00:01.00]我爱你\n".to_string();
    let lines = parse_lrc_with_language(&lrc, Some(3.0), ProjectLanguage::Mandarin);
    assert_eq!(lines[0].moras.len(), 3);
    assert_eq!(lines[0].moras[0].kana, "wo3");

    let gold = load_gold("ja/sample_001.json");
    let lrc = "[00:01.00]今日も\n".to_string();
    let lines = parse_lrc_with_language(&lrc, Some(3.0), ProjectLanguage::Japanese);
    let moras: Vec<&str> = lines[0].moras.iter().map(|m| m.kana.as_str()).collect();
    assert_eq!(moras, vec!["きょ", "ー", "も"]);
}
