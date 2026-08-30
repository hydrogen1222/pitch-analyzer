// Musical Note Engine: 音符转写引擎抽象 (任务书 Phase 3)
//
// 负责把连续音频/特征转换为离散音乐音符事件序列 (MusicalNoteEvent)。
//   - Canonical engine: GAME (Generative Adaptive MIDI Extractor)
//   - Fallback engine: LegacyFcpeNoteTracker (基于 NoteTracker v2 stable-plateau)
//   - Imported MIDI engine: 外部标准 MIDI

use serde::{Deserialize, Serialize};

pub mod game;
pub mod legacy_fcpe;

pub use game::GameNoteEngine;
pub use legacy_fcpe::LegacyFcpeNoteTracker;

/// 音乐音符数据来源
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum MusicalNoteSource {
    Game,
    #[default]
    LegacyFcpeTracker,
    ImportedMidi,
}

/// Binder / matcher 的统一音符窗口抽象: canonical MusicalNoteEvent 与
/// legacy NoteEvent 都能进入同一套准入/绑定逻辑
pub trait NoteWindow {
    fn window(&self) -> (f32, f32);
    fn confidence(&self) -> f32;
    fn midi_float(&self) -> f32;
    fn midi_rounded(&self) -> i32;
    /// 稳定占比 (stable/duration); 无该概念的实现返回 1.0
    fn stability(&self) -> f32 {
        1.0
    }
}

impl NoteWindow for MusicalNoteEvent {
    fn window(&self) -> (f32, f32) {
        (self.start, self.end)
    }
    fn confidence(&self) -> f32 {
        self.confidence
    }
    fn midi_float(&self) -> f32 {
        self.midi_float
    }
    fn midi_rounded(&self) -> i32 {
        self.midi_rounded
    }
    fn stability(&self) -> f32 {
        // GAME 输出本身就是稳定平台
        1.0
    }
}

impl NoteWindow for crate::models::NoteEvent {
    fn window(&self) -> (f32, f32) {
        (self.start, self.end)
    }
    fn confidence(&self) -> f32 {
        self.confidence
    }
    fn midi_float(&self) -> f32 {
        self.center_midi.unwrap_or(self.midi as f32)
    }
    fn midi_rounded(&self) -> i32 {
        self.midi
    }
    fn stability(&self) -> f32 {
        let dur = self.duration().max(1e-3);
        if self.stable_duration > 0.0 {
            (self.stable_duration / dur).clamp(0.5, 1.0)
        } else {
            1.0
        }
    }
}

/// 音乐音符事件 (Musical Note Track 核心数据结构)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicalNoteEvent {
    pub id: u32,
    pub start: f32,
    pub end: f32,

    /// 连续中心音高 (浮点 MIDI)
    pub midi_float: f32,
    /// 最近半音 (取整 MIDI)
    pub midi_rounded: i32,
    /// 音名 (如 "C4", "A#4")
    pub note_name: String,

    pub confidence: f32,
    pub source: MusicalNoteSource,

    /// 边界置信度 (GAME 输出, 可选)
    #[serde(default)]
    pub boundary_confidence: Option<f32>,
    /// 是否连音/连音线 (可选)
    #[serde(default)]
    pub is_slur: Option<bool>,
}

impl MusicalNoteEvent {
    pub fn duration(&self) -> f32 {
        (self.end - self.start).max(0.0)
    }
}

/// 音乐音符转写引擎统一 trait
pub trait MusicalNoteEngine: Send + Sync {
    fn name(&self) -> &'static str;

    fn transcribe(
        &self,
        audio: &[f32],
        sample_rate: u32,
        optional_boundaries: Option<&[f32]>,
    ) -> Result<Vec<MusicalNoteEvent>, Box<dyn std::error::Error>>;
}
