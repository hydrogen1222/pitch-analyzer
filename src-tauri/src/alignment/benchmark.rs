// Gold benchmark framework (Round4 Commit 12, 任务书 §20 / §22 Tier D)
//
// schema (§20.1): testdata/alignment_gold/{ja,zh}/sample_XXX.json
//   { language, text, line_start, line_end,
//     units: [{text, start, end, nucleus_start, nucleus_end, notes:[midi]}] }
//
// 评估指标 (§20.4):
//   Boundary: Onset/Offset MAE, median, P90, within 30/50/100ms
//   Note ownership: micro precision / recall / F1 (gold unit notes vs matcher)
//   Unpitched classification: precision / recall
//   Fallback rate: EnhancedLrc/ForcedAlign/MoraDp/WeightedFallback/unaligned
//
// 回归门 (Tier D): 与 testdata/alignment_gold/baseline_report.json 比较,
// 关键指标显著退化 → 测试失败。真实模型基准 (GAME / singing FA) 通过
// 环境变量加载私有 fixture (PITCH_ANALYZER_TEST_AUDIO_ROOT), 不在 CI 下载。

use crate::diagnostics::AlignmentSourceCounts;
use crate::models::{LyricLine, PitchTrack, SingingLanguage};
use crate::note_engine::MusicalNoteEvent;
use serde::{Deserialize, Serialize};

// ── Gold schema ────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldSample {
    /// "ja" | "zh"
    pub language: String,
    pub text: String,
    pub line_start: f32,
    pub line_end: f32,
    pub units: Vec<GoldUnit>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldUnit {
    /// 发音单位文本 (日语 mora 假名 / 中文拼音)
    pub text: String,
    pub start: f32,
    pub end: f32,
    pub nucleus_start: f32,
    pub nucleus_end: f32,
    /// 该单位的真实音符 (MIDI); 空 = unpitched (促音/无声化)
    pub notes: Vec<f32>,
}

// ── 合成器: gold → PitchTrack (无模型环境下的确定性输入) ──────────
//
// 在 gold 定义的发音窗口内生成恒定音高 (每个 note 均分时长),
// 窗口外 unvoiced (NaN)。真实音频基准走 Tier C 环境变量路径。

pub fn synthesize_track_from_gold(gold: &GoldSample, hop_secs: f32) -> PitchTrack {
    let total = gold.line_end + 0.5;
    let n = (total / hop_secs).ceil() as usize;
    let mut track = PitchTrack {
        times: (0..n).map(|i| i as f32 * hop_secs).collect(),
        frequencies: vec![440.0; n],
        confidences: vec![0.95; n],
        midis: vec![f32::NAN; n],
        rms: vec![0.0; n],
        flux: Vec::new(),
        ..Default::default()
    };
    for unit in &gold.units {
        let span = (unit.end - unit.start).max(hop_secs);
        let note_count = unit.notes.len().max(1);
        let note_span = span / note_count as f32;
        for (ni, &midi) in unit.notes.iter().enumerate() {
            let from = unit.start + note_span * ni as f32;
            let to = from + note_span;
            for (i, &t) in track.times.iter().enumerate() {
                if t >= from && t < to {
                    track.midis[i] = midi;
                    track.rms[i] = 0.1;
                }
            }
        }
    }
    track
}

// ── 评估器 ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkReport {
    pub sample: String,
    pub language: String,
    pub unit_count: usize,
    /// gold unit 与 pipeline mora 成功配对的数量
    pub paired_count: usize,
    // NaN/Infinity 无法序列化进 JSON (serde_json 写成 null),
    // 因此"可能无值"的指标一律用 Option
    pub onset_mae_ms: Option<f32>,
    pub offset_mae_ms: Option<f32>,
    pub onset_median_ms: Option<f32>,
    pub onset_p90_ms: Option<f32>,
    pub onset_within_30ms: Option<f32>,
    pub onset_within_50ms: Option<f32>,
    pub onset_within_100ms: Option<f32>,
    pub note_precision: Option<f32>,
    pub note_recall: Option<f32>,
    pub note_f1: Option<f32>,
    pub unpitched_precision: Option<f32>,
    pub unpitched_recall: Option<f32>,
    /// 对齐来源占比 (0..1), 与 §20.4 fallback rate 对应
    pub fallback_enhanced_lrc: f32,
    pub fallback_forced_align: f32,
    pub fallback_mora_dp: f32,
    pub fallback_weighted_fallback: f32,
    pub fallback_unaligned: f32,
}

impl BenchmarkReport {
    /// §22 Tier D machine-readable 摘要
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "sample": self.sample,
            "onset_mae_ms": self.onset_mae_ms,
            "p90_ms": self.onset_p90_ms,
            "note_f1": self.note_f1,
            "fallback_rate": self.fallback_mora_dp + self.fallback_weighted_fallback,
        })
    }
}

/// 评估一行: gold units ↔ pipeline moras 按序配对 (fixture 与 pipeline
/// 分段一致; 不一致时 paired_count 反映出来, 由门禁发现)。
pub fn evaluate_line(
    sample: &str,
    line: &LyricLine,
    gold: &GoldSample,
    notes: &[MusicalNoteEvent],
) -> BenchmarkReport {
    let paired = line.moras.len().min(gold.units.len());
    let mut onset_errs: Vec<f32> = Vec::new();
    let mut offset_errs: Vec<f32> = Vec::new();
    let mut note_tp = 0usize;
    let mut note_pred = 0usize;
    let mut note_gold = 0usize;
    let mut unpitched_tp = 0usize;
    let mut unpitched_pred = 0usize;
    let mut unpitched_gold = 0usize;

    let midi_of_id: std::collections::HashMap<u32, f32> =
        notes.iter().map(|n| (n.id, n.midi_float)).collect();

    for (index, gold_unit) in gold.units.iter().enumerate().take(paired) {
        let mora = &line.moras[index];
        let attachment = line.pitch_attachments.get(index);
        let timed = matches!((mora.start_time, mora.end_time), (Some(s), Some(e)) if e > s);

        if timed {
            onset_errs.push(((mora.start_time.unwrap() - gold_unit.start).abs()) * 1000.0);
            offset_errs.push(((mora.end_time.unwrap() - gold_unit.end).abs()) * 1000.0);
        }

        // note ownership (按四舍五入后的 MIDI 集合比较)
        let gold_set: std::collections::BTreeSet<i32> =
            gold_unit.notes.iter().map(|m| m.round() as i32).collect();
        let got_set: std::collections::BTreeSet<i32> = attachment
            .map(|a| {
                a.note_ids
                    .iter()
                    .filter_map(|id| midi_of_id.get(id).map(|m| m.round() as i32))
                    .collect()
            })
            .unwrap_or_default();
        note_pred += got_set.len();
        note_gold += gold_set.len();
        note_tp += gold_set.intersection(&got_set).count();

        // unpitched 分类: gold 无 note ⇔ 预测无 note
        let gold_unpitched = gold_set.is_empty();
        let pred_unpitched = got_set.is_empty();
        if gold_unpitched {
            unpitched_gold += 1;
            if pred_unpitched {
                unpitched_tp += 1;
            }
        }
        if pred_unpitched {
            unpitched_pred += 1;
        }
    }

    let (onset_mae, onset_median, onset_p90, w30, w50, w100) = boundary_stats(&onset_errs);
    let offset_mae = if offset_errs.is_empty() {
        None
    } else {
        Some(offset_errs.iter().sum::<f32>() / offset_errs.len() as f32)
    };
    let precision = if note_pred == 0 {
        None
    } else {
        Some(note_tp as f32 / note_pred as f32)
    };
    let recall = if note_gold == 0 {
        None
    } else {
        Some(note_tp as f32 / note_gold as f32)
    };
    let f1 = match (precision, recall) {
        (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
        (Some(_), Some(_)) => Some(0.0),
        _ => None,
    };
    let unpitched_precision = if unpitched_pred == 0 {
        None
    } else {
        Some(unpitched_tp as f32 / unpitched_pred as f32)
    };
    let unpitched_recall = if unpitched_gold == 0 {
        None
    } else {
        Some(unpitched_tp as f32 / unpitched_gold as f32)
    };

    let sources: AlignmentSourceCounts =
        crate::diagnostics::summarize(std::slice::from_ref(line)).sources;
    let token_total = (sources.enhanced_lrc
        + sources.forced_align
        + sources.mora_dp
        + sources.weighted_fallback
        + sources.timed_without_source
        + sources.unaligned)
        .max(1) as f32;

    BenchmarkReport {
        sample: sample.to_string(),
        language: gold.language.clone(),
        unit_count: gold.units.len(),
        paired_count: paired,
        onset_mae_ms: onset_mae,
        offset_mae_ms: offset_mae,
        onset_median_ms: onset_median,
        onset_p90_ms: onset_p90,
        onset_within_30ms: w30,
        onset_within_50ms: w50,
        onset_within_100ms: w100,
        note_precision: precision,
        note_recall: recall,
        note_f1: f1,
        unpitched_precision,
        unpitched_recall,
        fallback_enhanced_lrc: sources.enhanced_lrc as f32 / token_total,
        fallback_forced_align: sources.forced_align as f32 / token_total,
        fallback_mora_dp: (sources.mora_dp + sources.timed_without_source) as f32 / token_total,
        fallback_weighted_fallback: sources.weighted_fallback as f32 / token_total,
        fallback_unaligned: sources.unaligned as f32 / token_total,
    }
}

fn boundary_stats(
    errs_ms: &[f32],
) -> (
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<f32>,
    Option<f32>,
) {
    if errs_ms.is_empty() {
        return (None, None, None, None, None, None);
    }
    let mut sorted = errs_ms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mae = errs_ms.iter().sum::<f32>() / errs_ms.len() as f32;
    let median = percentile(&sorted, 0.5);
    let p90 = percentile(&sorted, 0.9);
    let ratio =
        |bound: f32| errs_ms.iter().filter(|&&e| e <= bound).count() as f32 / errs_ms.len() as f32;
    (
        Some(mae),
        Some(median),
        Some(p90),
        Some(ratio(30.0)),
        Some(ratio(50.0)),
        Some(ratio(100.0)),
    )
}

fn percentile(sorted: &[f32], q: f32) -> f32 {
    let idx = (q * (sorted.len() - 1) as f32).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 语言字符串 → 工程语言 (fixture 加载辅助)
pub fn project_language_of(gold: &GoldSample) -> crate::models::ProjectLanguage {
    match gold.language.as_str() {
        "ja" => crate::models::ProjectLanguage::Japanese,
        "zh" => crate::models::ProjectLanguage::Mandarin,
        _ => crate::models::ProjectLanguage::Auto,
    }
}

pub fn singing_language_of(gold: &GoldSample) -> SingingLanguage {
    match gold.language.as_str() {
        "ja" => SingingLanguage::Japanese,
        "zh" => SingingLanguage::Mandarin,
        _ => SingingLanguage::Unknown,
    }
}

// ── 回归门 (Tier D) ────────────────────────────────────────────

pub struct RegressionTolerance {
    pub onset_mae_ms: f32,
    pub note_f1: f32,
}

impl Default for RegressionTolerance {
    fn default() -> Self {
        Self {
            onset_mae_ms: 25.0,
            note_f1: 0.05,
        }
    }
}

/// 关键指标显著退化 → Err (test fail)。NaN (无 note 类 fixture) 跳过 F1。
pub fn check_regression(
    current: &BenchmarkReport,
    baseline: &BenchmarkReport,
    tol: &RegressionTolerance,
) -> Result<(), String> {
    match (current.onset_mae_ms, baseline.onset_mae_ms) {
        (Some(cur), Some(base)) if cur > base + tol.onset_mae_ms => {
            return Err(format!(
                "regression gate: onset MAE {cur:.1}ms > baseline {base:.1}ms + {:.1}ms",
                tol.onset_mae_ms
            ));
        }
        (None, Some(_)) => {
            return Err(
                "regression gate: onset metrics disappeared (no aligned pairs)".to_string(),
            );
        }
        _ => {}
    }
    match (current.note_f1, baseline.note_f1) {
        (Some(cur), Some(base)) if cur < base - tol.note_f1 => {
            return Err(format!(
                "regression gate: note F1 {cur:.3} < baseline {base:.3} - {:.3}",
                tol.note_f1
            ));
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gold_round_trip_and_synthesis() {
        let gold: GoldSample = serde_json::from_str(
            r#"{
                "language": "ja",
                "text": "かあ",
                "line_start": 1.0,
                "line_end": 3.0,
                "units": [
                    {"text": "か", "start": 1.0, "end": 1.5, "nucleus_start": 1.05, "nucleus_end": 1.5, "notes": [60]},
                    {"text": "あ", "start": 1.5, "end": 2.0, "nucleus_start": 1.5, "nucleus_end": 2.0, "notes": [62]},
                    {"text": "っ", "start": 2.0, "end": 2.3, "nucleus_start": 2.0, "nucleus_end": 2.3, "notes": []}
                ]
            }"#,
        )
        .unwrap();
        let json = serde_json::to_string(&gold).unwrap();
        let restored: GoldSample = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.units.len(), 3);
        let track = synthesize_track_from_gold(&gold, 0.01);
        // 静音窗口: 1.0s 前 NaN
        let idx = (1.05 / 0.01) as usize;
        assert_eq!(track.midis[idx], 60.0);
        let gap_idx = (0.9 / 0.01) as usize;
        assert!(track.midis[gap_idx].is_nan());
        // melisma: 1.5-2.0 内均分 62 (single note)
        let idx2 = (1.6 / 0.01) as usize;
        assert_eq!(track.midis[idx2], 62.0);
    }
}
