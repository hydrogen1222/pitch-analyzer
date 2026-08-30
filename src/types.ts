
export interface PitchTrack {
  times: number[];
  frequencies: number[];
  confidences: number[];
  midis: number[];
  rms?: number[];
  flux?: number[];
  note_events?: NoteEvent[];
  /** Canonical musical-note track; note_events is retained for compatibility. */
  musical_notes?: MusicalNoteEvent[];
  musical_note_source?: "Game" | "LegacyFcpeTracker" | "ImportedMidi";
  musical_note_model?: string | null;
  raw_game_notes?: MusicalNoteEvent[];
  canonical_sung_notes?: CanonicalSungNote[];
}

export interface MusicalNoteEvent {
  id: number;
  start: number;
  end: number;
  midi_float: number;
  midi_rounded: number;
  note_name: string;
  confidence: number;
  source: "Game" | "LegacyFcpeTracker" | "ImportedMidi";
  model_confidence?: number | null;
  boundary_confidence?: number | null;
  is_slur?: boolean | null;
  evidence?: NoteEvidence | null;
  class?: "Stable" | "Ornament" | "Transition" | "Uncertain" | null;
}

export interface NoteEvidence {
  fcpe_frame_count: number;
  voiced_coverage: number;
  median_midi?: number | null;
  midi_mad_cents?: number | null;
  pitch_delta_cents?: number | null;
  support_score: number;
}

export interface CanonicalSungNote {
  event_ids: number[];
  start: number;
  end: number;
  game_midi: number;
  fcpe_median_midi?: number | null;
  display_midi: number;
  voiced_coverage: number;
  fcpe_support: number;
  confidence: number;
  class: "Stable" | "Ornament" | "Transition" | "Uncertain";
  evidence: NoteEvidence;
}

export interface NoteEvent {
  start: number;
  end: number;
  midi: number;
  note_name: string;
  confidence: number;
  center_midi?: number | null;
  stable_duration?: number;
  tracker_version?: number;
}

export interface AnalysisParams {
  confidence_threshold: number;
  fmin: number;
  fmax: number;
  smoothing: number;
  median_smoothing: number;
  quantize: boolean;
  min_note_duration_ms: number;
}

export interface Preset {
  name: string;
  description: string;
  params: AnalysisParams;
}

