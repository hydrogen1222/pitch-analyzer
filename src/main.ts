import { invoke } from "@tauri-apps/api/core";
import { open, save, message, ask } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { PitchCanvas } from "./pitch_canvas";
import { KaraokeDisplay } from "./karaoke_display";
import { PRESETS, type PitchTrack, type AnalysisParams } from "./types";
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
let presetBtns: NodeListOf<HTMLButtonElement> | null;
let presetDescEl: HTMLElement | null;
let confidenceInput: HTMLInputElement | null;
let fminInput: HTMLInputElement | null;
let fmaxInput: HTMLInputElement | null;
let quantizeInput: HTMLInputElement | null;
let medianInput: HTMLInputElement | null;
let smoothingInput: HTMLInputElement | null;
let advancedToggleBtn: HTMLButtonElement | null;
let advancedContentEl: HTMLElement | null;
let importAudioBtn: HTMLButtonElement | null;
let importLrcBtn: HTMLButtonElement | null;
let importTxtBtn: HTMLButtonElement | null;
let clearLyricsBtn: HTMLButtonElement | null;
let saveProjBtn: HTMLButtonElement | null;
let loadProjBtn: HTMLButtonElement | null;
let exportSrtBtn: HTMLButtonElement | null;
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

function applyPreset(name: string) {
  const preset = PRESETS[name];
  if (!preset) return;
  presetBtns?.forEach((btn) => btn.classList.toggle("active", btn.dataset.preset === name));
  if (confidenceInput) confidenceInput.value = String(preset.params.confidence_threshold);
  if (fminInput) fminInput.value = String(preset.params.fmin);
  if (fmaxInput) fmaxInput.value = String(preset.params.fmax);
  if (quantizeInput) quantizeInput.checked = preset.params.quantize;
  if (medianInput) medianInput.value = String(preset.params.median_smoothing);
  if (smoothingInput) smoothingInput.value = String(preset.params.smoothing);
  if (presetDescEl) presetDescEl.textContent = preset.description;
}

function getCurrentParams(): AnalysisParams {
  return {
    confidence_threshold: parseFloat(confidenceInput?.value || "0.3"),
    fmin: parseFloat(fminInput?.value || "65"),
    fmax: parseFloat(fmaxInput?.value || "1300"),
    smoothing: parseFloat(smoothingInput?.value || "15"),
    median_smoothing: parseFloat(medianInput?.value || "11"),
    quantize: quantizeInput?.checked || false,
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
}

function updateTimeDisplay() {
  if (timeDisplay && state.duration > 0) {
    timeDisplay.textContent = `${formatTime(state.currentTime)} / ${formatTime(state.duration)}`;
  }
  if (progressSlider && state.duration > 0) {
    progressSlider.value = String(Math.floor((state.currentTime / state.duration) * 1000));
  }
}

function updateCurrentPitch() {
  if (!state.track || !karaokeDisplay) return;
  const { times, midis } = state.track;
  let idx = times.findIndex((t) => t > state.currentTime);
  if (idx < 0) idx = times.length - 1;
  if (idx > 0) idx--;
  const midi = midis[idx];
  karaokeDisplay.setCurrentMidi(isFinite(midi) ? midi : null);
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
    state.duration = track.times[track.times.length - 1];
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
    if (clearLyricsBtn) clearLyricsBtn.disabled = false;
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
    if (clearLyricsBtn) clearLyricsBtn.disabled = false;
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

    const data = await invoke("load_project", { path: selected }) as { audio_path?: string; pitch_track?: PitchTrack; lyrics?: LyricLine[] };
    if (data.pitch_track) {
      state.track = data.pitch_track;
      state.duration = data.pitch_track.times[data.pitch_track.times.length - 1];
      state.currentTime = 0;
      if (pitchCanvas) { pitchCanvas.setTrack(data.pitch_track); pitchCanvas.setTime(0); }
      enableControls(true);
    }
    if (data.lyrics && data.lyrics.length > 0) {
      state.lyrics = data.lyrics;
      if (karaokeDisplay) karaokeDisplay.setLyrics(data.lyrics);
      if (clearLyricsBtn) clearLyricsBtn.disabled = false;
    }
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
  presetBtns = document.querySelectorAll(".preset-btn");
  presetDescEl = document.querySelector("#preset-desc");
  confidenceInput = document.querySelector("#confidence");
  fminInput = document.querySelector("#fmin");
  fmaxInput = document.querySelector("#fmax");
  quantizeInput = document.querySelector("#quantize");
  medianInput = document.querySelector("#median");
  smoothingInput = document.querySelector("#smoothing");
  advancedToggleBtn = document.querySelector("#advanced-toggle");
  advancedContentEl = document.querySelector("#advanced-content");
  importAudioBtn = document.querySelector("#import-audio");
  importLrcBtn = document.querySelector("#import-lrc");
  importTxtBtn = document.querySelector("#import-txt");
  clearLyricsBtn = document.querySelector("#clear-lyrics");
  saveProjBtn = document.querySelector("#save-proj");
  loadProjBtn = document.querySelector("#load-proj");
  exportSrtBtn = document.querySelector("#export-srt");
  pitchFontInput = document.querySelector("#font-pitch");
  lyricFontInput = document.querySelector("#font-lyric");
  selectModelBtn = document.querySelector("#select-model");

  progressContainerEl = document.querySelector("#progress-container");
  progressFillEl = document.querySelector("#progress-fill");

  if (pitchCanvasEl) { pitchCanvas = new PitchCanvas(pitchCanvasEl); pitchCanvas.resize(); }
  if (karaokeDisplayEl && karaokeHeaderEl) {
    karaokeDisplay = new KaraokeDisplay(karaokeDisplayEl, karaokeHeaderEl, karaokeDisplayEl);
  }
  applyPreset("pop");

  // Events
  window.addEventListener("resize", () => { if (pitchCanvas) pitchCanvas.resize(); });
  presetBtns?.forEach((btn) => btn.addEventListener("click", () => { const n = btn.dataset.preset; if (n) applyPreset(n); }));
  advancedToggleBtn?.addEventListener("click", () => {
    if (advancedContentEl) {
      const vis = advancedContentEl.style.display !== "none";
      advancedContentEl.style.display = vis ? "none" : "block";
      if (advancedToggleBtn) advancedToggleBtn.textContent = vis ? "展开" : "收起";
    }
  });

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
    updateCurrentPitch();
    updateTimeDisplay();
    if (pitchCanvas) pitchCanvas.draw();
    if (karaokeDisplay) karaokeDisplay.render();
    requestAnimationFrame(loop);
  }
  loop();
}

window.addEventListener("DOMContentLoaded", initApp);
