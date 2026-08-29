use crate::analyzer::PitchAnalyzer;
use crate::models::{AnalysisParams, LyricLine, NoteTrackingParams, PitchTrack, ProjectData};
use crate::playback::AudioPlayer;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

pub mod analyzer;
pub mod audio;
pub mod decoder;
pub mod dsp;
pub mod export;
pub mod lyrics;
pub mod mel;
pub mod models;
pub mod note_tracker;
pub mod playback;

struct AppState {
    analyzer: Mutex<Option<PitchAnalyzer>>,
    track: Mutex<Option<PitchTrack>>,
    lyrics: Mutex<Vec<LyricLine>>,
    player: Mutex<Option<AudioPlayer>>,
    audio_path: Mutex<Option<String>>,
    /// 当前分析参数 (全局只使用同一套参数)
    analysis_params: Mutex<Option<AnalysisParams>>,
}

fn find_model_files(app_handle: &tauri::AppHandle) -> Option<(PathBuf, PathBuf)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(res) = app_handle.path().resource_dir() {
        candidates.push(res.join("models"));
        candidates.push(res.clone());
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

pub fn try_init_ort_dylib() {
    if std::env::var("ORT_DYLIB_PATH").is_ok() {
        return;
    }

    // 0. 显式路径优先: 仓库/打包产物的 resources 目录 (避免 PATH 上的旧版 DLL)
    let dylib_name = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    let mut explicit: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            explicit.push(dir.join("resources").join(dylib_name));
            explicit.push(dir.join(dylib_name));
        }
    }
    // 编译期路径: cargo test / dev 运行时定位 src-tauri/resources
    explicit.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(dylib_name));
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        explicit.push(PathBuf::from(&manifest_dir).join("resources").join(dylib_name));
    }
    for p in explicit {
        if p.exists() {
            std::env::set_var("ORT_DYLIB_PATH", &p);
            eprintln!("ORT_DYLIB_PATH = {}", p.display());
            return;
        }
    }

    // 1. Linux: 系统目录与 Python 环境中搜寻 libonnxruntime.so
    if cfg!(target_os = "linux") {
        let find_in_dir = |dir_path: &str| -> Option<PathBuf> {
            let path = Path::new(dir_path);
            if !path.is_dir() {
                return None;
            }
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if let Some(file_name) = p.file_name().and_then(|n| n.to_str()) {
                        if file_name.starts_with("libonnxruntime.so") {
                            return Some(p);
                        }
                    }
                }
            }
            None
        };

        let direct_candidates = ["/usr/lib", "/usr/local/lib", "/usr/lib64", "/opt"];
        for dir in direct_candidates {
            if let Some(p) = find_in_dir(dir) {
                std::env::set_var("ORT_DYLIB_PATH", &p);
                eprintln!("ORT_DYLIB_PATH = {}", p.display());
                return;
            }
        }

        let mut search_dirs = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            search_dirs.push(format!("{}/.local/lib", home));
        }
        if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
            search_dirs.push(format!("{}/lib", venv));
        }
        search_dirs.push("../pitch/.venv/lib".to_string());
        search_dirs.push("../../pitch/.venv/lib".to_string());
        search_dirs.push("./.venv/lib".to_string());
        search_dirs.push("../.venv/lib".to_string());

        let py_versions = ["python3.13", "python3.12", "python3.11", "python3.10", "python3.9", "python3.8"];
        for base_dir in search_dirs {
            for py_ver in &py_versions {
                let capi_path = format!("{}/{}/site-packages/onnxruntime/capi", base_dir, py_ver);
                if let Some(p) = find_in_dir(&capi_path) {
                    std::env::set_var("ORT_DYLIB_PATH", &p);
                    eprintln!("ORT_DYLIB_PATH = {}", p.display());
                    return;
                }
            }
        }
    }

    eprintln!("Warning: 未找到 onnxruntime 运行库, 请设置 ORT_DYLIB_PATH");
}

pub fn init_bundled_ort_dylib(app_handle: &tauri::AppHandle) {
    if std::env::var("ORT_DYLIB_PATH").is_ok() {
        return;
    }

    #[cfg(target_os = "windows")]
    let lib_name = "onnxruntime.dll";
    #[cfg(target_os = "linux")]
    let lib_name = "libonnxruntime.so";
    #[cfg(target_os = "macos")]
    let lib_name = "libonnxruntime.dylib";

    if let Ok(resource_path) = app_handle.path().resolve(format!("resources/{}", lib_name), tauri::path::BaseDirectory::Resource) {
        if resource_path.exists() {
            std::env::set_var("ORT_DYLIB_PATH", &resource_path);
            eprintln!("Loaded bundled ORT dylib from resource: {}", resource_path.display());
            return;
        }
    }

    try_init_ort_dylib();
}

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    progress: f32,
    stage: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct AppConfig {
    model_path: String,
    config_path: String,
}

fn get_config_path(app_handle: &tauri::AppHandle) -> Option<PathBuf> {
    app_handle.path().app_config_dir().ok().map(|p| p.join("config.json"))
}

fn load_stored_config(app_handle: &tauri::AppHandle) -> Option<AppConfig> {
    let p = get_config_path(app_handle)?;
    if p.exists() {
        if let Ok(content) = std::fs::read_to_string(p) {
            return serde_json::from_str(&content).ok();
        }
    }
    None
}

fn save_stored_config(app_handle: &tauri::AppHandle, config_path: &str, model_path: &str) {
    if let Some(p) = get_config_path(app_handle) {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let cfg = AppConfig {
            model_path: model_path.to_string(),
            config_path: config_path.to_string(),
        };
        if let Ok(content) = serde_json::to_string_pretty(&cfg) {
            let _ = std::fs::write(p, content);
        }
    }
}

#[tauri::command]
async fn init_analyzer(app_handle: tauri::AppHandle, app_state: tauri::State<'_, AppState>) -> Result<String, String> {
    init_bundled_ort_dylib(&app_handle);

    // 1. Try loading from stored config first
    let mut resolved_paths = None;
    if let Some(cfg) = load_stored_config(&app_handle) {
        let cfg_path = PathBuf::from(&cfg.config_path);
        let mdl_path = PathBuf::from(&cfg.model_path);
        if cfg_path.exists() && mdl_path.exists() {
            eprintln!("Successfully loaded model from app configuration: {}", cfg.model_path);
            resolved_paths = Some((cfg_path, mdl_path));
        }
    }

    // 2. Fallback to auto-detecting
    let (config_path, model_path) = if let Some(paths) = resolved_paths {
        paths
    } else {
        find_model_files(&app_handle)
            .ok_or_else(|| "找不到 models/fcpe.onnx 或 fcpe_config.json".to_string())?
    };

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
async fn init_analyzer_with_paths(
    app_handle: tauri::AppHandle,
    app_state: tauri::State<'_, AppState>,
    config_path: String,
    model_path: String,
) -> Result<String, String> {
    init_bundled_ort_dylib(&app_handle);
    let cfg_path = PathBuf::from(&config_path);
    let mdl_path = PathBuf::from(&model_path);
    if !cfg_path.exists() || !mdl_path.exists() {
        return Err("所选的配置文件或模型文件不存在".to_string());
    }

    let content = std::fs::read_to_string(&cfg_path).map_err(|e| format!("读取 config 失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| format!("解析 config 失败: {}", e))?;
    let cent_table: Vec<f32> = json["cent_table"]
        .as_array().ok_or_else(|| "config 缺少 cent_table".to_string())?
        .iter().filter_map(|v| v.as_f64().map(|x| x as f32)).collect();
    let analyzer = PitchAnalyzer::new(&mdl_path.to_string_lossy(), cent_table)
        .map_err(|e| format!("初始化 analyzer 失败: {}", e))?;

    *app_state.analyzer.lock().unwrap() = Some(analyzer);

    // 初始化播放器
    match AudioPlayer::new() {
        Ok(player) => *app_state.player.lock().unwrap() = Some(player),
        Err(e) => eprintln!("Warning: 播放器初始化失败: {}", e),
    }

    save_stored_config(&app_handle, &config_path, &model_path);
    Ok(format!("已成功加载自定义模型: {}", mdl_path.display()))
}

#[tauri::command]
async fn analyze_audio(
    app_handle: tauri::AppHandle,
    app_state: tauri::State<'_, AppState>,
    audio_path: String,
    params: AnalysisParams,
) -> Result<PitchTrack, String> {
    use tauri::Emitter;

    let config = params.to_analyzer_config();

    let track = {
        let guard = app_state.analyzer.lock().unwrap();
        let analyzer = guard.as_ref().ok_or_else(|| "Analyzer 尚未初始化".to_string())?;
        let app_handle_clone = app_handle.clone();

        analyzer.analyze(&audio_path, &config, move |progress, stage| {
            let _ = app_handle_clone.emit("analysis-progress", ProgressPayload {
                progress,
                stage: stage.to_string(),
            });
        }).map_err(|e| format!("分析失败: {}", e))?
    };

    // 确保播放器可用并加载音频
    ensure_player(&app_state);
    if let Some(player) = app_state.player.lock().unwrap().as_ref() {
        let _ = player.load(&audio_path);
    }
    *app_state.audio_path.lock().unwrap() = Some(audio_path);
    // 保存当前参数，供 rebind 与导出使用同一阈值
    *app_state.analysis_params.lock().unwrap() = Some(params);
    // 重新绑定 pitch 到歌词
    let t = track.clone();
    *app_state.track.lock().unwrap() = Some(track);
    // 音轨更新后, 自动估计过逐字时间的歌词需要重新对齐 (enhanced LRC 的时间保留)
    {
        let mut lyrics_guard = app_state.lyrics.lock().unwrap();
        if lyrics_guard.iter().any(|l| l.token_timing_auto) {
            let align_params = crate::lyrics::TokenAlignParams::default();
            crate::lyrics::align_token_times(&mut lyrics_guard, &t, &align_params);
        }
    }
    rebind_lyrics(&app_state);
    Ok(t)
}

#[tauri::command]
fn load_lyrics_lrc(app_state: tauri::State<AppState>, path: String) -> Result<Vec<LyricLine>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let duration = app_state.track.lock().unwrap().as_ref()
        .map(|t| t.times.last().copied().unwrap_or(0.0));
    let mut lines = crate::lyrics::parse_lrc(&content, duration);
    // 句级约束下根据音频特征做字级对齐 (无 track 时回退均匀分配)
    {
        let track_guard = app_state.track.lock().unwrap();
        match track_guard.as_ref() {
            Some(track) => {
                let align_params = crate::lyrics::TokenAlignParams::default();
                crate::lyrics::align_token_times(&mut lines, track, &align_params);
            }
            None => {
                crate::lyrics::distribute_token_times(&mut lines);
            }
        }
    }
    // 先写入 state → rebind → 从更新后的 state clone → 返回
    *app_state.lyrics.lock().unwrap() = lines;
    rebind_lyrics(&app_state);
    Ok(app_state.lyrics.lock().unwrap().clone())
}

#[tauri::command]
fn load_lyrics_txt(app_state: tauri::State<AppState>, path: String) -> Result<Vec<LyricLine>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let lines = crate::lyrics::parse_txt(&content);
    // 先写入 state → rebind → 从更新后的 state clone → 返回
    *app_state.lyrics.lock().unwrap() = lines;
    rebind_lyrics(&app_state);
    Ok(app_state.lyrics.lock().unwrap().clone())
}

#[tauri::command]
fn clear_lyrics(app_state: tauri::State<AppState>) -> Result<(), String> {
    app_state.lyrics.lock().unwrap().clear();
    Ok(())
}

/// 使用当前共享参数 rebind 歌词 (不再硬编码 0.3)
fn rebind_lyrics(app_state: &tauri::State<AppState>) {
    let track_guard = app_state.track.lock().unwrap();
    let track = match track_guard.as_ref() {
        Some(t) => t,
        None => return,
    };
    let params = app_state.analysis_params.lock().unwrap().clone().unwrap_or_default();
    let mut note_tracking = NoteTrackingParams::default();
    note_tracking.min_note_duration_ms = params.min_note_duration_ms;
    let mut lyrics_guard = app_state.lyrics.lock().unwrap();
    crate::lyrics::bind_pitch_to_tokens(
        &mut lyrics_guard,
        track,
        params.confidence_threshold,
        &note_tracking,
    );
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
        analysis_params: app_state.analysis_params.lock().unwrap().clone(),
    };
    let json = serde_json::to_string_pretty(&data).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("写入失败: {}", e))?;
    Ok(())
}

/// 加载工程: 统一恢复 track / lyrics / params / audio_path / 播放器 后端状态
#[tauri::command]
fn load_project(app_state: tauri::State<AppState>, path: String) -> Result<ProjectData, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    let data: ProjectData = serde_json::from_str(&content).map_err(|e| format!("解析失败: {}", e))?;

    *app_state.track.lock().unwrap() = data.pitch_track.clone();
    *app_state.lyrics.lock().unwrap() = data.lyrics.clone();
    *app_state.audio_path.lock().unwrap() = data.audio_path.clone();
    *app_state.analysis_params.lock().unwrap() = data.analysis_params.clone();

    // 恢复播放器 (音频文件存在时)
    if let Some(audio) = &data.audio_path {
        if Path::new(audio).exists() {
            ensure_player(&app_state);
            if let Some(player) = app_state.player.lock().unwrap().as_ref() {
                let _ = player.load(audio);
            }
        }
    }

    Ok(data)
}

fn ensure_player(app_state: &AppState) {
    let mut guard = app_state.player.lock().unwrap();
    if guard.is_none() {
        if let Ok(player) = AudioPlayer::new() {
            *guard = Some(player);
        }
    }
}

#[tauri::command]
fn export_srt(app_state: tauri::State<AppState>, path: String) -> Result<(), String> {
    let track = app_state.track.lock().unwrap();
    let track = track.as_ref().ok_or_else(|| "没有分析数据".to_string())?;
    let lyrics = app_state.lyrics.lock().unwrap();
    crate::export::srt::export_srt(track, &lyrics, Path::new(&path))
}

#[tauri::command]
fn export_ass(
    app_state: tauri::State<AppState>,
    path: String,
    pitch_font_size: Option<u32>,
    lyric_font_size: Option<u32>,
) -> Result<(), String> {
    if app_state.track.lock().unwrap().is_none() {
        return Err("没有分析数据，请先分析音频".to_string());
    }
    let lyrics = app_state.lyrics.lock().unwrap();
    let pitch_size = pitch_font_size.unwrap_or(40).clamp(14, 120);
    let lyric_size = lyric_font_size.unwrap_or(28).clamp(12, 72);
    crate::export::ass::export_ass(&lyrics, Path::new(&path), pitch_size, lyric_size)
}

#[tauri::command]
fn midi_to_note_name(midi: f32) -> String {
    crate::models::midi_to_note_name(midi)
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
            analysis_params: Mutex::new(None),
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            init_analyzer,
            init_analyzer_with_paths,
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
            export_ass,
            midi_to_note_name,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
