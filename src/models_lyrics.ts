
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
}

export interface LyricLine {
  text: string;
  start_time: number | null;
  end_time: number | null;
  tokens: LyricToken[];
  primary_text?: string;
  translations?: string[];
}
