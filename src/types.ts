
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
  boundary_confidence?: number | null;
  is_slur?: boolean | null;
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

