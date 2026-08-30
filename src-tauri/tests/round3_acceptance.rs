//! Round3 acceptance tests.
//!
//! These tests are intentionally ignored for the fast test suite. The
//! acceptance job runs them with `ROUND3_ACCEPTANCE=1`; missing model/audio
//! resources then fail loudly instead of being silently skipped.

use pitch_analyzer_tauri_lib::analyzer::PitchAnalyzer;
use pitch_analyzer_tauri_lib::forced_align::{ForcedAlignBackend, MmsForcedAlignBackend};
use pitch_analyzer_tauri_lib::lyrics::{
    align_token_times_with_backend, parse_lrc, TokenAlignParams,
};
use pitch_analyzer_tauri_lib::models::AnalysisParams;
use pitch_analyzer_tauri_lib::models::PitchTrack;
use pitch_analyzer_tauri_lib::note_engine::game::{
    assert_game_parity, compare_game_reference, GameModelPaths, GameNoteEngine, GameReferenceNote,
};
use pitch_analyzer_tauri_lib::note_engine::{
    CanonicalNotePostProcessor, MusicalNoteEngine, MusicalNoteSource,
};
use std::path::{Path, PathBuf};

fn require_acceptance() {
    assert_eq!(
        std::env::var("ROUND3_ACCEPTANCE").as_deref(),
        Ok("1"),
        "set ROUND3_ACCEPTANCE=1 to run model-backed acceptance tests"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a repository parent")
        .to_path_buf()
}

fn game_dir() -> PathBuf {
    if let Some(value) = std::env::var_os("ROUND3_GAME_MODEL_DIR") {
        return PathBuf::from(value);
    }
    repo_root().join("models/GAME-1.0.3-small-onnx")
}

fn fcpe_model() -> PathBuf {
    std::env::var_os("ROUND3_FCPE_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("models/fcpe.onnx"))
}

fn fcpe_cent_table() -> Vec<f32> {
    let path = std::env::var_os("ROUND3_FCPE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("models/fcpe_config.json"));
    let json: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read FCPE config {}: {e}", path.display())),
    )
    .expect("parse FCPE config");
    json["cent_table"]
        .as_array()
        .expect("FCPE config cent_table")
        .iter()
        .map(|value| value.as_f64().expect("numeric cent_table") as f32)
        .collect()
}

fn synthetic_song(sample_rate: u32) -> Vec<f32> {
    let n = sample_rate as usize * 4;
    (0..n)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let hz = if t < 2.0 { 220.0 } else { 329.63 };
            0.15 * (2.0 * std::f32::consts::PI * hz * t).sin()
        })
        .collect()
}

#[test]
#[ignore = "loads the real GAME ONNX bundle"]
fn game_real_model_is_not_fixed_c4() {
    require_acceptance();
    pitch_analyzer_tauri_lib::try_init_ort_dylib();
    let dir = game_dir();
    assert!(
        GameModelPaths::find_in_dir(&dir).is_some(),
        "incomplete GAME model: {}",
        dir.display()
    );
    let engine = GameNoteEngine::try_load(&dir).expect("real GAME model must load");
    let notes = engine
        .transcribe(&synthetic_song(44_100), 44_100, None)
        .expect("real GAME inference must succeed");
    assert!(
        !notes.is_empty(),
        "GAME returned no notes for the synthetic melody"
    );
    assert!(
        notes.iter().any(|n| n.midi_rounded != 60),
        "GAME output looks like a fixed C4 placeholder: {notes:?}"
    );
    assert!(notes.windows(2).all(|w| w[0].start <= w[1].start));
    assert!(notes
        .iter()
        .all(|n| n.source == pitch_analyzer_tauri_lib::note_engine::MusicalNoteSource::Game));
}

#[test]
#[ignore = "loads the real GAME ONNX bundle and exercises Mode B"]
fn game_mode_b_accepts_external_boundaries() {
    require_acceptance();
    pitch_analyzer_tauri_lib::try_init_ort_dylib();
    let engine = GameNoteEngine::try_load(&game_dir()).expect("real GAME model must load");
    let notes = engine
        .transcribe(
            &synthetic_song(44_100),
            44_100,
            Some(&[0.0, 1.0, 2.0, 3.0, 4.0]),
        )
        .expect("GAME Mode B inference must succeed");
    assert!(notes.windows(2).all(|w| w[0].start <= w[1].start));
}

fn required_fixture(name: &str) -> PathBuf {
    let key = format!("ROUND3_{name}");
    let path = std::env::var_os(&key)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{key} must point to an acceptance fixture"));
    assert!(path.is_file(), "fixture does not exist: {}", path.display());
    path
}

#[test]
#[ignore = "compares the same WAV against an official GAME/reference JSON"]
fn game_official_reference_parity() {
    require_acceptance();
    let audio = required_fixture("GAME_PARITY_AUDIO");
    let reference_path = required_fixture("GAME_PARITY_REFERENCE");
    let reference_value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&reference_path).expect("read GAME reference JSON"),
    )
    .expect("parse GAME reference JSON");
    let reference: Vec<GameReferenceNote> = if reference_value.is_array() {
        serde_json::from_value(reference_value).expect("reference JSON array of notes")
    } else {
        serde_json::from_value(reference_value["notes"].clone())
            .expect("reference JSON object.notes array")
    };

    pitch_analyzer_tauri_lib::try_init_ort_dylib();
    let engine = GameNoteEngine::try_load(&game_dir()).expect("GAME engine must load");
    let decoded =
        pitch_analyzer_tauri_lib::audio::load_audio_mono(&audio, engine.target_sample_rate())
            .expect("decode parity audio");
    let actual = engine
        .transcribe(&decoded.samples, decoded.sample_rate, None)
        .expect("Rust GAME inference");
    let report = compare_game_reference(&actual, &reference);
    println!("GAME parity report: {report:?}");
    assert_game_parity(&report).expect("Rust GAME must match official reference");
}

#[test]
#[ignore = "downloads/loads the real MMS-FA acoustic model"]
fn forced_align_real_model_returns_monotonic_moras() {
    require_acceptance();
    let audio = required_fixture("FA_AUDIO");
    let backend = MmsForcedAlignBackend::from_env().expect("MMS-FA backend must be configured");
    let lines = parse_lrc("[00:00.00]あい", Some(2.0));
    assert_eq!(lines.len(), 1);
    let track = PitchTrack::default();
    let result = backend
        .align_line(&lines[0], &track, &audio, 0.0, 2.0)
        .expect("MMS-FA must run actual acoustic inference");
    assert_eq!(
        result.source,
        pitch_analyzer_tauri_lib::models::AlignmentSource::ForcedAlign
    );
    assert_eq!(result.moras.len(), lines[0].moras.len());
    assert!(result
        .moras
        .windows(2)
        .all(|w| w[0].end <= w[1].start + 0.001));
}

#[test]
#[ignore = "runs the full FCPE + GAME + lyric alignment chain"]
fn round3_e2e_fixture_has_canonical_track_and_aligned_lyrics() {
    require_acceptance();
    let audio = required_fixture("E2E_AUDIO");
    let lrc = required_fixture("E2E_LRC");
    let content = std::fs::read_to_string(&lrc).expect("read E2E LRC");
    let mut lines = parse_lrc(&content, Some(120.0));
    assert!(!lines.is_empty(), "E2E LRC contains no timed lines");

    // Exercise the same FCPE -> GAME canonical pipeline used by AppState.
    pitch_analyzer_tauri_lib::try_init_ort_dylib();
    let model = fcpe_model();
    assert!(
        model.is_file(),
        "FCPE model does not exist: {}",
        model.display()
    );
    let analyzer = PitchAnalyzer::new(
        model.to_str().expect("FCPE model path is UTF-8"),
        fcpe_cent_table(),
    )
    .expect("FCPE model must load");
    let mut track = analyzer
        .analyze(
            audio.to_str().expect("audio path is UTF-8"),
            &AnalysisParams::default().to_analyzer_config(),
            |_, _| {},
        )
        .expect("FCPE E2E inference");
    let engine = GameNoteEngine::try_load(&game_dir()).expect("GAME engine must load");
    let decoded = pitch_analyzer_tauri_lib::audio::load_audio_mono(
        Path::new(&audio),
        engine.target_sample_rate(),
    )
    .expect("decode E2E audio");
    let notes = engine
        .transcribe(&decoded.samples, decoded.sample_rate, None)
        .expect("GAME E2E inference");
    assert!(!notes.is_empty(), "E2E GAME track is empty");
    let processor = CanonicalNotePostProcessor::new(0.3);
    let canonical = processor.process(&notes, &track);
    track.raw_game_notes = notes;
    track.canonical_sung_notes = canonical.clone();
    track.musical_notes = processor.accepted_events(&canonical);
    track.musical_note_source = MusicalNoteSource::Game;
    track.musical_note_model = Some(engine.model_tag().to_string());
    let track = PitchTrack {
        musical_notes: track.musical_notes,
        musical_note_source: pitch_analyzer_tauri_lib::note_engine::MusicalNoteSource::Game,
        musical_note_model: track.musical_note_model,
        ..track
    };

    let backend = MmsForcedAlignBackend::from_env().expect("MMS-FA backend must be configured");
    align_token_times_with_backend(
        &mut lines,
        &track,
        &TokenAlignParams::default(),
        Some(audio.as_path()),
        Some(&backend),
    );
    assert!(lines.iter().flat_map(|l| l.tokens.iter()).any(|t| {
        t.alignment_source == Some(pitch_analyzer_tauri_lib::models::AlignmentSource::ForcedAlign)
    }));
}
