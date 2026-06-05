use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchTrack {
    pub times: Vec<f32>,
    pub frequencies: Vec<f32>,
    pub confidences: Vec<f32>,
    pub midis: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub confidence_threshold: f32,
    pub fmin: f32,
    pub fmax: f32,
    pub smoothing: usize,
    pub median_smoothing: usize,
    pub quantize: bool,
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
        }
    }
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricToken {
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    #[serde(default)]
    pub pitch_notes: Vec<PitchNote>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectData {
    pub audio_path: Option<String>,
    pub pitch_track: Option<PitchTrack>,
    pub lyrics: Vec<LyricLine>,
}
