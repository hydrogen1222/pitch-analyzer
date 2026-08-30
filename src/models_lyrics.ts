
export interface NoteBinding {
  note_event_index: number;
  overlap_ms: number;
  overlap_ratio_token: number;
  overlap_ratio_note: number;
  score: number;
}

export interface ReadingSpan {
  surface: string;
  reading: string;
  pronunciation: string;
  char_start: number;
  char_end: number;
  display_start: number;
  display_end: number;
  mora_start: number;
  mora_end: number;
  confidence: number;
}

export interface RubyAnnotation {
  surface: string;
  reading: string;
  display_start: number;
  display_end: number;
  raw_start: number;
  raw_end: number;
}

export interface MoraUnit {
  kana: string;
  phonemes: string[];
  reading_span_id: number;
  char_start: number;
  char_end: number;
  reading_offset_start?: number;
  reading_offset_end?: number;
  display_start?: number;
  display_end?: number;
  display_group_id?: number;
  start_time?: number | null;
  end_time?: number | null;
  confidence?: number;
  note_bindings?: NoteBinding[];
}

export interface PitchNote {
  start_time: number;
  end_time: number;
  median_midi: number;
  mean_midi: number;
  rounded_midi: number;
  confidence_mean: number;
  point_count: number;
}

export interface ReadingDisplayGroup {
  id: number;
  char_start: number;
  char_end: number;
  reading_span_id: number;
  mora_start: number;
  mora_end: number;
  surface: string;
  reading: string;
  start_time?: number | null;
  end_time?: number | null;
  pitch_notes: PitchNote[];
  primary_note?: PitchNote | null;
  unpitched_reason?: string | null;
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
  /** 无音高原因: physical, alignment, or GAME↔FCPE evidence rejection */
  unpitched_reason?: string | null;
  reading_span_ids?: number[];
  reading_group_ids?: number[];
  char_start?: number;
  char_end?: number;
}

export interface LyricLine {
  text: string;
  start_time: number | null;
  end_time: number | null;
  tokens: LyricToken[];
  primary_text?: string;
  translations?: string[];
  token_timing_auto?: boolean;
  ruby_annotations?: RubyAnnotation[];
  reading_spans?: ReadingSpan[];
  reading_display_groups?: ReadingDisplayGroup[];
  moras?: MoraUnit[];
}
