use crate::analyzer::PitchAnalyzer;
use crate::models::{AnalyzerConfig, LyricLine, PitchTrack, ProjectData};
use crate::playback::AudioPlayer;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

mod analyzer;
pub mod audio;
pub mod decoder;
pub mod dsp;
pub mod lyrics;
pub mod mel;
pub mod models;
pub mod playback;

struct AppState {
    analyzer: Mutex<Option<PitchAnalyzer>>,
    track: Mutex<Option<PitchTrack>>,
    lyrics: Mutex<Vec<LyricLine>>,
    player: Mutex<Option<AudioPlayer>>,
    audio_path: Mutex<Option<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AnalysisParams {
    confidence_threshold: f32,
    fmin: f32,
    fmax: f32,
    smoothing: f64,
    median_smoothing: f64,
    quantize: bool,
}

fn find_model_files(app_handle: &tauri::AppHandle) -> Option<(PathBuf, PathBuf)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app_handle.path().resource_dir() {
        candidates.push(res.join("models"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models"));
            candidates.push(dir.join("../../models"));
            candidates.push(dir.join("../../../models"));
        }
    }
    candidates.push(PathBuf::from("models"));
    for dir in candidates {
        let cfg = dir.join("fcpe_config.json");
        let onnx = dir.join("fcpe.onnx");
        if cfg.exists() && onnx.exists() {
            return Some((cfg, onnx));
        }
    }
    None
}

fn try_init_ort_dylib() {
    if std::env::var("ORT_DYLIB_PATH").is_ok() {
        return;
    }
    let candidates = [
        "/usr/lib/libonnxruntime.so",
        "/usr/local/lib/libonnxruntime.so",
        "/usr/lib/libonnxruntime.so.1",
        "/usr/local/lib/libonnxruntime.so.1",
        "/usr/lib64/libonnxruntime.so",
        "/opt/libonnxruntime.so",
    ];
    for path in candidates {
        if Path::new(path).exists() {
            std::env::set_var("ORT_DYLIB_PATH", path);
            eprintln!("ORT_DYLIB_PATH = {}", path);
            return;
        }
    }
    // Also try to find from python site-packages if available
    if let Ok(home) = std::env::var("HOME") {
        let python_candidates = [
            format!("{}/.local/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.so", home),
            format!("{}/.local/lib/python3.12/site-packages/onnxruntime/capi/libonnxruntime.so", home),
            format!("{}/.local/lib/python3.11/site-packages/onnxruntime/capi/libonnxruntime.so", home),
        ];
        for path in python_candidates {
            if Path::new(&path).exists() {
                std::env::set_var("ORT_DYLIB_PATH", path);
                eprintln!("ORT_DYLIB_PATH = {}", path);
                return;
            }
        }
    }
    eprintln!("Warning: 未找到 libonnxruntime, 请设置 ORT_DYLIB_PATH");
}

#[tauri::command]
fn init_analyzer(app_handle: tauri::AppHandle, app_state: tauri::State<AppState>) -> Result<String, String> {
    try_init_ort_dylib();
    let (config_path, model_path) = find_model_files(&app_handle)
        .ok_or_else(|| "找不到 models/fcpe.onnx 或 fcpe_config.json".to_string())?;
    let content = std::fs::read_to_string(&config_path).map_err(|e| format!("读取 config 失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| format!("解析 config 失败: {}", e))?;
    let cent_table: Vec<f32> = json["cent_table"]
        .as_array().ok_or_else(|| "config 缺少 cent_table".to_string())?
        .iter().filter_map(|v| v.as_f64().map(|x| x as f32)).collect();
    let analyzer = PitchAnalyzer::new(&model_path.to_string_lossy(), cent_table)
        .map_err(|e| format!("初始化 analyzer 失败: {}", e))?;
    *app_state.analyzer.lock().unwrap() = Some(analyzer);
    // 初始化播放器
    match AudioPlayer::new() {
        Ok(player) => *app_state.player.lock().unwrap() = Some(player),
        Err(e) => eprintln!("Warning: 播放器初始化失败: {}", e),
    }
    Ok(format!("已加载模型: {}", model_path.display()))
}

#[tauri::command]
fn analyze_audio(app_state: tauri::State<AppState>, audio_path: String, params: AnalysisParams) -> Result<PitchTrack, String> {
    let guard = app_state.analyzer.lock().unwrap();
    let analyzer = guard.as_ref().ok_or_else(|| "Analyzer 尚未初始化".to_string())?;
    let config = AnalyzerConfig {
        confidence_threshold: params.confidence_threshold,
        fmin: params.fmin,
        fmax: params.fmax,
        smoothing: params.smoothing as usize,
        median_smoothing: params.median_smoothing as usize,
        quantize: params.quantize,
    };
    let track = analyzer.analyze(&audio_path, &config).map_err(|e| format!("分析失败: {}", e))?;
    // 加载到播放器
    if let Some(player) = app_state.player.lock().unwrap().as_ref() {
        let _ = player.load(&audio_path);
    }
    *app_state.audio_path.lock().unwrap() = Some(audio_path);
    // 重新绑定 pitch 到歌词
    let t = track.clone();
    *app_state.track.lock().unwrap() = Some(track);
    rebind_lyrics(&app_state);
    Ok(t)
}

#[tauri::command]
fn load_lyrics_lrc(app_state: tauri::State<AppState>, path: String) -> Result<Vec<LyricLine>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let duration = app_state.track.lock().unwrap().as_ref()
        .map(|t| t.times.last().copied().unwrap_or(0.0));
    let mut lines = crate::lyrics::parse_lrc(&content, duration);
    crate::lyrics::distribute_token_times(&mut lines);
    let result = lines.clone();
    *app_state.lyrics.lock().unwrap() = lines;
    rebind_lyrics(&app_state);
    Ok(result)
}

#[tauri::command]
fn load_lyrics_txt(app_state: tauri::State<AppState>, path: String) -> Result<Vec<LyricLine>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let lines = crate::lyrics::parse_txt(&content);
    let result = lines.clone();
    *app_state.lyrics.lock().unwrap() = lines;
    rebind_lyrics(&app_state);
    Ok(result)
}

#[tauri::command]
fn clear_lyrics(app_state: tauri::State<AppState>) -> Result<(), String> {
    app_state.lyrics.lock().unwrap().clear();
    Ok(())
}

fn rebind_lyrics(app_state: &tauri::State<AppState>) {
    let track_guard = app_state.track.lock().unwrap();
    let track = match track_guard.as_ref() {
        Some(t) => t,
        None => return,
    };
    let mut lyrics_guard = app_state.lyrics.lock().unwrap();
    crate::lyrics::bind_pitch_to_tokens(&mut lyrics_guard, track, 0.3);
}

#[tauri::command]
fn playback_play(app_state: tauri::State<AppState>) -> Result<(), String> {
    app_state.player.lock().unwrap().as_ref()
        .ok_or_else(|| "播放器未初始化".to_string())?
        .play()
}

#[tauri::command]
fn playback_pause(app_state: tauri::State<AppState>) -> Result<(), String> {
    app_state.player.lock().unwrap().as_ref()
        .ok_or_else(|| "播放器未初始化".to_string())?
        .pause()
}

#[tauri::command]
fn playback_seek(app_state: tauri::State<AppState>, secs: f32) -> Result<(), String> {
    app_state.player.lock().unwrap().as_ref()
        .ok_or_else(|| "播放器未初始化".to_string())?
        .seek(secs)
}

#[tauri::command]
fn playback_set_volume(app_state: tauri::State<AppState>, vol: f32) -> Result<(), String> {
    app_state.player.lock().unwrap().as_ref()
        .ok_or_else(|| "播放器未初始化".to_string())?
        .set_volume(vol)
}

#[tauri::command]
fn playback_position(app_state: tauri::State<AppState>) -> Result<f32, String> {
    Ok(app_state.player.lock().unwrap().as_ref()
        .map(|p| p.position()).unwrap_or(0.0))
}

#[tauri::command]
fn playback_duration(app_state: tauri::State<AppState>) -> Result<f32, String> {
    Ok(app_state.player.lock().unwrap().as_ref()
        .map(|p| p.duration()).unwrap_or(0.0))
}

#[tauri::command]
fn playback_is_playing(app_state: tauri::State<AppState>) -> Result<bool, String> {
    Ok(app_state.player.lock().unwrap().as_ref()
        .map(|p| p.is_playing()).unwrap_or(false))
}

#[tauri::command]
fn save_project(app_state: tauri::State<AppState>, path: String) -> Result<(), String> {
    let data = ProjectData {
        audio_path: app_state.audio_path.lock().unwrap().clone(),
        pitch_track: app_state.track.lock().unwrap().clone(),
        lyrics: app_state.lyrics.lock().unwrap().clone(),
    };
    let json = serde_json::to_string_pretty(&data).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("写入失败: {}", e))?;
    Ok(())
}

#[tauri::command]
fn load_project(path: String) -> Result<ProjectData, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("解析失败: {}", e))
}

#[tauri::command]
fn export_srt(app_state: tauri::State<AppState>, path: String) -> Result<(), String> {
    let track = app_state.track.lock().unwrap();
    let track = track.as_ref().ok_or_else(|| "没有分析数据".to_string())?;
    let lyrics = app_state.lyrics.lock().unwrap();
    crate::lyrics::export_srt(track, &lyrics, Path::new(&path))
}

#[tauri::command]
fn midi_to_note_name(midi: f32) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    if midi.is_nan() {
        return "---".to_string();
    }
    let m = midi.round() as i32;
    format!("{}{}", names[(((m % 12) + 12) % 12) as usize], m / 12 - 1)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            analyzer: Mutex::new(None),
            track: Mutex::new(None),
            lyrics: Mutex::new(Vec::new()),
            player: Mutex::new(None),
            audio_path: Mutex::new(None),
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            init_analyzer,
            analyze_audio,
            load_lyrics_lrc,
            load_lyrics_txt,
            clear_lyrics,
            playback_play,
            playback_pause,
            playback_seek,
            playback_set_volume,
            playback_position,
            playback_duration,
            playback_is_playing,
            save_project,
            load_project,
            export_srt,
            midi_to_note_name,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
