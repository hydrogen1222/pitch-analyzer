
import { LyricLine, LyricToken, PitchNote } from "./models_lyrics";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export class KaraokeDisplay {
  container: HTMLElement;
  headerEl: HTMLElement;
  displayEl: HTMLElement;

  lyricsLines: LyricLine[] = [];
  currentTime: number = 0;
  currentMidi: number | null = null;
  pitchFontSize: number = 48;
  lyricFontSize: number = 18;

  constructor(container: HTMLElement, headerEl: HTMLElement, displayEl: HTMLElement) {
    this.container = container;
    this.headerEl = headerEl;
    this.displayEl = displayEl;
  }

  setLyrics(lines: LyricLine[]) {
    this.lyricsLines = lines;
  }

  setTime(time: number) {
    this.currentTime = time;
  }

  setCurrentMidi(midi: number | null) {
    this.currentMidi = midi;
  }

  setPitchFontSize(size: number) {
    this.pitchFontSize = size;
  }

  setLyricFontSize(size: number) {
    this.lyricFontSize = size;
  }

  render() {
    if (this.lyricsLines.length > 0) {
      this.renderLyrics();
    } else {
      this.renderPitchOnly();
    }
  }

  private renderPitchOnly() {
    this.headerEl.textContent = "当前音高";
    this.displayEl.innerHTML = "";

    const wrap = document.createElement("div");
    wrap.style.display = "flex";
    wrap.style.flexDirection = "column";
    wrap.style.alignItems = "center";

    const noteEl = document.createElement("div");
    noteEl.className = "karaoke-pitch";
    noteEl.style.fontSize = `${this.pitchFontSize}px`;

    if (this.currentMidi !== null && isFinite(this.currentMidi)) {
      const midiRounded = Math.round(this.currentMidi);
      const oct = Math.floor(midiRounded / 12) - 1;
      const noteName = NOTE_NAMES[midiRounded % 12];
      noteEl.textContent = `${noteName}${oct}`;

      const midiEl = document.createElement("div");
      midiEl.className = "karaoke-midi";
      midiEl.style.fontSize = `${Math.max(11, Math.floor(this.pitchFontSize * 0.35))}px`;
      midiEl.textContent = `MIDI: ${this.currentMidi.toFixed(2)}`;

      wrap.appendChild(noteEl);
      wrap.appendChild(midiEl);
    } else {
      noteEl.textContent = "---";
      wrap.appendChild(noteEl);
    }

    this.displayEl.appendChild(wrap);
  }

  private renderLyrics() {
    this.headerEl.textContent = "♪ 当前歌词";
    this.displayEl.innerHTML = "";

    const currentLineAndIdx = this.findCurrentLineAndToken();
    if (!currentLineAndIdx) {
      this.renderPitchOnly();
      return;
    }
    const [line, currentTokenIdx] = currentLineAndIdx;

    const wrap = document.createElement("div");
    wrap.style.display = "flex";
    wrap.style.flexDirection = "column";
    wrap.style.alignItems = "center";
    wrap.style.gap = "8px";

    const notesRow = document.createElement("div");
    notesRow.style.display = "flex";
    notesRow.style.alignItems = "center";
    notesRow.style.gap = "8px";

    const lyricsRow = document.createElement("div");
    lyricsRow.style.display = "flex";
    lyricsRow.style.alignItems = "center";
    lyricsRow.style.gap = "8px";

    const tokenWidths = this.calculateTokenWidths(line);

    line.tokens.forEach((token, i) => {
      const text = token.text.split("|")[0];
      const w = tokenWidths[i];

      const noteBox = document.createElement("div");
      noteBox.style.width = `${w}px`;
      noteBox.style.display = "flex";
      noteBox.style.justifyContent = "center";
      noteBox.style.alignItems = "center";

      if (token.pitch_notes.length > 0) {
        const noteEl = document.createElement("span");
        noteEl.style.padding = "4px 10px";
        noteEl.style.borderRadius = "6px";
        noteEl.style.backgroundColor = "rgba(0, 212, 170, 0.2)";
        noteEl.style.color = "#e6fff9";
        noteEl.style.fontWeight = "700";
        noteEl.style.fontSize = `${Math.max(10, Math.floor(this.lyricFontSize * 0.55))}px`;
        const midiRounded = Math.round(token.pitch_notes[0].median_midi);
        const oct = Math.floor(midiRounded / 12) - 1;
        const noteName = NOTE_NAMES[midiRounded % 12];
        noteEl.textContent = `${noteName}${oct}`;
        noteBox.appendChild(noteEl);
      }

      notesRow.appendChild(noteBox);

      const tokenEl = document.createElement("span");
      tokenEl.style.width = `${w}px`;
      tokenEl.style.textAlign = "center";
      tokenEl.style.fontSize = `${this.lyricFontSize}px`;
      tokenEl.style.fontWeight = "700";
      tokenEl.style.color = "#e6e6e6";
      tokenEl.textContent = text;
      lyricsRow.appendChild(tokenEl);
    });

    wrap.appendChild(notesRow);
    wrap.appendChild(lyricsRow);

    if (line.translations && line.translations.length > 0) {
      const transEl = document.createElement("div");
      transEl.style.fontSize = `${Math.max(11, Math.floor(this.lyricFontSize * 0.6))}px`;
      transEl.style.color = "#a0c8e0";
      transEl.style.marginTop = "6px";
      transEl.textContent = line.translations.join(" / ");
      wrap.appendChild(transEl);
    }

    const bottomRightInfo = document.createElement("div");
    bottomRightInfo.style.position = "absolute";
    bottomRightInfo.style.bottom = "16px";
    bottomRightInfo.style.right = "20px";
    bottomRightInfo.style.fontSize = "11px";
    bottomRightInfo.style.color = "#888";

    if (this.currentMidi !== null && isFinite(this.currentMidi)) {
      const midiRounded = Math.round(this.currentMidi);
      const oct = Math.floor(midiRounded / 12) - 1;
      const noteName = NOTE_NAMES[midiRounded % 12];
      bottomRightInfo.textContent = `音高: ${noteName}${oct} (${this.currentMidi.toFixed(2)})`;
    }

    this.displayEl.appendChild(wrap);
    this.displayEl.appendChild(bottomRightInfo);
  }

  private findCurrentLineAndToken(): [LyricLine, number] | null {
    for (const line of this.lyricsLines) {
      if (line.start_time === null || line.end_time === null) continue;
      if (this.currentTime < line.start_time || this.currentTime > line.end_time) continue;

      let currentTokenIdx = -1;
      for (let i = 0; i < line.tokens.length; i++) {
        const token = line.tokens[i];
        if (token.start_time === null || token.end_time === null) continue;
        if (this.currentTime >= token.start_time && this.currentTime <= token.end_time) {
          currentTokenIdx = i;
          break;
        }
      }
      return [line, currentTokenIdx];
    }
    return null;
  }

  private calculateTokenWidths(line: LyricLine): number[] {
    const totalChars = line.tokens.reduce((s, t) => s + t.text.split("|")[0].length, 0);
    const maxW = 100;
    const minW = 36;
    const baseW = Math.min(maxW, Math.max(minW, Math.floor(600 / (line.tokens.length || 1))));

    const widths: number[] = [];
    for (const t of line.tokens) {
      const text = t.text.split("|")[0];
      const charCount = text.length;
      let w = baseW;
      if (totalChars > 0) {
        const ratio = charCount / (totalChars / line.tokens.length);
        w = Math.max(minW, Math.min(maxW, Math.floor(baseW * Math.min(ratio, 1.5))));
      }
      widths.push(w);
    }
    return widths;
  }
}
