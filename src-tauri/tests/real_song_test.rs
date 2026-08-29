// 真实歌曲流水线验收测试 (任务书 §19 最终验收场景)
//
// 对仓库根目录下的 MSST 人声干声 FLAC 跑完整分析链路 (解码→mel→FCPE→DSP→NoteEvent)，
// 验收标准:
//   1. NoteEvent 层: 不存在短于 min_note_duration 的事件
//   2. NoteEvent 层: 不存在"短暂八度往返" (中间事件与前后恰差 ±12 且 < 80ms)
//   3. Clean Pitch 层: 无 ≤80ms 的快速大跳往返 (气声/起音毛刺)
//   4. MIDI 值域合理, voiced 比例合理
//   5. LRC 绑定: 对齐到有声区域的歌词大部分获得主音
//
// 依赖 models/fcpe.onnx + fcpe_config.json (缺失时跳过)。
// 运行: cargo test --release --test real_song_test -- --ignored --nocapture

use pitch_analyzer_tauri_lib::analyzer::PitchAnalyzer;
use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times, bind_pitch_to_tokens, parse_lrc, TokenAlignParams,
};
use pitch_analyzer_tauri_lib::models::{AnalysisParams, NoteTrackingParams, PitchTrack};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn find_songs() -> Vec<PathBuf> {
    let root = repo_root();
    let mut songs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e.to_ascii_lowercase().as_str(), "flac" | "wav" | "mp3"))
                .unwrap_or(false)
        })
        .collect();
    songs.sort();
    songs
}

fn load_cent_table() -> Option<Vec<f32>> {
    let path = repo_root().join("models/fcpe_config.json");
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let table: Vec<f32> = json["cent_table"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_f64().map(|x| x as f32))
        .collect();
    Some(table)
}

/// 统计 clean track 中的 "短暂大跳后回到原音" 毛刺数量。
/// 定义: 帧i 之后 max_gap 帧内出现 |Δ|>jump 的帧 j, 且 j 之后 max_gap 帧内
/// 回到 base ±0.75 半音 → 记一次毛刺。真实换音 (跳过去并稳定) 不计。
fn count_transient_spikes(midis: &[f32], max_gap: usize, jump: f32) -> usize {
    let n = midis.len();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < n {
        let base = midis[i];
        if base.is_nan() {
            i += 1;
            continue;
        }
        let mut detected = false;
        for j in (i + 1)..((i + 1 + max_gap).min(n)) {
            let v = midis[j];
            if v.is_nan() {
                continue;
            }
            if (v - base).abs() > jump {
                // 大跳: 看之后是否很快回到 base (毛刺) 或稳定在新音 (真实换音)
                for k in (j + 1)..((j + 1 + max_gap).min(n)) {
                    let w = midis[k];
                    if w.is_nan() {
                        continue;
                    }
                    if (w - base).abs() <= 0.75 {
                        detected = true;
                        break;
                    }
                    if (w - v).abs() <= 0.75 {
                        break;
                    }
                }
                break;
            } else if (v - base).abs() <= 0.75 {
                break;
            }
        }
        if detected {
            count += 1;
        }
        i += 1;
    }
    count
}

/// 孤立发声孤岛: 两侧都是 NaN 的有限段 (显示层闪点来源), 统计 ≤ max_len 帧的数量
fn count_isolated_voiced_islands(midis: &[f32], max_len: usize) -> usize {
    let n = midis.len();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < n {
        if midis[i].is_nan() {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && !midis[i].is_nan() {
            i += 1;
        }
        let e = i - 1;
        let isolated_left = s == 0 || midis[s - 1].is_nan();
        let isolated_right = e + 1 >= n || midis[e + 1].is_nan();
        if isolated_left && isolated_right && e - s + 1 <= max_len {
            count += 1;
        }
    }
    count
}

/// 单帧锯齿: |m[i]-m[i-1]| ≥ jump 且 |m[i+1]-m[i]| ≥ jump (来回跳一帧)
fn count_zigzag(midis: &[f32], jump: f32) -> usize {
    let n = midis.len();
    let mut count = 0usize;
    for i in 1..n.saturating_sub(1) {
        let (a, b, c) = (midis[i - 1], midis[i], midis[i + 1]);
        if a.is_nan() || b.is_nan() || c.is_nan() {
            continue;
        }
        if (b - a).abs() >= jump && (c - b).abs() >= jump && (c - a).abs() < jump {
            count += 1;
        }
    }
    count
}

/// NoteEvent 层的短暂八度往返: 中间事件与前后事件都恰差 ~±12 半音且持续 < 80ms
fn count_octave_roundtrips(events: &[pitch_analyzer_tauri_lib::models::NoteEvent]) -> usize {
    let mut count = 0usize;
    for i in 1..events.len().saturating_sub(1) {
        let d_prev = (events[i].midi - events[i - 1].midi).abs() as f32;
        let d_next = (events[i].midi - events[i + 1].midi).abs() as f32;
        let oct = |d: f32| (d - 12.0).abs() <= 1.5;
        if oct(d_prev) && oct(d_next) && events[i].duration() < 0.08 {
            count += 1;
        }
    }
    count
}

fn run_pipeline(analyzer: &PitchAnalyzer, path: &Path) -> PitchTrack {
    let params = AnalysisParams::default();
    let config = params.to_analyzer_config();
    analyzer
        .analyze(path.to_str().unwrap(), &config, |progress, stage| {
            if (progress * 100.0) as i32 % 25 == 0 {
                eprint!("\r  [{:>3}%] {}", (progress * 100.0) as i32, stage);
            }
        })
        .expect("analyze failed")
}

#[test]
#[ignore]
fn real_songs_acceptance() {
    pitch_analyzer_tauri_lib::try_init_ort_dylib();
    let Some(cent_table) = load_cent_table() else {
        eprintln!("Skipping: models/fcpe_config.json not found");
        return;
    };
    let model_path = repo_root().join("models/fcpe.onnx");
    if !model_path.exists() {
        eprintln!("Skipping: models/fcpe.onnx not found");
        return;
    }
    let songs = find_songs();
    if songs.is_empty() {
        eprintln!("Skipping: no songs found in repo root");
        return;
    }

    let analyzer = PitchAnalyzer::new(model_path.to_str().unwrap(), cent_table).expect("load model");
    let note_params = NoteTrackingParams::default();

    let mut song_idx = 0usize;
    for song in &songs {
        let name = song.file_name().unwrap().to_string_lossy().to_string();
        println!("\n=== {} ===", name);
        let t0 = std::time::Instant::now();
        let track = run_pipeline(&analyzer, song);
        println!("\n  analyze time: {:?}", t0.elapsed());

        // 基本量
        let total = track.times.len();
        let voiced: Vec<usize> = (0..total).filter(|&i| !track.midis[i].is_nan()).collect();
        let voiced_ratio = voiced.len() as f32 / total.max(1) as f32;
        let duration = track.times.last().copied().unwrap_or(0.0);
        println!(
            "  duration: {:.1}s, frames: {}, voiced: {} ({:.1}%)",
            duration,
            total,
            voiced.len(),
            voiced_ratio * 100.0
        );
        assert!(voiced_ratio > 0.03, "voiced ratio too low: {:.2}", voiced_ratio);

        // MIDI 值域
        let mut midis_v: Vec<f32> = voiced.iter().map(|&i| track.midis[i]).collect();
        midis_v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let midi_lo = midis_v[midis_v.len() / 100]; // 1 分位
        let midi_hi = midis_v[midis_v.len() * 99 / 100]; // 99 分位
        println!("  midi p1/p99: {:.1} / {:.1}", midi_lo, midi_hi);
        assert!(midi_lo > 24.0 && midi_hi < 96.0, "midi range implausible");

        // NoteEvent 层
        let events = &track.note_events;
        let durs: Vec<f32> = events.iter().map(|e| e.duration()).collect();
        let min_dur_ms = note_params.min_note_duration_ms;
        let mut dumped = false;
        let short_events: Vec<(f32, i32, f32, f32)> = events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.duration() < min_dur_ms / 1000.0 * 0.9)
            .map(|(i, e)| {
                let prev = if i > 0 { Some(events[i - 1].midi) } else { None };
                let next = events.get(i + 1).map(|e| e.midi);
                println!(
                    "    short event: midi={} dur={:.1}ms start={:.2} prev={:?} next={:?}",
                    e.midi,
                    e.duration() * 1000.0,
                    e.start,
                    prev,
                    next
                );
                // 首个短事件: 打印周围 45 帧的 clean midi (2 位小数), 并对切片重跑 tracker
                if !dumped {
                    dumped = true;
                    let s_idx = (e.start / 0.01).round() as i64;
                    let lo = (s_idx - 20).max(0) as usize;
                    let hi = ((s_idx + 25) as usize).min(track.midis.len());
                    let seq: Vec<String> = track.midis[lo..hi]
                        .iter()
                        .map(|m| if m.is_nan() { "·".to_string() } else { format!("{:.2}", m) })
                        .collect();
                    println!("    frames[{}..{}]: {}", lo, hi, seq.join(" "));
                    let slice_times: Vec<f32> = (lo..hi).map(|k| k as f32 * 0.01).collect();
                    let slice_midis: Vec<f32> = track.midis[lo..hi].to_vec();
                    let slice_conf: Vec<f32> = track.confidences[lo..hi].to_vec();
                    let slice_events = pitch_analyzer_tauri_lib::note_tracker::build_note_events(
                        &slice_times,
                        &slice_midis,
                        &slice_conf,
                        &note_params,
                    );
                    for ev in &slice_events {
                        println!(
                            "    slice event: midi={} {:.3}~{:.3} ({:.0}ms)",
                            ev.midi,
                            ev.start,
                            ev.end,
                            ev.duration() * 1000.0
                        );
                    }
                }
                (e.duration(), e.midi, e.start, e.end)
            })
            .collect();
        println!(
            "  note events: {}, median dur: {:.0}ms",
            events.len(),
            median(&mut durs.clone()) * 1000.0
        );
        assert!(
            short_events.is_empty(),
            "found {} events shorter than {:.0}ms",
            short_events.len(),
            min_dur_ms,
        );

        let oct_roundtrips = count_octave_roundtrips(events);
        println!("  octave roundtrip events (<80ms): {}", oct_roundtrips);
        assert_eq!(oct_roundtrips, 0, "octave roundtrip events must be suppressed");

        // Clean Pitch 层: ≤80ms 快速大跳往返
        let spikes = count_transient_spikes(&track.midis, 8, 6.0);
        println!("  clean-track transient spikes (<=80ms roundtrip >6 semi): {}", spikes);
        assert_eq!(spikes, 0, "transient pitch spikes must be cleaned");

        // 残余细抖动统计 (观察指标, 暂不强制)
        let small_spikes = count_transient_spikes(&track.midis, 5, 2.0);
        let zigzag = count_zigzag(&track.midis, 2.0);
        let zigzag1 = count_zigzag(&track.midis, 1.0);
        println!(
            "  jitter stats: <=50ms/>=2semi roundtrip: {}, 1-frame zigzag >=2semi: {}, >=1semi: {}",
            small_spikes, zigzag, zigzag1
        );
        // 孤立发声孤岛 (静音里的 1-2 帧毛刺) 必须为 0
        let blips = count_isolated_voiced_islands(&track.midis, 2);
        println!("  isolated voiced blips (<=20ms): {}", blips);
        assert_eq!(blips, 0, "isolated voiced blips must be dropped");

        // LRC 绑定: 在最长有声段上生成合成歌词
        let lrc = synthetic_lrc(&track, 8);
        let mut lines = parse_lrc(&lrc, Some(duration));
        align_token_times(&mut lines, &track, &TokenAlignParams::default());
        bind_pitch_to_tokens(&mut lines, &track, 0.3, &note_params);
        let total_tokens: usize = lines.iter().map(|l| l.tokens.len()).sum();
        let bound: usize = lines
            .iter()
            .flat_map(|l| &l.tokens)
            .filter(|t| t.primary_note.is_some())
            .count();
        let coverage = bound as f32 / total_tokens.max(1) as f32;
        println!(
            "  lrc bind: {} lines / {} tokens, primary coverage {:.0}%",
            lines.len(),
            total_tokens,
            coverage * 100.0
        );
        assert!(coverage > 0.6, "primary note coverage too low: {:.2}", coverage);

        // token 时间单调 + 最小字长
        for line in &lines {
            for w in line.tokens.windows(2) {
                let (a, b) = (w[0].end_time.unwrap(), w[1].start_time.unwrap());
                assert!(b >= a - 1e-4, "token times not monotonic");
            }
            for t in &line.tokens {
                let d = t.end_time.unwrap() - t.start_time.unwrap();
                assert!(d >= 0.02, "token too short: {:.0}ms", d * 1000.0);
            }
        }

        // PITCH_DUMP=<dir>: 导出第一首歌的样例 SRT/ASS 供人工检查
        if song_idx == 0 {
            if let Ok(dir) = std::env::var("PITCH_DUMP") {
                let stem = song.file_stem().unwrap().to_string_lossy();
                let _ = std::fs::create_dir_all(&dir);
                let srt_path = Path::new(&dir).join(format!("{}.srt", stem));
                let ass_path = Path::new(&dir).join(format!("{}.ass", stem));
                pitch_analyzer_tauri_lib::export::srt::export_srt(&track, &lines, &srt_path)
                    .expect("export sample srt");
                pitch_analyzer_tauri_lib::export::ass::export_ass(&lines, &ass_path, 48, 28)
                    .expect("export sample ass");
                println!("  dumped sample: {}", srt_path.display());
            }
        }
        let _ = song_idx;
        song_idx += 1;
    }
}

/// 取最长 n_voiced 段生成合成 LRC (每段一句 10 字歌词)
fn synthetic_lrc(track: &PitchTrack, n_lines: usize) -> String {
    // 找最长有声区间
    let mut segs: Vec<(f32, f32)> = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..track.times.len() {
        let voiced = !track.midis[i].is_nan();
        if voiced && start.is_none() {
            start = Some(i);
        } else if !voiced && start.is_some() {
            segs.push((track.times[start.unwrap()], track.times[i]));
            start = None;
        }
    }
    if let Some(s) = start {
        segs.push((track.times[s], *track.times.last().unwrap()));
    }
    segs.sort_by(|a, b| (b.1 - b.0).partial_cmp(&(a.1 - a.0)).unwrap());
    segs.truncate(n_lines);
    segs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let mut lrc = String::new();
    for (i, (s, _e)) in segs.iter().enumerate() {
        let text = "啦".repeat(8 + (i % 3)); // 8~10 字
        let min = (s / 60.0).floor() as u32;
        let sec = s % 60.0;
        lrc.push_str(&format!("[{:02}:{:05.2}]{}\n", min, sec, text));
    }
    lrc
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}
