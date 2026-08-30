
import { LyricLine, PitchNote } from "./models_lyrics";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
export type PitchDisplayMode = "compact" | "musical-detail" | "debug-raw";

export class KaraokeDisplay {
  container: HTMLElement;
  headerEl: HTMLElement;
  displayEl: HTMLElement;

  lyricsLines: LyricLine[] = [];
  currentTime: number = 0;
  currentMidi: number | null = null;
  pitchFontSize: number = 48;
  lyricFontSize: number = 18;
  /// Compact / musically significant detail / debug raw. Raw events are
  /// intentionally only shown by the debug overlay.
  displayMode: PitchDisplayMode = "compact";

  get detailedPitch(): boolean {
    return this.displayMode === "musical-detail";
  }

  set detailedPitch(value: boolean) {
    this.displayMode = value ? "musical-detail" : "compact";
  }

  // 音名读数抗抖: vibrato 跨半音边界时 round() 会高频闪烁,
  // 候选音名需持续 noteHoldSecs 才替换 (MIDI 数值保持实时)
  private shownNote: number | null = null;
  private candidateNote: number | null = null;
  private candidateSince: number = 0;
  private noteHoldSecs: number = 0.07;

  /// Debug Overlay: 点击 token 时的回调 (line 下标, token 下标)
  onTokenDebug: ((lineIdx: number, tokenIdx: number) => void) | null = null;

  // 渲染缓存: 内容无变化时每帧只更新 MIDI 小字, 不重建整个 DOM
  private lyricsVersion: number = 0;
  private renderedKey: string = "";
  private midiInfoEl: HTMLElement | null = null;

  constructor(container: HTMLElement, headerEl: HTMLElement, displayEl: HTMLElement) {
    this.container = container;
    this.headerEl = headerEl;
    this.displayEl = displayEl;
  }

  setLyrics(lines: LyricLine[]) {
    this.lyricsLines = lines;
    this.lyricsVersion++;
  }

  setTime(time: number) {
    this.currentTime = time;
  }

  setCurrentMidi(midi: number | null) {
    if (midi === null || !isFinite(midi)) {
      this.currentMidi = null;
      this.shownNote = null;
      this.candidateNote = null;
      return;
    }
    this.currentMidi = midi;
    const rounded = Math.round(midi);
    if (this.shownNote === null) {
      this.shownNote = rounded;
      this.candidateNote = null;
      return;
    }
    if (rounded === this.shownNote) {
      this.candidateNote = null;
      return;
    }
    if (this.candidateNote === rounded) {
      if (this.currentTime - this.candidateSince >= this.noteHoldSecs) {
        this.shownNote = rounded;
        this.candidateNote = null;
      }
    } else {
      this.candidateNote = rounded;
      this.candidateSince = this.currentTime;
    }
  }

  /// 当前稳定音名 (MIDI 数字), null = 无声
  get stableNoteMidi(): number | null {
    return this.shownNote;
  }

  setPitchFontSize(size: number) {
    this.pitchFontSize = size;
  }

  setLyricFontSize(size: number) {
    this.lyricFontSize = size;
  }

  render() {
    const [lineIdx, tokenIdx] = this.findCurrentLineAndTokenIdx();
    // 无歌词 (纯音高) 模式下音高数字必须每帧跟随, 把当前音高并入缓存键;
    // 歌词模式下主内容与音高无关, 只每帧更新右下角小字
    const midiPart = lineIdx < 0 ? (this.currentMidi !== null ? this.currentMidi.toFixed(2) : "none") : "lyrics";
    const key = `${this.lyricsVersion}|${lineIdx}|${tokenIdx}|${this.pitchFontSize}|${this.lyricFontSize}|${this.displayMode}|${midiPart}`;
    if (key !== this.renderedKey) {
      this.renderedKey = key;
      this.midiInfoEl = null;
      this.displayEl.innerHTML = "";
      if (lineIdx >= 0) {
        this.renderLyrics(this.lyricsLines[lineIdx], tokenIdx);
      } else {
        this.renderPitchOnly();
      }
    }
    this.updateMidiInfo();
  }

  /// 每帧只更新右下角当前音高小字 (廉价 DOM 更新)
  private updateMidiInfo() {
    if (!this.midiInfoEl) return;
    if (this.currentMidi !== null && isFinite(this.currentMidi) && this.shownNote !== null) {
      const midiRounded = this.shownNote;
      const oct = Math.floor(midiRounded / 12) - 1;
      const noteName = NOTE_NAMES[((midiRounded % 12) + 12) % 12];
      this.midiInfoEl.textContent = `音高: ${noteName}${oct} (${this.currentMidi.toFixed(2)})`;
    } else {
      this.midiInfoEl.textContent = "";
    }
  }

  private renderPitchOnly() {
    this.headerEl.textContent = "当前音高";

    const wrap = document.createElement("div");
    wrap.style.display = "flex";
    wrap.style.flexDirection = "column";
    wrap.style.alignItems = "center";

    const noteEl = document.createElement("div");
    noteEl.className = "karaoke-pitch";
    noteEl.style.fontSize = `${this.pitchFontSize}px`;

    if (this.currentMidi !== null && isFinite(this.currentMidi)) {
      // 音名用保持后的稳定值 (抗 vibrato 闪烁), MIDI 数值保持实时
      if (this.shownNote !== null) {
        const midiRounded = this.shownNote;
        const oct = Math.floor(midiRounded / 12) - 1;
        const noteName = NOTE_NAMES[((midiRounded % 12) + 12) % 12];
        noteEl.textContent = `${noteName}${oct}`;
      } else {
        noteEl.textContent = "---";
      }

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

  private renderLyrics(line: LyricLine, currentTokenIdx: number) {
    this.headerEl.textContent = "♪ 当前歌词";

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
    const groups = this.displayGroups(line);

    groups.forEach(({ tokenIndices, group }) => {
      const w = tokenIndices.reduce((sum, index) => sum + tokenWidths[index], 0);
      const isActive = tokenIndices.includes(currentTokenIdx);
      const leadToken = line.tokens[tokenIndices[0]];

      const noteBox = document.createElement("div");
      noteBox.style.width = `${w}px`;
      noteBox.style.display = "flex";
      noteBox.style.justifyContent = "center";
      noteBox.style.alignItems = "center";

      // Compact uses the evidence-backed primary. Musical detail uses only
      // consolidated significant notes; raw GAME/FCPE events stay in debug.
      const notes = group?.pitch_notes?.length ? group.pitch_notes : leadToken.pitch_notes;
      const primary = group?.primary_note ?? leadToken.primary_note ?? this.bestNote(leadToken);
      if (this.displayMode === "musical-detail" && notes.length > 1) {
        const names = this.formatMusicalDetail(notes);
        const noteEl = document.createElement("span");
        noteEl.style.padding = "4px 8px";
        noteEl.style.borderRadius = "6px";
        noteEl.style.backgroundColor = isActive
          ? "rgba(0, 212, 170, 0.55)"
          : "rgba(0, 212, 170, 0.2)";
        noteEl.style.color = isActive ? "#ffffff" : "#e6fff9";
        noteEl.style.fontWeight = "700";
        noteEl.style.fontSize = `${Math.max(9, Math.floor(this.lyricFontSize * 0.42))}px`;
        noteEl.style.whiteSpace = "nowrap";
        noteEl.textContent = names;
        noteBox.appendChild(noteEl);
      } else if (this.displayMode !== "debug-raw" && primary) {
        const noteEl = document.createElement("span");
        noteEl.style.padding = "4px 10px";
        noteEl.style.borderRadius = "6px";
        if (isActive) {
          noteEl.style.backgroundColor = "rgba(0, 212, 170, 0.55)";
          noteEl.style.color = "#ffffff";
          noteEl.style.transform = "scale(1.05)";
          noteEl.style.transition = "all 0.1s ease";
        } else {
          noteEl.style.backgroundColor = "rgba(0, 212, 170, 0.2)";
          noteEl.style.color = "#e6fff9";
        }
        noteEl.style.fontWeight = "700";
        noteEl.style.fontSize = `${Math.max(10, Math.floor(this.lyricFontSize * 0.55))}px`;
        const midiRounded = Math.round(primary.median_midi);
        const oct = Math.floor(midiRounded / 12) - 1;
        const noteName = NOTE_NAMES[((midiRounded % 12) + 12) % 12];
        noteEl.textContent = `${noteName}${oct}`;
        noteBox.appendChild(noteEl);
      }

      notesRow.appendChild(noteBox);

      const groupLyrics = document.createElement("span");
      groupLyrics.style.width = `${w}px`;
      groupLyrics.style.display = "inline-flex";
      groupLyrics.style.justifyContent = "center";
      tokenIndices.forEach((i) => {
        const token = line.tokens[i];
        const tokenEl = document.createElement("span");
        tokenEl.style.width = `${tokenWidths[i]}px`;
        tokenEl.style.textAlign = "center";
        tokenEl.style.fontSize = `${this.lyricFontSize}px`;
        tokenEl.style.fontWeight = "700";
        tokenEl.style.color = i === currentTokenIdx ? "#00d4aa" : "#e6e6e6";
        tokenEl.textContent = token.text.split("|")[0];
        tokenEl.style.cursor = "pointer";
        const li = this.lyricsLines.indexOf(line);
        tokenEl.onclick = () => this.onTokenDebug?.(li, i);
        groupLyrics.appendChild(tokenEl);
      });
      lyricsRow.appendChild(groupLyrics);
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
    this.midiInfoEl = bottomRightInfo;

    this.displayEl.appendChild(wrap);
    this.displayEl.appendChild(bottomRightInfo);
  }

  // 与后端 select_primary_note 一致的评分回退: duration × confidence
  private bestNote(token: { pitch_notes: PitchNote[] }): PitchNote | null {
    let best = null;
    let bestScore = -Infinity;
    for (const n of token.pitch_notes) {
      const score = (n.end_time - n.start_time) * n.confidence_mean;
      if (score > bestScore) {
        bestScore = score;
        best = n;
      }
    }
    return best;
  }

  private formatMusicalDetail(notes: PitchNote[]): string {
    const names: string[] = [];
    for (const note of notes) {
      const rounded = Math.round(note.median_midi);
      const name = `${NOTE_NAMES[((rounded % 12) + 12) % 12]}${Math.floor(rounded / 12) - 1}`;
      if (names[names.length - 1] !== name) names.push(name);
    }
    if (names.length <= 3) return names.join("→");
    return `${names[0]}→…→${names[names.length - 1]} ×${names.length}`;
  }

  private displayGroups(line: LyricLine): Array<{
    tokenIndices: number[];
    group?: NonNullable<LyricLine["reading_display_groups"]>[number];
  }> {
    const sourceGroups = line.reading_display_groups ?? [];
    const used = new Set<number>();
    const result: Array<{
      tokenIndices: number[];
      group?: NonNullable<LyricLine["reading_display_groups"]>[number];
    }> = [];
    for (const group of sourceGroups) {
      const tokenIndices = line.tokens
        .map((token, index) => ({ token, index }))
        .filter(({ token }) =>
          (token.char_start ?? 0) < group.char_end && group.char_start < (token.char_end ?? 0),
        )
        .map(({ index }) => index);
      if (tokenIndices.length > 0) {
        tokenIndices.forEach((index) => used.add(index));
        result.push({ tokenIndices, group });
      }
    }
    line.tokens.forEach((_, index) => {
      if (!used.has(index)) result.push({ tokenIndices: [index] });
    });
    result.sort((a, b) => a.tokenIndices[0] - b.tokenIndices[0]);
    return result;
  }

  private findCurrentLineAndTokenIdx(): [number, number] {
    for (let li = 0; li < this.lyricsLines.length; li++) {
      const line = this.lyricsLines[li];
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
      return [li, currentTokenIdx];
    }
    return [-1, -1];
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
