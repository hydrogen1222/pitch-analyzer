// Legacy FCPE Note Tracker: 基于 FCPE 连续 F0 + NoteTracker v2 stable-plateau
// 作为 GAME 模型缺失时的标准 fallback 引擎。

use crate::models::NoteTrackingParams;
use crate::note_engine::{MusicalNoteEngine, MusicalNoteEvent, MusicalNoteSource};
use crate::note_tracker;

#[derive(Default)]
pub struct LegacyFcpeNoteTracker {
    pub params: NoteTrackingParams,
}

impl LegacyFcpeNoteTracker {
    pub fn new(params: NoteTrackingParams) -> Self {
        Self { params }
    }

    /// 从已计算的 F0 连续轨迹转写音符
    pub fn transcribe_from_track(
        &self,
        times: &[f32],
        midis: &[f32],
        confidences: &[f32],
    ) -> Vec<MusicalNoteEvent> {
        let events = note_tracker::build_note_events(times, midis, confidences, &self.params);
        events
            .into_iter()
            .enumerate()
            .map(|(i, ev)| MusicalNoteEvent {
                id: i as u32,
                start: ev.start,
                end: ev.end,
                midi_float: ev.center_midi.unwrap_or(ev.midi as f32),
                midi_rounded: ev.midi,
                note_name: ev.note_name,
                confidence: ev.confidence,
                source: MusicalNoteSource::LegacyFcpeTracker,
                boundary_confidence: None,
                is_slur: None,
            })
            .collect()
    }
}

impl MusicalNoteEngine for LegacyFcpeNoteTracker {
    fn name(&self) -> &'static str {
        "legacy-fcpe-v2"
    }

    fn transcribe(
        &self,
        _audio: &[f32],
        _sample_rate: u32,
        _optional_boundaries: Option<&[f32]>,
    ) -> Result<Vec<MusicalNoteEvent>, Box<dyn std::error::Error>> {
        // 注意: 若直接从 raw audio 驱动, 需前置 FCPE 提取 F0;
        // 推荐在 pipeline 中使用 transcribe_from_track 共享 F0 特征
        Err(
            "LegacyFcpeNoteTracker requires pre-extracted F0 track (use transcribe_from_track)"
                .into(),
        )
    }
}
