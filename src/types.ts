
export interface PitchTrack {
  times: number[];
  frequencies: number[];
  confidences: number[];
  midis: number[];
}

export interface AnalysisParams {
  confidence_threshold: number;
  fmin: number;
  fmax: number;
  smoothing: number;
  median_smoothing: number;
  quantize: boolean;
}

export interface Preset {
  name: string;
  description: string;
  params: AnalysisParams;
}

export const PRESETS: Record<string, Preset> = {
  pop: {
    name: "流行",
    description: "适合流行音乐人声分析",
    params: {
      confidence_threshold: 0.3,
      fmin: 65,
      fmax: 1300,
      smoothing: 15,
      median_smoothing: 11,
      quantize: false,
    },
  },
  folk: {
    name: "民谣",
    description: "适合民谣、清唱人声分析",
    params: {
      confidence_threshold: 0.25,
      fmin: 82,
      fmax: 1047,
      smoothing: 17,
      median_smoothing: 13,
      quantize: false,
    },
  },
  classical: {
    name: "古典",
    description: "适合美声、古典声乐",
    params: {
      confidence_threshold: 0.2,
      fmin: 65,
      fmax: 1568,
      smoothing: 21,
      median_smoothing: 15,
      quantize: true,
    },
  },
};
