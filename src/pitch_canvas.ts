
import { PitchTrack } from "./types";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const BLACK_KEY_INDICES = new Set([1, 3, 6, 8, 10]);

export class PitchCanvas {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  track: PitchTrack | null = null;
  currentTime: number = 0;
  audioData: Float32Array | null = null;
  audioSampleRate: number = 44100;

  private viewMinMidi: number = 40;
  private viewMaxMidi: number = 80;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Failed to get 2d context");
    this.ctx = ctx;
  }

  resize() {
    const dpr = window.devicePixelRatio || 1;
    const rect = this.canvas.getBoundingClientRect();
    this.canvas.width = rect.width * dpr;
    this.canvas.height = rect.height * dpr;
    this.ctx.scale(dpr, dpr);
  }

  setTrack(track: PitchTrack) {
    this.track = track;
    this.autoFitView();
  }

  setAudioData(data: Float32Array, sampleRate: number) {
    this.audioData = data;
    this.audioSampleRate = sampleRate;
  }

  setTime(time: number) {
    this.currentTime = time;
  }

  private autoFitView() {
    if (!this.track) return;
    let min = Infinity;
    let max = -Infinity;
    for (const m of this.track.midis) {
      if (isFinite(m)) {
        if (m < min) min = m;
        if (m > max) max = m;
      }
    }
    if (!isFinite(min) || !isFinite(max)) {
      min = 40;
      max = 80;
    }
    this.viewMinMidi = Math.floor(min) - 5;
    this.viewMaxMidi = Math.ceil(max) + 5;
    if (this.viewMaxMidi - this.viewMinMidi < 14) {
      const center = (this.viewMinMidi + this.viewMaxMidi) / 2;
      this.viewMinMidi = center - 7;
      this.viewMaxMidi = center + 7;
    }
  }

  draw() {
    const ctx = this.ctx;
    const rect = this.canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    const topH = h * 0.12;
    const mainH = h - topH;

    ctx.clearRect(0, 0, w, h);

    const audioRect = { x: 0, y: 0, w, h: topH };
    const mainRect = { x: 0, y: topH, w, h: mainH };

    this.drawAudioWaveform(audioRect);
    this.drawPianoRollBackground(mainRect);
    this.drawPitchTrack(mainRect);
    this.drawPlayCursor(rect, this.currentTime);
  }

  private drawAudioWaveform(rect: { x: number; y: number; w: number; h: number }) {
    const ctx = this.ctx;
    const { x, y, w, h } = rect;

    ctx.fillStyle = "#1e1e1e";
    ctx.fillRect(x, y, w, h);

    if (!this.track || !this.audioData) return;

    const duration = this.track.times[this.track.times.length - 1];
    const samplesPerPixel = this.audioData.length / w;
    const mid = y + h / 2;

    ctx.strokeStyle = "rgba(160, 160, 160, 0.5)";
    ctx.lineWidth = 1;
    ctx.beginPath();

    for (let px = 0; px < w; px++) {
      const start = Math.floor(px * samplesPerPixel);
      const end = Math.floor((px + 1) * samplesPerPixel);
      if (start >= this.audioData.length) break;

      let min = 0;
      let max = 0;
      for (let i = start; i < end && i < this.audioData.length; i++) {
        const v = this.audioData[i];
        if (v < min) min = v;
        if (v > max) max = v;
      }

      const yMin = mid + min * (h * 0.4);
      const yMax = mid + max * (h * 0.4);
      ctx.moveTo(x + px, yMin);
      ctx.lineTo(x + px, yMax);
    }
    ctx.stroke();
  }

  private drawPianoRollBackground(rect: { x: number; y: number; w: number; h: number }) {
    const ctx = this.ctx;
    const { x, y, w, h } = rect;

    const midiStart = Math.floor(this.viewMinMidi);
    const midiEnd = Math.ceil(this.viewMaxMidi);
    const nNotes = midiEnd - midiStart;

    for (let m = midiStart; m <= midiEnd; m++) {
      const noteIdx = m % 12;
      const noteY = y + h * (1 - (m - this.viewMinMidi) / nNotes);
      const noteH = h / nNotes;

      if (noteIdx === 0) {
        ctx.fillStyle = "#23232c";
      } else if (BLACK_KEY_INDICES.has(noteIdx)) {
        ctx.fillStyle = "#161616";
      } else {
        ctx.fillStyle = "#1c1c1c";
      }
      ctx.fillRect(x, noteY - noteH, w, noteH);

      ctx.strokeStyle = "#323232";
      ctx.lineWidth = noteIdx === 0 ? 2 : 1;
      ctx.beginPath();
      ctx.moveTo(x, noteY);
      ctx.lineTo(x + w, noteY);
      ctx.stroke();
    }

    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    ctx.fillStyle = "#707070";
    ctx.font = "12px 'Segoe UI', sans-serif";
    for (let m = midiStart; m <= midiEnd; m += 1) {
      if (m % 12 !== 0 && m % 12 !== 3 && m % 12 !== 7) continue;
      const noteIdx = m % 12;
      const oct = Math.floor(m / 12) - 1;
      const noteY = y + h * (1 - (m - this.viewMinMidi) / nNotes);
      ctx.fillText(`${NOTE_NAMES[noteIdx]}${oct}`, x + 45, noteY - h / nNotes / 2);
    }
  }

  private drawPitchTrack(rect: { x: number; y: number; w: number; h: number }) {
    const ctx = this.ctx;
    const { x, y, w, h } = rect;
    if (!this.track || this.track.times.length === 0) return;

    const duration = this.track.times[this.track.times.length - 1];
    const midiRange = this.viewMaxMidi - this.viewMinMidi;

    // First pass: draw the connecting lines
    ctx.strokeStyle = "rgba(0, 212, 170, 0.9)";
    ctx.lineWidth = 3;
    ctx.beginPath();

    let lineStarted = false;
    for (let i = 0; i < this.track.times.length; i++) {
      const t = this.track.times[i];
      const m = this.track.midis[i];
      const px = x + (t / duration) * w;
      const py = y + h * (1 - (m - this.viewMinMidi) / midiRange);

      if (!isFinite(m)) {
        if (lineStarted) {
          ctx.stroke();
          ctx.beginPath();
          lineStarted = false;
        }
        continue;
      }

      if (!lineStarted) {
        ctx.moveTo(px, py);
        lineStarted = true;
      } else {
        ctx.lineTo(px, py);
      }
    }

    if (lineStarted) {
      ctx.stroke();
    }

    // Second pass: draw the circular points
    ctx.fillStyle = "rgba(0, 212, 170, 0.4)";
    for (let i = 0; i < this.track.times.length; i++) {
      const t = this.track.times[i];
      const m = this.track.midis[i];
      if (!isFinite(m)) continue;

      const px = x + (t / duration) * w;
      const py = y + h * (1 - (m - this.viewMinMidi) / midiRange);
      ctx.beginPath();
      ctx.arc(px, py, 4, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  private drawPlayCursor(rect: { x: number; y: number; w: number; h: number }, time: number) {
    const ctx = this.ctx;
    if (!this.track) return;
    const duration = this.track.times[this.track.times.length - 1];
    const px = (time / duration) * rect.w;

    ctx.strokeStyle = "#ff5252";
    ctx.lineWidth = 3;
    ctx.beginPath();
    ctx.moveTo(px, 0);
    ctx.lineTo(px, rect.h);
    ctx.stroke();
  }

  timeToPixel(time: number): number {
    if (!this.track || this.track.times.length === 0) return 0;
    const duration = this.track.times[this.track.times.length - 1];
    if (!isFinite(duration) || duration <= 0) return 0;
    const rect = this.canvas.getBoundingClientRect();
    return (time / duration) * rect.width;
  }

  pixelToTime(px: number): number {
    if (!this.track || this.track.times.length === 0) return 0;
    const duration = this.track.times[this.track.times.length - 1];
    if (!isFinite(duration) || duration <= 0) return 0;
    const rect = this.canvas.getBoundingClientRect();
    return (px / rect.width) * duration;
  }
}
