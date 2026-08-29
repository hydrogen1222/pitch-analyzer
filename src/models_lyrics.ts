
export interface PitchNote {
  start_time: number;
  end_time: number;
  median_midi: number;
  mean_midi: number;
  rounded_midi: number;
  confidence_mean: number;
  point_count: number;
}

export interface LyricToken {
  text: string;
  start_time: number | null;
  end_time: number | null;
  pitch_notes: PitchNote[];
  primary_note?: PitchNote | null;
  /** 对齐来源: enhanced_lrc / forced_align / mora_dp / weighted_fallback */
  alignment_source?: string | null;
  /** 对齐置信度 (已知读音占比, 0~1) */
  alignment_confidence?: number;
  /** 无音高原因: no_voicing / closure_or_devoicing / low_pitch_confidence / no_overlapping_note / alignment_missing */
  unpitched_reason?: string | null;
}

export interface LyricLine {
  text: string;
  start_time: number | null;
  end_time: number | null;
  tokens: LyricToken[];
  primary_text?: string;
  translations?: string[];
}
