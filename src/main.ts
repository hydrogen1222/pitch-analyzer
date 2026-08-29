import { invoke } from "@tauri-apps/api/core";
import { open, save, message, ask } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { PitchCanvas } from "./pitch_canvas";
import { KaraokeDisplay } from "./karaoke_display";
import { type PitchTrack, type AnalysisParams } from "./types";
import type { LyricLine } from "./models_lyrics";

// 检查是否在 Tauri 环境中运行
const isTauri = typeof window !== "undefined" && "__TAURI__" in window;

let state = {
  track: null as PitchTrack | null,
  lyrics: [] as LyricLine[],
  currentTime: 0,
  isPlaying: false,
  duration: 0,
};

let pitchCanvasEl: HTMLCanvasElement | null;
let karaokeDisplayEl: HTMLElement | null;
let karaokeHeaderEl: HTMLElement | null;
let playBtn: HTMLButtonElement | null;
let progressSlider: HTMLInputElement | null;
let volumeSlider: HTMLInputElement | null;
let timeDisplay: HTMLElement | null;
let statusEl: HTMLElement | null;
let importAudioBtn: HTMLButtonElement | null;
let importLrcBtn: HTMLButtonElement | null;
let importTxtBtn: HTMLButtonElement | null;
let clearLyricsBtn: HTMLButtonElement | null;
let saveProjBtn: HTMLButtonElement | null;
let loadProjBtn: HTMLButtonElement | null;
let exportSrtBtn: HTMLButtonElement | null;
let exportAssBtn: HTMLButtonElement | null;
let pitchFontInput: HTMLInputElement | null;
let lyricFontInput: HTMLInputElement | null;
let selectModelBtn: HTMLButtonElement | null;
let isAnalyzerInitialized = false;

let progressContainerEl: HTMLElement | null = null;
let progressFillEl: HTMLElement | null = null;

let pitchCanvas: PitchCanvas | null;
let karaokeDisplay: KaraokeDisplay | null;

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}



function getCurrentParams(): AnalysisParams {
  return {
    confidence_threshold: 0.3,
    fmin: 65,
    fmax: 1300,
    smoothing: 15,
    median_smoothing: 11,
    quantize: false,
    min_note_duration_ms: 45,
  };
}

function setStatus(text: string) {
  if (statusEl) {
    statusEl.textContent = text;
    statusEl.classList.toggle("warning-link", !isAnalyzerInitialized);
  }
}

function enableControls(hasTrack: boolean) {
  if (playBtn) playBtn.disabled = !hasTrack;
  if (progressSlider) progressSlider.disabled = !hasTrack;
  if (saveProjBtn) saveProjBtn.disabled = !hasTrack;
  if (exportSrtBtn) exportSrtBtn.disabled = !hasTrack;
  if (exportAssBtn) exportAssBtn.disabled = !hasTrack;
}

function updateTimeDisplay() {
  if (timeDisplay && state.duration > 0) {
    timeDisplay.textContent = `${formatTime(state.currentTime)} / ${formatTime(state.duration)}`;
  }
  if (progressSlider && state.duration > 0) {
    progressSlider.value = String(Math.floor((state.currentTime / state.duration) * 1000));
  }
}

/// 当前位置的平滑音高读数:
/// - ±2 帧 (50ms) 窗口内取中值 → 吸收单帧错误与边缘半音抖动
/// - 最近有限帧距离 > 2 帧视为真正的无声间隙 → 显示 ---
function smoothedMidiAt(track: PitchTrack, t: number): number | null {
  const { times, midis } = track;
  let lo = 0;
  let hi = times.length - 1;
  let idx = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (times[mid] <= t) {
      idx = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  const at = (k: number) => {
    const m = midis[k];
    return m !== undefined && isFinite(m) ? m : null;
  };
  if (at(idx) === null) {
    let nearest = Infinity;
    for (let k = Math.max(0, idx - 3); k <= Math.min(times.length - 1, idx + 3); k++) {
      if (at(k) !== null) nearest = Math.min(nearest, Math.abs(k - idx));
    }
    if (nearest > 2) return null;
  }
  const vals: number[] = [];
  for (let k = Math.max(0, idx - 2); k <= Math.min(times.length - 1, idx + 2); k++) {
    const v = at(k);
    if (v !== null) vals.push(v);
  }
  if (vals.length === 0) return null;
  vals.sort((a, b) => a - b);
  return vals[Math.floor(vals.length / 2)];
}

function updateCurrentPitch() {
  if (!state.track || !karaokeDisplay) return;
  const midi = smoothedMidiAt(state.track, state.currentTime);
  karaokeDisplay.setCurrentMidi(midi);
  if (pitchCanvas) pitchCanvas.setTime(state.currentTime);
}

async function doSelectModel(): Promise<boolean> {
  try {
    const modelPath = await open({
      title: "选择音高模型文件 (fcpe.onnx)",
      multiple: false,
      directory: false,
      filters: [{ name: "Model", extensions: ["onnx"] }],
    });
    if (!modelPath) return false;

    const configPath = await open({
      title: "选择模型配置文件 (fcpe_config.json)",
      multiple: false,
      directory: false,
      filters: [{ name: "Config", extensions: ["json"] }],
    });
    if (!configPath) return false;

    setStatus("正在加载外部模型...");
    await invoke("init_analyzer_with_paths", { configPath, modelPath });
    isAnalyzerInitialized = true;
    setStatus("就绪");
    await message("音高模型加载成功！", { title: "成功", kind: "info" });
    return true;
  } catch (e) {
    console.error("Select model failed:", e);
    setStatus("加载模型失败");
    await message("加载模型失败: " + e, { title: "错误", kind: "error" });
    return false;
  }
}

async function doImportAudio() {
  if (!isAnalyzerInitialized) {
    const confirmed = await ask("未载入音高模型。是否现在选择外部模型文件？\n\n(提示: 分析歌曲需要 fcpe.onnx 及 fcpe_config.json)", {
      title: "未载入音高模型",
      kind: "warning",
      okLabel: "选择模型",
      cancelLabel: "取消",
    });
    if (confirmed) {
      const ok = await doSelectModel();
      if (!ok) return;
    } else {
      return;
    }
  }

  try {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Audio", extensions: ["wav", "flac", "ogg", "mp3", "m4a"] },
        { name: "All", extensions: ["*"] },
      ],
    });
    if (!selected) return;

    if (progressContainerEl && progressFillEl) {
      progressContainerEl.style.display = "block";
      progressFillEl.style.width = "0%";
    }
    setStatus("正在分析...");

    const params = getCurrentParams();
    const track = (await invoke("analyze_audio", { audioPath: selected, params })) as PitchTrack;
    state.track = track;
    state.duration = (track.times && track.times.length > 0) ? track.times[track.times.length - 1] : 0;
    state.currentTime = 0;
    if (pitchCanvas) { pitchCanvas.setTrack(track); pitchCanvas.setTime(0); }
    enableControls(true);
    setStatus(`分析完成 (时长: ${formatTime(state.duration)})`);
  } catch (e) {
    console.error("Import audio failed:", e);
    setStatus("分析失败");
    await message("分析失败: " + e, { title: "错误", kind: "error" });
  } finally {
    setTimeout(() => {
      if (progressContainerEl) progressContainerEl.style.display = "none";
    }, 1500);
  }
}

async function doImportLrc() {
  try {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "LRC", extensions: ["lrc"] },
        { name: "All", extensions: ["*"] }
      ]
    });
    if (!selected) return;

    const lines = (await invoke("load_lyrics_lrc", { path: selected })) as LyricLine[];
    state.lyrics = lines;
    if (karaokeDisplay) karaokeDisplay.setLyrics(lines);
    if (clearLyricsBtn) clearLyricsBtn.disabled = lines.length === 0;
    setStatus(`已加载 ${lines.length} 行歌词`);
  } catch (e) {
    console.error("Import LRC failed:", e);
    await message("加载 LRC 失败: " + e, { title: "错误", kind: "error" });
  }
}

async function doImportTxt() {
  try {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Text", extensions: ["txt"] },
        { name: "All", extensions: ["*"] }
      ]
    });
    if (!selected) return;

    const lines = (await invoke("load_lyrics_txt", { path: selected })) as LyricLine[];
    state.lyrics = lines;
    if (karaokeDisplay) karaokeDisplay.setLyrics(lines);
    if (clearLyricsBtn) clearLyricsBtn.disabled = lines.length === 0;
    setStatus(`已加载 ${lines.length} 行歌词`);
  } catch (e) {
    console.error("Import TXT failed:", e);
    await message("加载 TXT 失败: " + e, { title: "错误", kind: "error" });
  }
}

async function doSaveProject() {
  try {
    const selected = await save({
      filters: [{ name: "Project", extensions: ["json"] }],
      defaultPath: "pitch.proj.json"
    });
    if (!selected) return;

    await invoke("save_project", { path: selected });
    setStatus("项目已保存");
  } catch (e) {
    console.error("Save project failed:", e);
    await message("保存失败: " + e, { title: "错误", kind: "error" });
  }
}

async function doLoadProject() {
  try {
    const selected = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: "Project", extensions: ["json"] },
        { name: "All", extensions: ["*"] }
      ]
    });
    if (!selected) return;

    const data = await invoke("load_project", { path: selected }) as { audio_path?: string; pitch_track?: PitchTrack; lyrics?: LyricLine[]; analysis_params?: AnalysisParams };
    if (data.pitch_track) {
      state.track = data.pitch_track;
      state.duration = (data.pitch_track.times && data.pitch_track.times.length > 0) ? data.pitch_track.times[data.pitch_track.times.length - 1] : 0;
      state.currentTime = 0;
      if (pitchCanvas) { pitchCanvas.setTrack(data.pitch_track); pitchCanvas.setTime(0); }
      enableControls(true);
    }
    state.lyrics = data.lyrics || [];
    if (karaokeDisplay) karaokeDisplay.setLyrics(state.lyrics);
    if (clearLyricsBtn) clearLyricsBtn.disabled = state.lyrics.length === 0;
    // 恢复分析参数的逻辑已被移除，因为这些参数已经作为系统最佳默认值硬编码
    setStatus("项目已加载");
  } catch (e) {
    console.error("Load project failed:", e);
    await message("加载失败: " + e, { title: "错误", kind: "error" });
  }
}

async function doExportSrt() {
  try {
    const selected = await save({
      filters: [{ name: "SRT", extensions: ["srt"] }],
      defaultPath: "pitch.srt"
    });
    if (!selected) return;

    await invoke("export_srt", { path: selected });
    setStatus("SRT 已导出");
  } catch (e) {
    console.error("Export SRT failed:", e);
    await message("导出失败: " + e, { title: "错误", kind: "error" });
  }
}

async function doExportAss() {
  try {
    const selected = await save({
      filters: [{ name: "ASS", extensions: ["ass"] }],
      defaultPath: "pitch.ass"
    });
    if (!selected) return;

    // 音高/歌词基础字号取自底部控制栏, 与卡拉OK预览保持一致
    const pitchFontSize = parseInt(pitchFontInput?.value || "48");
    const lyricFontSize = parseInt(lyricFontInput?.value || "18");
    await invoke("export_ass", { path: selected, pitchFontSize, lyricFontSize });
    setStatus("ASS 已导出 (可直接用 ffmpeg/libass 烧录)");
  } catch (e) {
    console.error("Export ASS failed:", e);
    await message("导出失败: " + e, { title: "错误", kind: "error" });
  }
}

async function initApp() {
  // 检查是否在 Tauri 中运行
  if (!isTauri) {
    console.error("Not running in Tauri environment!");
    const statusEl = document.querySelector("#status");
    if (statusEl) statusEl.textContent = "⚠️ 请在 Tauri 应用中运行";
    await message("请使用 `pnpm tauri dev` 来运行此应用，而不是直接在浏览器中打开！", {
      title: "错误",
      kind: "error"
    });
    return;
  }

  try {
    await invoke("init_analyzer");
    isAnalyzerInitialized = true;
    setStatus("就绪");
  } catch (e) {
    console.error("Init failed:", e);
    isAnalyzerInitialized = false;
    setStatus("未载入音高模型 (点击选择)");
  }

  try {
    await listen("analysis-progress", (event: any) => {
      const payload = event.payload as { progress: number; stage: string };
      if (progressContainerEl && progressFillEl) {
        progressContainerEl.style.display = "block";
        progressFillEl.style.width = `${payload.progress * 100}%`;
      }
      setStatus(`${payload.stage} (${Math.round(payload.progress * 100)}%)`);
    });
  } catch (e) {
    console.error("Listen to progress event failed:", e);
  }

  pitchCanvasEl = document.querySelector("#pitch-canvas");
  karaokeDisplayEl = document.querySelector("#karaoke-display");
  karaokeHeaderEl = document.querySelector("#karaoke-header");
  playBtn = document.querySelector("#play-btn");
  progressSlider = document.querySelector("#progress-slider");
  volumeSlider = document.querySelector("#volume-slider");
  timeDisplay = document.querySelector("#time-display");
  statusEl = document.querySelector("#status");
  importAudioBtn = document.querySelector("#import-audio");
  importLrcBtn = document.querySelector("#import-lrc");
  importTxtBtn = document.querySelector("#import-txt");
  clearLyricsBtn = document.querySelector("#clear-lyrics");
  saveProjBtn = document.querySelector("#save-proj");
  loadProjBtn = document.querySelector("#load-proj");
  exportSrtBtn = document.querySelector("#export-srt");
  exportAssBtn = document.querySelector("#export-ass");
  pitchFontInput = document.querySelector("#font-pitch");
  lyricFontInput = document.querySelector("#font-lyric");
  selectModelBtn = document.querySelector("#select-model");

  progressContainerEl = document.querySelector("#progress-container");
  progressFillEl = document.querySelector("#progress-fill");

  if (pitchCanvasEl) { pitchCanvas = new PitchCanvas(pitchCanvasEl); pitchCanvas.resize(); }
  if (karaokeDisplayEl && karaokeHeaderEl) {
    karaokeDisplay = new KaraokeDisplay(karaokeDisplayEl, karaokeHeaderEl, karaokeDisplayEl);
  }
  // Events
  window.addEventListener("resize", () => { if (pitchCanvas) pitchCanvas.resize(); });

  importAudioBtn?.addEventListener("click", doImportAudio);
  importLrcBtn?.addEventListener("click", doImportLrc);
  importTxtBtn?.addEventListener("click", doImportTxt);

  clearLyricsBtn?.addEventListener("click", async () => {
    try { await invoke("clear_lyrics"); } catch (_) {}
    state.lyrics = [];
    if (karaokeDisplay) karaokeDisplay.setLyrics([]);
    if (clearLyricsBtn) clearLyricsBtn.disabled = true;
    setStatus("歌词已清除");
  });

  saveProjBtn?.addEventListener("click", doSaveProject);
  loadProjBtn?.addEventListener("click", doLoadProject);
  exportSrtBtn?.addEventListener("click", doExportSrt);
  exportAssBtn?.addEventListener("click", doExportAss);
  selectModelBtn?.addEventListener("click", () => { doSelectModel(); });
  statusEl?.addEventListener("click", () => {
    if (!isAnalyzerInitialized) {
      doSelectModel();
    }
  });

  // Playback
  playBtn?.addEventListener("click", async () => {
    try {
      if (state.isPlaying) {
        await invoke("playback_pause");
        state.isPlaying = false;
        if (playBtn) playBtn.textContent = "▶";
      } else {
        await invoke("playback_play");
        state.isPlaying = true;
        if (playBtn) playBtn.textContent = "⏸";
      }
    } catch (e) { console.error(e); }
  });

  progressSlider?.addEventListener("input", async () => {
    if (state.duration > 0) {
      const value = parseFloat(progressSlider?.value || "0");
      const t = (value / 1000) * state.duration;
      state.currentTime = t;
      try { await invoke("playback_seek", { secs: t }); } catch (_) {}
      if (pitchCanvas) pitchCanvas.setTime(t);
      if (karaokeDisplay) karaokeDisplay.setTime(t);
      updateTimeDisplay();
    }
  });

  volumeSlider?.addEventListener("input", async () => {
    const v = parseFloat(volumeSlider?.value || "100") / 100;
    try { await invoke("playback_set_volume", { vol: v }); } catch (_) {}
  });

  pitchFontInput?.addEventListener("input", () => {
    if (karaokeDisplay) karaokeDisplay.setPitchFontSize(parseInt(pitchFontInput?.value || "48"));
  });
  lyricFontInput?.addEventListener("input", () => {
    if (karaokeDisplay) karaokeDisplay.setLyricFontSize(parseInt(lyricFontInput?.value || "18"));
  });

  // Animation loop — sync position from Rust player
  async function loop() {
    if (state.isPlaying) {
      try {
        state.currentTime = await invoke("playback_position") as number;
        const isStillPlaying = await invoke("playback_is_playing") as boolean;
        if (!isStillPlaying || state.currentTime >= state.duration) {
          state.isPlaying = false;
          if (playBtn) playBtn.textContent = "▶";
        }
      } catch (_) {}
    }
    // 歌词组件的时间必须每帧同步 (此前只在拖进度条时更新, 播放中歌词冻结)
    if (karaokeDisplay) karaokeDisplay.setTime(state.currentTime);
    updateCurrentPitch();
    updateTimeDisplay();
    if (pitchCanvas) pitchCanvas.draw();
    if (karaokeDisplay) karaokeDisplay.render();
    requestAnimationFrame(loop);
  }
  loop();
}

window.addEventListener("DOMContentLoaded", initApp);
