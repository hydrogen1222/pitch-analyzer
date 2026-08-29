
export interface PitchTrack {
  times: number[];
  frequencies: number[];
  confidences: number[];
  midis: number[];
  rms?: number[];
  note_events?: NoteEvent[];
}

export interface NoteEvent {
  start: number;
  end: number;
  midi: number;
  note_name: string;
  confidence: number;
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

