use serde::{Deserialize, Serialize};

/// 三级音高数据中的 "Clean Pitch Track"
/// 保留真实连续 F0 曲线用于绘图，另存逐帧 RMS 与离散 NoteEvent 供字幕使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchTrack {
    pub times: Vec<f32>,
    pub frequencies: Vec<f32>,
    pub confidences: Vec<f32>,
    pub midis: Vec<f32>,
    /// 逐帧(10ms) RMS 包络，与 times 对齐，供歌词字级对齐使用
    #[serde(default)]
    pub rms: Vec<f32>,
    /// 离散稳定音符事件 (Annotation Note Track)
    #[serde(default)]
    pub note_events: Vec<NoteEvent>,
}

/// 离散稳定音符事件 (Annotation Note Track)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEvent {
    pub start: f32,
    pub end: f32,
    pub midi: i32,
    pub note_name: String,
    pub confidence: f32,
}

impl NoteEvent {
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// Note Tracker 参数集中管理，避免散落 magic numbers
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NoteTrackingParams {
    /// 新音成为有效音符所需的最短时长
    pub min_note_duration_ms: f32,
    /// 候选新音需持续该时长才承认切换 (debounce)
    pub switch_confirm_ms: f32,
    /// 短于该时长且恰差一个八度 → 视为 octave error
    pub octave_error_max_ms: f32,
    /// 半音边界 hysteresis (cents)，避免 vibrato 触发闪烁
    pub semitone_hysteresis_cents: f32,
    /// 绑定时忽略 token 首尾的极短 margin
    pub edge_ignore_ms: f32,
}

impl Default for NoteTrackingParams {
    fn default() -> Self {
        Self {
            min_note_duration_ms: 45.0,
            switch_confirm_ms: 60.0,
            octave_error_max_ms: 80.0,
            semitone_hysteresis_cents: 25.0,
            edge_ignore_ms: 20.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub confidence_threshold: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub smoothing: usize,
    pub median_smoothing: usize,
    pub quantize: bool,
    pub note_tracking: NoteTrackingParams,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.3,
            fmin: 50.0,
            fmax: 2000.0,
            smoothing: 15,
            median_smoothing: 11,
            quantize: false,
            note_tracking: NoteTrackingParams::default(),
        }
    }
}

/// 前端下发/后端保存的当前分析参数 (全局只使用同一套参数)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisParams {
    pub confidence_threshold: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub smoothing: f64,
    pub median_smoothing: f64,
    pub quantize: bool,
    pub min_note_duration_ms: f32,
}

impl Default for AnalysisParams {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.3,
            fmin: 65.0,
            fmax: 1300.0,
            smoothing: 15.0,
            median_smoothing: 11.0,
            quantize: false,
            min_note_duration_ms: 45.0,
        }
    }
}

impl AnalysisParams {
    /// 转为 analyzer 配置
    pub fn to_analyzer_config(&self) -> AnalyzerConfig {
        let mut note_tracking = NoteTrackingParams::default();
        note_tracking.min_note_duration_ms = self.min_note_duration_ms;
        AnalyzerConfig {
            confidence_threshold: self.confidence_threshold,
            fmin: self.fmin,
            fmax: self.fmax,
            smoothing: self.smoothing as usize,
            median_smoothing: self.median_smoothing as usize,
            quantize: self.quantize,
            note_tracking,
        }
    }
}

/// 一个字内的一个检测音 (由 NoteEvent 或帧级分段生成)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchNote {
    pub start_time: f32,
    pub end_time: f32,
    pub median_midi: f32,
    pub mean_midi: f32,
    pub rounded_midi: i32,
    pub confidence_mean: f32,
    pub point_count: usize,
}

/// 一个字在句内的时间范围与音高绑定结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricToken {
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    #[serde(default)]
    pub pitch_notes: Vec<PitchNote>,
    /// 字幕默认显示的主音 (duration × mean_confidence 评分最高者)
    #[serde(default)]
    pub primary_note: Option<PitchNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricLine {
    /// Display text (含翻译, 用 " | " 分隔)
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    pub tokens: Vec<LyricToken>,
    #[serde(default)]
    pub primary_text: String,
    #[serde(default)]
    pub translations: Vec<String>,
    /// 逐字时间是否为程序自动估计 (DP 对齐或均匀分配)。
    /// 自动估计的时间在重新分析音轨后需要重新对齐；
    /// enhanced LRC 自带的逐字时间 (false) 则永远保留。
    #[serde(default)]
    pub token_timing_auto: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectData {
    pub audio_path: Option<String>,
    pub pitch_track: Option<PitchTrack>,
    pub lyrics: Vec<LyricLine>,
    #[serde(default)]
    pub analysis_params: Option<AnalysisParams>,
}

/// MIDI → 音名 (C4 / F#4 ...)，NaN → "---"
pub fn midi_to_note_name(midi: f32) -> String {
    let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    if midi.is_nan() {
        return "---".to_string();
    }
    let m = midi.round() as i32;
    format!("{}{}", names[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}
