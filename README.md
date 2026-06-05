<a id="top"></a>

<div align="center">

# 🎵 Pitch Analyzer / 人声音高测量工具

**An intelligent desktop application for vocal pitch analysis, visualization, and karaoke subtitle labeling.**

**基于 AI 深度学习的人声音高分析、钢琴卷帘可视化与卡拉OK歌词自动对齐工具。**

[FCPE](https://github.com/CNChTu/FCPE) Pitch Engine · Tauri 2 + Rust · Cross-Platform Desktop App

[English](#english) · [中文](#中文)

</div>

---

## 📖 Table of Contents / 目录

<details open>
<summary><b>English Table of Contents</b></summary>

- [🎯 What Is This?](#what-is-this)
- [✨ Key Features](#key-features)
- [🚀 Quick Start (For Users)](#quick-start-for-users)
  - [Prerequisites](#prerequisites)
  - [Step 1: Download & Install](#step-1-download--install)
  - [Step 2: Load the AI Model](#step-2-load-the-ai-model)
  - [Step 3: Setup ONNX Runtime](#step-3-setup-onnx-runtime)
- [🎮 Step-by-Step Tutorial](#step-by-step-tutorial)
- [⚙️ Parameter Tuning (Plain English)](#parameter-tuning-plain-english)
- [🔧 Easy Troubleshooting](#easy-troubleshooting)
- [💻 For Developers (Compiling from Source)](#for-developers-compiling-from-source)

</details>

<details open>
<summary><b>中文目录</b></summary>

- [🎯 这是什么？](#%E8%BF%99%E6%98%AF%E4%BB%80%E4%B9%88-1)
- [✨ 核心功能](#%E6%A0%B8%E5%BF%83%E5%8A%9F%E8%83%BD)
- [🚀 快速上手（小白指南）](#%E5%BF%AB%E9%80%9F%E4%B8%8A%E6%89%8B%E5%B0%8F%E7%99%BD%E6%8C%87%E5%8D%97)
  - [准备工作](#准备工作)
  - [第一步：下载与安装](#第一步%E4%B8%8B%E8%BD%BD%E4%B8%8E%E5%AE%89%E8%A3%85)
  - [第二步：配置 AI 模型文件](#第二步%E9%85%8D%E7%BD%AE-ai-%E6%A8%A1%E5%9E%8B%E6%96%87%E4%BB%B6)
  - [第三步：配置 ONNX 运行环境](#第三步%E9%85%8D%E7%BD%AE-onnx-%E8%BF%90%E8%A1%8C%E7%8E%AF%E5%A2%83)
- [🎮 傻瓜式使用教程](#%E5%82%BB%E7%93%9C%E5%BC%8F%E4%BD%BF%E7%94%A8%E6%95%99%E7%A8%8B)
- [⚙️ 常用参数大白话翻译](#%E5%B8%B8%E7%94%A8%E5%8F%82%E6%95%B0%E5%A4%A7%E7%99%BD%E8%AF%9D%E7%BF%BB%E8%AF%91)
- [🔧 常见问题与自助排查](#%E5%B8%B8%E8%A7%85%E9%97%AE%E9%A2%98%E4%B8%8E%E8%87%AA%E5%8A%A9%E6%8E%92%E6%9F%A5)
- [💻 开发者专区（源码编译与调试）](#%E5%BC%80%E5%8F%91%E8%85%85%E4%B8%93%E5%8C%BA%E6%BA%90%E7%A0%81%E7%BC%96%E8%AF%91%E4%B8%8E%E8%B0%83%E8%AF%95)

</details>

---

<a id="english"></a>

## 🎯 What Is This?

**Pitch Analyzer** is a friendly desktop software that automatically extracts and visualizes the notes and pitch curve of your singing. Using state-of-the-art **FCPE** deep learning AI, it listens to your raw vocal recording, traces the frequency, snaps the pitch to musical notes, and helps you easily align lyrics to build karaoke subtitles.

If you are a **singer**, **content creator**, or **vocal tuner**, this is your go-to companion:
> Import your audio ➜ Watch notes appear on a beautiful piano roll ➜ Import lyrics ➜ Play & export synced CJK/English subtitles (SRT) with notes.

---

## ✨ Key Features

- 🔬 **Highly Accurate AI Pitch Tracking** — Powered by the FCPE model via ONNX Runtime. Perfect even on quiet/noisy acapella tracks.
- 🎹 **Visual Piano Roll** — A scrolling view showing your pitch curve aligned over piano keys with a real-time playback cursor.
- 🎤 **Karaoke Lyrics Sync** — Import LRC (with/without translations) or plain text TXT. The app automatically splits CJK/English characters and highlights them in sync.
- 📋 **One-Click Subtitle Export (SRT)** — Generates subtitle tracks with note annotations (e.g. `Hello [C4]`) for video editing.
- 💾 **Project Save/Load** — Save your workspace into a single JSON project file and reopen it anytime.

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🚀 Quick Start (For Users)

### Prerequisites

| System | Recommended Setup |
|--------|-------------------|
| **Windows** | Windows 10 or 11 |
| **Linux** | Ubuntu / Arch / Fedora (with GTK3 libraries) |

### Step 1: Download & Install
Download the installer or binary for your operating system from the **GitHub Releases** page:
* **Windows**: Run the `.exe` setup wizard.
* **Linux**: Download the `.AppImage` (make it executable via `chmod +x`) or install the `.deb` package.

### Step 2: Load the AI Model
Because AI models are too large to host on source code, you need to place them manually:
1. Create a folder named `models` in the same directory as the executable file.
2. Put these two files inside the `models` folder:
   - `fcpe.onnx` (the main AI model file, ~43 MB)
   - `fcpe_config.json` (the model configuration parameters, ~8 KB)

### Step 3: Setup ONNX Runtime
The app uses ONNX Runtime to accelerate calculations.
* **Windows**: Ready to run out-of-the-box.
* **Linux**: Install it via Python:
  ```bash
  pip install onnxruntime
  ```
  The app will automatically detect it. If you get a warning, set this environment variable:
  ```bash
  export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
  ```

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🎮 Step-by-Step Tutorial

```
1. Import Audio ➜ 2. Fine-tune Pitch ➜ 3. Load Lyrics ➜ 4. Preview Playback ➜ 5. Export SRT
```

1. **Import Audio**: Click the blue **"📂 Import Audio & Analyze"** button. The app will decode and resample your file, run the AI model, and draw the green pitch curve on the piano roll. An analysis progress bar at the bottom will keep you updated.
2. **Parameters & Presets**: If your singer's voice is high or low, or has noise, select a preset in the sidebar:
   - **Pop**: Good for standard pop/rock vocal tracks.
   - **Folk**: Good for solo acoustic or clean acapellas.
   - **Classical**: Snaps pitches to exact piano notes (Quantized).
3. **Add Lyrics**: Click **"🎵 Import LRC"** (recommended for synced lyrics) or **"📝 Import TXT"**. The words will align with the notes. Active syllables will glow green as they play!
4. **Playback**: Press the play (▶) button. You can slide the progress bar or change volume.
5. **Export Subtitles**: Click **"📋 Export SRT"** to save your annotated subtitles. Drag the SRT file into Premiere, CapCut, or DaVinci Resolve!

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## ⚙️ Parameter Tuning (Plain English)

- **Confidence Threshold**: The "cutoff" filter. Lower values (e.g., `0.15`) detect softer vocal details but might pick up background noise. Higher values (e.g., `0.40`) only keep very clean, loud singing.
- **Min/Max Frequency (Hz)**: Limit the range to match the singer's vocal range (e.g. Bass vs Soprano) to filter out high-frequency noise or low-frequency rumbling.
- **Peak Filtering (Median)**: Removes sudden pitch spikes caused by glitches or breaths.
- **Curve Smoothing**: Smooths out natural voice vibrato to make the pitch track cleaner.
- **Quantize to Semitone**: Snaps your pitch line directly to the nearest piano keys, turning fluid slides into distinct musical notes.

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🔧 Easy Troubleshooting

### ❌ "找不到 models/fcpe.onnx" (Model Not Found)
Make sure the `models/` folder sits exactly next to the application executable, containing both `fcpe.onnx` and `fcpe_config.json`.

### ❌ "未找到 libonnxruntime" (ONNX Runtime Not Found - Linux)
Run `pip install onnxruntime` in your system. If you are using a virtual environment, start the app from that terminal environment.

### ❌ The Play button is disabled
You must click "Import Audio & Analyze" and wait for the analysis progress bar to finish (reaching 100%) before playback is unlocked.

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 💻 For Developers (Compiling from Source)

If you wish to compile or modify the application, follow these developer commands:

```bash
# 1. Install frontend dependencies
pnpm install

# 2. Run in development mode
pnpm tauri dev

# 3. Run Rust backend tests
cargo test --manifest-path src-tauri/Cargo.lock

# 4. Build release package
pnpm tauri build
```

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

<a id="中文"></a>

## 🎯 这是什么？

**人声音高测量工具** 是一款专为歌手、音视频创作者和修音师设计的桌面应用。它可以通过 AI 自动提取并可视化您录音中人声的音高曲线。

基于先进的 **FCPE** 深度学习模型，它能智能识别清唱人声，提取频率并自动对齐 MIDI 音符，最终协助您完成精细化的卡拉OK歌词标注与字幕制作。

> 导入音频 ➜ 钢琴卷帘窗实时显示音轨 ➜ 导入歌词 ➜ 随心播放并一键导出带音高标注的双语字幕（SRT）。

---

## ✨ 核心功能

- 🔬 **极准的 AI 音高跟踪** — 基于 FCPE 模型与 ONNX Runtime 硬件加速，干净人声和略带底噪的清唱皆可精准感应。
- 🎹 **直观的钢琴卷帘窗** — 绿色的音高曲线在钢琴键背景上一目了然，带播放光标指示。
- 🎤 **卡拉OK 歌词自动对齐** — 导入 LRC（支持双语合并）或 TXT 文本，程序自动分词并对齐，播放时唱到的字会变成**青色并微微放大发光**。
- 📋 **一键导出 SRT 字幕** — 导出带音名标注（如 `你好 [C4]`）的字幕文件，方便导入各大视频剪辑软件。
- 💾 **项目工程存取** — 支持将所有分析参数和对齐歌词保存为 JSON 工程文件，下次直接打开。

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🚀 快速上手（小白指南）

### 准备工作

| 操作系统 | 推荐环境 |
|--------|-------------------|
| **Windows** | Windows 10 或 11 |
| **Linux** | Ubuntu / Arch / Fedora (需安装 GTK3 依赖) |

### 第一步：下载与安装
在项目的 **GitHub Releases** 页面下载安装包：
* **Windows**：下载 `.exe` 安装包并点击下一步完成安装。
* **Linux**：下载 `.AppImage`（右键添加可执行权限即可运行）或 `.deb` 安装包。

### 第二步：配置 AI 模型文件
由于 AI 模型文件太大，我们没有将其打包进源码库。请在**可执行程序同级目录**下：
1. 新建一个名为 `models` 的文件夹。
2. 将以下两个模型文件放入 `models` 中：
   - `fcpe.onnx`（音高检测模型，约 43 MB）
   - `fcpe_config.json`（模型配置文件，约 8 KB）

### 第三步：配置 ONNX 运行环境
应用依赖 ONNX Runtime 来运行模型。
* **Windows**：无需额外配置，开箱即用。
* **Linux**：建议在系统中通过 pip 安装运行库：
  ```bash
  pip install onnxruntime
  ```
  应用会自动尝试在系统中搜寻该运行库。如果运行提示未找到，可以手动设置环境变量：
  ```bash
  export ORT_DYLIB_PATH=/你的路径/libonnxruntime.so
  ```

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🎮 傻瓜式使用教程

```
导入人声音频 ➜ 侧边栏微调音高 ➜ 导入歌词对照 ➜ 随心试听 ➜ 导出带音高字幕
```

1. **导入音频**：点击左侧蓝色的 **"📂 导入音频并分析"** 按钮，选择人声录音。侧边栏底部会出现进度条，显示解码、推理等进度。分析完成后，钢琴卷帘窗上会出现绿色的音高曲线。
2. **选择适合的预设**：侧边栏上方有快捷预设按钮：
   - **流行**：适配绝大多数流行与摇滚人声。
   - **民谣**：适配比较干净的民谣清唱、弹唱人声。
   - **古典**：适配美声或合唱，音高线会自动吸附到钢琴键上。
3. **导入歌词**：点击 **"🎵 导入 LRC"**（带时间戳的歌词文件）或 **"📝 导入 TXT"**（纯文本）。歌词会显示在顶部面板，播放时唱到的字会变成**青色并微微放大发光**。
4. **播放试听**：点击下方的播放按钮 (▶) 即可试听。拖拽滚动条可以调节音轨进度，拖动音量条调节音量大小。
5. **导出成果**：点击 **"📋 导出 SRT"**，可直接将带音名字幕保存到本地。导入剪辑软件（如剪映、PR、FCP）即可制作出带有音符提示的高档卡拉OK字幕！

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## ⚙️ 常用参数大白话翻译

- **置信度阈值**：音高感应的灵敏度。越低越灵敏（如 `0.15` 会检测到细微的换气或微弱的尾音，但容易引入杂音）；越高越保守（如 `0.40` 只保留唱得非常清晰响亮的部分）。
- **最低/最高频率 (Hz)**：限制歌手的声部范围（例如男低音 vs 女高音），有效过滤高频杂音或低频风声。
- **峰值过滤 (中值)**：自动剔除音高中由于喷麦、呼吸引起的突变尖峰。
- **曲线平滑**：消减歌手嗓音中细微的多余抖动和颤音，让线条更平滑。
- **对齐到半音**：开启后，滑动音高会变成类似于琴键的一格格阶梯，自动归类到最邻近的音符上。

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 🔧 常见问题与自助排查

### ❌ 提示 "找不到 models/fcpe.onnx"
请确保 `models` 文件夹和您运行的程序在同一目录层级下，并且里面包含有 `fcpe.onnx` 以及 `fcpe_config.json` 两个文件。

### ❌ 提示 "未找到 libonnxruntime" (Linux)
请确认您是否运行过 `pip install onnxruntime`。如果是，请确保在运行应用时能访问到 Python 包环境。

### ❌ 为什么播放按钮点不了？
您必须先成功“导入音频并分析”，等底部的分析进度条走到 100% 后，播放控制才会解锁。

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>

---

## 💻 开发者专区（源码编译与调试）

如果您需要调试源码或自主编译构建，请使用以下命令：

```bash
# 1. 安装前端所需依赖
pnpm install

# 2. 启动前端和 Tauri 的开发模式
pnpm tauri dev

# 3. 运行 Rust 后端自动化测试
cargo test --manifest-path src-tauri/Cargo.lock

# 4. 打包发布应用
pnpm tauri build
```

<p align="right">(<a href="#top">Back to Top / 返回顶部</a>)</p>
