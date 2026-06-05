<div align="center">

# 🎵 Pitch Analyzer / 人声音高测量工具

**基于深度学习的人声音高分析与可视化工具**

[FCPE](https://github.com/CNChTu/FCPE) 音高引擎 · Tauri 2 + Rust · 跨平台桌面应用

[English](#english) · [中文](#中文)

</div>

---

<a id="english"></a>

## 🎯 What Is This?

Pitch Analyzer is a desktop app that detects the pitch (fundamental frequency) of vocal tracks in audio files. It uses the **FCPE** deep learning model to achieve high-accuracy pitch estimation, even on clean vocals without instrumental accompaniment.

Think of it as: **import a song → see every note the singer sings, quantized to MIDI, with a piano-roll visualization and karaoke-style lyric display.**

### Key Features

| Feature | Description |
|---------|-------------|
| 🔬 **FCPE Pitch Detection** | State-of-the-art vocal pitch estimation via ONNX Runtime (CPU) |
| 🎹 **Piano Roll Visualization** | Real-time pitch curve on a piano-key background with play cursor |
| 🎤 **Karaoke Lyrics** | LRC (bilingual) / TXT lyric import with per-character note labels |
| 📋 **SRT Export** | Subtitle files with note names per lyric token (e.g. `Hello [C4]`) |
| 💾 **Project Save/Load** | JSON project files for later re-opening |
| ▶️ **Built-in Playback** | Play/pause/seek with synchronized visual cursor |
| ⚙️ **Presets & Tuning** | Pop / Folk / Classical presets + fine-grained parameter control |

---

## 📸 Screenshots

> *(Insert screenshots here after first release)*

---

## 🚀 Quick Start

### Prerequisites

| Dependency | Version | Why |
|-----------|---------|-----|
| [Node.js](https://nodejs.org/) | 20+ | Frontend build tooling |
| [pnpm](https://pnpm.io/) | 9+ | Package manager |
| [Rust](https://rustup.rs/) | 1.75+ | Backend language |
| System libs (Linux) | — | GTK3, WebKit2GTK, etc. (see below) |
| ONNX Runtime | 1.17+ | ML inference engine |

### Step 1: Install System Dependencies

**Linux (Debian/Ubuntu):**
```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
```

**Linux (Arch/Manjaro):**
```bash
sudo pacman -S gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg
```

**Windows:**
No extra system libs needed. Install [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) if you don't have them.

### Step 2: Get the FCPE Model

The app needs an FCPE model in ONNX format. You have two options:

**Option A — Export from the Python prototype** (if you have the original `torchfcpe` package):
```bash
cd /path/to/original/pitch/project
uv run export_fcpe_onnx.py --output-dir /path/to/pitch-analyzer-tauri/models
```

**Option B — Use pre-exported model files:**
Place these two files in the `models/` folder at the project root:
- `fcpe.onnx` — the ONNX model file (~43 MB)
- `fcpe_config.json` — model configuration with cent table (~8 KB)

> 📌 The `models/` folder should sit next to `package.json`.

### Step 3: Install ONNX Runtime

The app needs the ONNX Runtime shared library at runtime.

**Option A — Via pip** (easiest on Linux, reuses an existing venv):
```bash
pip install onnxruntime
# The .so file is at: ~/.local/lib/python3.X/site-packages/onnxruntime/capi/libonnxruntime.so.X.Y.Z
```

**Option B — Download manually:**
1. Go to [onnxruntime releases](https://github.com/microsoft/onnxruntime/releases)
2. Download `onnxruntime-linux-x64-1.20.0.tgz` (or latest)
3. Extract and note the path to `lib/libonnxruntime.so`

The app will try to auto-detect the library from common locations. If it can't find it, set the environment variable:
```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
```

### Step 4: Install & Run

```bash
# Clone and enter the project
cd pitch-analyzer-tauri

# Install frontend dependencies
pnpm install

# Start the development app
pnpm tauri dev
```

The app window should open. Click **"📂 Import Audio & Analyze"** to load a WAV/FLAC/MP3/OGG file.

---

## 🎮 How to Use

### Basic Workflow

```
Import Audio → Analyze → (Optional: Import Lyrics) → Play / Export
```

1. **Import Audio**: Click the blue button or drag a file. The app will:
   - Decode the audio to 16 kHz mono
   - Compute mel spectrogram
   - Run FCPE inference
   - Apply DSP post-processing (Hampel → Median → Savitzky-Golay)
   - Display the pitch curve on the piano roll

2. **Adjust Parameters**: Use presets (Pop/Folk/Classical) or fine-tune:
   - **Confidence Threshold** — lower = more notes detected, but more false positives
   - **Fmin / Fmax** — frequency range to keep
   - **Median Filter** — removes pitch spikes
   - **Savitzky-Golay** — smooths the pitch curve
   - **Quantize** — snap pitch to nearest semitone

3. **Import Lyrics**: Load an LRC file (with timestamps) or plain TXT.
   - LRC supports bilingual lyrics (same timestamp = merged as translation)
   - Lyrics are automatically time-aligned and pitch-bound

4. **Playback**: Use ▶/⏸ and the progress slider. The red cursor follows the pitch.

5. **Export**:
   - **SRT** — subtitle file with note names per lyric character
   - **Project JSON** — save everything for later re-opening

### Parameter Presets

| Preset | Best For | Confidence | Smoothing | Quantize |
|--------|----------|------------|-----------|----------|
| **Pop** | Pop, rock, J-pop vocals | 0.30 | 15 / 11 | Off |
| **Folk** | Acoustic, a cappella | 0.25 | 17 / 13 | Off |
| **Classical** | Opera, choral | 0.20 | 21 / 15 | On |

---

## 🏗️ Architecture

```
┌──────────────────────────────────────────────┐
│  Web Frontend (TypeScript + Canvas)          │
│  ├─ Piano Roll + Pitch Curve (Canvas 2D)    │
│  ├─ Karaoke Lyrics Display (DOM)            │
│  ├─ Playback Controls                        │
│  └─ Parameter Panel + File Operations        │
├──────────────────────────────────────────────┤
│  Tauri 2 IPC (invoke commands)               │
├──────────────────────────────────────────────┤
│  Rust Backend                                │
│  ├─ Audio Decode (symphonia → 16k mono)     │
│  ├─ Mel Spectrogram (librosa-compatible)     │
│  ├─ FCPE ONNX Inference (ort crate)          │
│  ├─ Local Argmax Decoder (cent_table → f0)   │
│  ├─ DSP Post-processing (Hampel/Median/Savg) │
│  ├─ Lyrics Parser + Aligner (LRC/TXT)        │
│  ├─ Audio Playback (rodio, dedicated thread)  │
│  └─ SRT Export + Project Serialization        │
└──────────────────────────────────────────────┘
```

### File Map

| File | Purpose |
|------|---------|
| `src/main.ts` | App entry point, all Tauri command calls |
| `src/pitch_canvas.ts` | Piano roll + pitch curve renderer |
| `src/karaoke_display.ts` | Karaoke lyric display |
| `src/types.ts` | Analysis params, presets |
| `src-tauri/src/lib.rs` | Tauri commands (16 total) + app state |
| `src-tauri/src/analyzer.rs` | FCPE pipeline orchestrator |
| `src-tauri/src/audio.rs` | Audio decode + resample |
| `src-tauri/src/mel.rs` | Mel spectrogram (STFT + mel filterbank) |
| `src-tauri/src/decoder.rs` | FCPE local_argmax decoder |
| `src-tauri/src/dsp.rs` | Pitch post-processing filters |
| `src-tauri/src/playback.rs` | Audio player (rodio, thread-isolated) |
| `src-tauri/src/lyrics.rs` | LRC/TXT parser + aligner + SRT export |
| `src-tauri/src/models.rs` | Data structures |

---

## 🧪 Testing

```bash
cd src-tauri

# Core pipeline tests (mel accuracy, ONNX inference, e2e with real audio)
ORT_DYLIB_PATH=/path/to/libonnxruntime.so cargo test --test integration -- --nocapture

# Lyrics parsing tests
cargo test --test lyrics_test -- --nocapture

# SRT export tests
cargo test --test srt_test -- --nocapture

# Run all tests
ORT_DYLIB_PATH=/path/to/libonnxruntime.so cargo test -- --nocapture
```

### Test Coverage

| Test | What It Checks |
|------|---------------|
| `test_mel_matches_python` | Rust mel spectrogram vs Python reference (max diff < 2e-5) |
| `test_onnx_inference_matches_python` | ONNX output vs PyTorch (max diff < 2e-7) |
| `test_decoder_produces_f0` | FCPE local_argmax produces valid f0 values |
| `test_end_to_end_real_audio` | Full pipeline on a 5-min FLAC |
| `test_tokenize_mixed` | CJK + English tokenization |
| `test_parse_lrc_*` | LRC parsing, bilingual merge |
| `test_export_srt_*` | SRT output with/without lyrics |

---

## 📦 Building for Distribution

```bash
pnpm tauri build
```

Output will be in `src-tauri/target/release/bundle/`.

> ⚠️ For distribution, you need to either:
> 1. Bundle `libonnxruntime.so` alongside the binary, or
> 2. Instruct users to install ONNX Runtime separately
>
> The model files (`fcpe.onnx`, `fcpe_config.json`) must also be in the `models/` directory relative to the executable.

---

## 🔧 Troubleshooting

### "找不到 models/fcpe.onnx"

The model files are not included in the repo (too large). See [Step 2](#step-2--get-the-fcpe-model) above.

### "ORT_DYLIB_PATH not set" / "未找到 libonnxruntime"

The app can't find the ONNX Runtime shared library. Set it explicitly:
```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.20.0
```

### Port 1420 already in use

Another Tauri dev instance is running. Kill it:
```bash
lsof -ti :1420 | xargs kill -9
```

### Audio decode is slow for large files

The resampler (rubato) processes the entire file at once. For files > 10 minutes, decoding may take 30-60 seconds. This will be improved in a future release with chunked processing.

### Build fails on Linux: "cannot find -lgtk-3"

Install GTK3 development headers:
```bash
# Debian/Ubuntu
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev

# Arch
sudo pacman -S gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg
```

---

## 📊 Performance

Measured on a 5-minute FLAC (vocals, 16 kHz, ~31400 frames):

| Stage | Time |
|-------|------|
| Audio decode + resample | ~20s |
| Mel spectrogram | ~8s |
| FCPE ONNX inference | **~2.4s** |
| DSP post-processing | ~0.4s |
| **Total** | **~31s** |

> ONNX inference is **faster than PyTorch eager mode** on the same CPU.

---

## 📜 License

This project is for personal and educational use. The FCPE model is from [torchfcpe](https://github.com/CNChTu/FCPE) — please check its license for commercial use.

---

<a id="中文"></a>

---

<div align="center">

# 🎵 人声音高测量工具

**基于深度学习的人声音高分析与可视化桌面应用**

</div>

## 🎯 这是什么？

这是一款桌面应用，可以检测音频文件中**人声的音高（基频）**。它使用 **FCPE** 深度学习模型实现高精度音高估计，即使在没有伴奏的清唱人声上也能准确工作。

简单来说：**导入一首歌 → 看到歌手唱的每一个音符，量化为 MIDI 编号，有钢琴卷帘可视化和卡拉OK 歌词显示。**

### 核心功能

| 功能 | 说明 |
|------|------|
| 🔬 **FCPE 音高检测** | 基于 ONNX Runtime 的 SOTA 人声音高估计（CPU 推理） |
| 🎹 **钢琴卷帘可视化** | 实时音高曲线 + 钢琴键背景 + 播放光标 |
| 🎤 **卡拉OK 歌词** | 支持 LRC（含双语合并）/ TXT 歌词导入，每字标注音符 |
| 📋 **SRT 字幕导出** | 每个歌词字对应音符名（如 `你 [C4]`） |
| 💾 **项目保存/加载** | JSON 格式，随时保存和重新打开分析结果 |
| ▶️ **内置播放器** | 播放/暂停/跳转，光标同步跟踪 |
| ⚙️ **预设与微调** | 流行/民谣/古典预设 + 细粒度参数调节 |

---

## 🚀 快速开始

### 你需要先安装的东西

| 依赖 | 版本 | 用途 |
|------|------|------|
| [Node.js](https://nodejs.org/) | 20+ | 前端构建 |
| [pnpm](https://pnpm.io/) | 9+ | 包管理器 |
| [Rust](https://rustup.rs/) | 1.75+ | 后端语言 |
| 系统库 (Linux) | — | GTK3 等（见下方） |
| ONNX Runtime | 1.17+ | AI 推理引擎 |

> 💡 **小白提示**：Rust 安装只需一行命令：`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

### 第一步：安装系统依赖

**Linux (Debian/Ubuntu)：**
```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
```

**Linux (Arch/Manjaro)：**
```bash
sudo pacman -S gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg
```

**Windows：**
不需要额外系统库。确保已安装 [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)。

### 第二步：获取 FCPE 模型

应用需要 FCPE 模型的 ONNX 格式文件。两种方式：

**方式 A — 从 Python 原型导出**（如果你有原来的 `torchfcpe` 包）：
```bash
cd /path/to/original/pitch/project
uv run export_fcpe_onnx.py --output-dir /path/to/pitch-analyzer-tauri/models
```

**方式 B — 直接放置模型文件：**
将以下两个文件放到项目根目录的 `models/` 文件夹中：
- `fcpe.onnx` — ONNX 模型文件（约 43 MB）
- `fcpe_config.json` — 模型配置（含 cent 表，约 8 KB）

> 📌 `models/` 文件夹应该和 `package.json` 在同一层级。

### 第三步：安装 ONNX Runtime

应用运行时需要 ONNX Runtime 动态库。

**方式 A — 通过 pip 安装**（Linux 最简单，复用现有 venv）：
```bash
pip install onnxruntime
# .so 文件位置：~/.local/lib/python3.X/site-packages/onnxruntime/capi/libonnxruntime.so.X.Y.Z
```

**方式 B — 手动下载：**
1. 前往 [onnxruntime releases](https://github.com/microsoft/onnxruntime/releases)
2. 下载 `onnxruntime-linux-x64-1.20.0.tgz`（或最新版）
3. 解压并记下 `lib/libonnxruntime.so` 的路径

应用会尝试从常见路径自动检测。如果找不到，设置环境变量：
```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
```

### 第四步：安装依赖并运行

```bash
# 进入项目目录
cd pitch-analyzer-tauri

# 安装前端依赖
pnpm install

# 启动开发版应用
pnpm tauri dev
```

应用窗口会弹出来。点击蓝色的 **「📂 导入音频并分析」** 按钮加载 WAV/FLAC/MP3/OGG 文件即可。

---

## 🎮 使用说明

### 基本流程

```
导入音频 → 分析 → （可选：导入歌词）→ 播放 / 导出
```

1. **导入音频**：点击蓝色按钮或拖入文件。应用会自动：
   - 解码音频为 16 kHz 单声道
   - 计算 Mel 频谱
   - 运行 FCPE 推理
   - 应用后处理（Hampel 滤波 → 中值滤波 → Savitzky-Golay 平滑）
   - 在钢琴卷帘上显示音高曲线

2. **调节参数**：使用预设（流行/民谣/古典）或手动调节：
   - **置信阈值** — 越低检测到的音符越多，但误检也会增加
   - **最低/最高频率** — 保留的频率范围
   - **峰值过滤** — 去除音高突变点
   - **曲线平滑** — 平滑音高曲线
   - **对齐半音** — 将音高吸附到最近的半音

3. **导入歌词**：加载 LRC（带时间戳）或纯 TXT 文件。
   - LRC 支持双语歌词（相同时间戳的行会自动合并为翻译）
   - 歌词会自动分配每字时间并绑定音高

4. **播放**：使用 ▶/⏸ 按钮和进度条。红色光标会跟随当前播放位置。

5. **导出**：
   - **SRT 字幕** — 每个歌词字对应一个音符名
   - **项目 JSON** — 保存所有分析结果，下次可以直接打开

### 参数预设说明

| 预设 | 适用场景 | 置信阈值 | 平滑 | 量化 |
|------|---------|---------|------|------|
| **流行** | 流行、摇滚、J-POP 人声 | 0.30 | 15 / 11 | 关 |
| **民谣** | 民谣、清唱、吉他弹唱 | 0.25 | 17 / 13 | 关 |
| **古典** | 美声、合唱、古典声乐 | 0.20 | 21 / 15 | 开 |

---

## 🏗️ 技术架构

```
┌──────────────────────────────────────────────┐
│  Web 前端 (TypeScript + Canvas)              │
│  ├─ 钢琴卷帘 + 音高曲线 (Canvas 2D)         │
│  ├─ 卡拉OK 歌词显示 (DOM)                    │
│  ├─ 播放控制条                                │
│  └─ 参数面板 + 文件操作                        │
├──────────────────────────────────────────────┤
│  Tauri 2 IPC (invoke 命令调用)                │
├──────────────────────────────────────────────┤
│  Rust 后端                                    │
│  ├─ 音频解码 (symphonia → 16kHz 单声道)      │
│  ├─ Mel 频谱计算 (librosa 兼容)               │
│  ├─ FCPE ONNX 推理 (ort crate)                │
│  ├─ 局部 Argmax 解码器 (cent_table → f0)      │
│  ├─ DSP 后处理 (Hampel/中值/Savgol 滤波)      │
│  ├─ 歌词解析器 + 对齐器 (LRC/TXT)             │
│  ├─ 音频播放 (rodio, 独立线程)                │
│  └─ SRT 导出 + 项目序列化                      │
└──────────────────────────────────────────────┘
```

### 文件说明

| 文件 | 用途 |
|------|------|
| `src/main.ts` | 应用入口，所有 Tauri 命令调用 |
| `src/pitch_canvas.ts` | 钢琴卷帘 + 音高曲线绘制 |
| `src/karaoke_display.ts` | 卡拉OK 歌词显示 |
| `src/types.ts` | 分析参数、预设定义 |
| `src-tauri/src/lib.rs` | Tauri 命令（共 16 个）+ 应用状态 |
| `src-tauri/src/analyzer.rs` | FCPE 分析流水线 |
| `src-tauri/src/audio.rs` | 音频解码 + 重采样 |
| `src-tauri/src/mel.rs` | Mel 频谱（STFT + Mel 滤波器组） |
| `src-tauri/src/decoder.rs` | FCPE local_argmax 解码器 |
| `src-tauri/src/dsp.rs` | 音高后处理滤波器 |
| `src-tauri/src/playback.rs` | 音频播放器（rodio，线程隔离） |
| `src-tauri/src/lyrics.rs` | LRC/TXT 解析 + 对齐 + SRT 导出 |
| `src-tauri/src/models.rs` | 数据结构定义 |

---

## 🧪 运行测试

```bash
cd src-tauri

# 核心流水线测试（Mel 精度、ONNX 推理、真实音频端到端）
ORT_DYLIB_PATH=/path/to/libonnxruntime.so cargo test --test integration -- --nocapture

# 歌词解析测试
cargo test --test lyrics_test -- --nocapture

# SRT 导出测试
cargo test --test srt_test -- --nocapture

# 运行所有测试
ORT_DYLIB_PATH=/path/to/libonnxruntime.so cargo test -- --nocapture
```

### 测试覆盖

| 测试 | 验证内容 |
|------|---------|
| `test_mel_matches_python` | Rust Mel 频谱与 Python 参考对比（最大误差 < 2e-5） |
| `test_onnx_inference_matches_python` | ONNX 推理与 PyTorch 对比（最大误差 < 2e-7） |
| `test_decoder_produces_f0` | FCPE 解码器输出合法 f0 值 |
| `test_end_to_end_real_audio` | 5 分钟真实 FLAC 全流水线测试 |
| `test_tokenize_mixed` | 中日英混合分词 |
| `test_parse_lrc_*` | LRC 解析、双语合并 |
| `test_export_srt_*` | 有/无歌词的 SRT 导出 |

---

## 📦 打包发布

```bash
pnpm tauri build
```

产物在 `src-tauri/target/release/bundle/` 中。

> ⚠️ 发布版需要：
> 1. 将 `libonnxruntime.so` 与可执行文件一起打包，或
> 2. 提示用户自行安装 ONNX Runtime
>
> 模型文件（`fcpe.onnx`、`fcpe_config.json`）也必须放在可执行文件同级的 `models/` 目录中。

---

## 🔧 常见问题

### ❌ "找不到 models/fcpe.onnx"

模型文件不在仓库中（文件太大）。请按上方 [第二步](#第二步获取-fcpe-模型) 操作。

### ❌ "ORT_DYLIB_PATH not set" / "未找到 libonnxruntime"

应用找不到 ONNX Runtime 动态库。请手动设置：
```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so.1.20.0
```

### ❌ 端口 1420 被占用

另一个 Tauri 开发实例正在运行。杀掉它：
```bash
lsof -ti :1420 | xargs kill -9
```

### ❌ 大文件解码很慢

重采样器（rubato）目前是全文件一次性处理。超过 10 分钟的文件可能需要 30-60 秒解码。后续版本会改为分块处理。

### ❌ Linux 编译失败: "cannot find -lgtk-3"

需要安装 GTK3 开发头文件：
```bash
# Debian/Ubuntu
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev

# Arch
sudo pacman -S gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg
```

---

## 📊 性能数据

使用 5 分钟 FLAC 人声（16 kHz，约 31400 帧）测量：

| 阶段 | 耗时 |
|------|------|
| 音频解码 + 重采样 | ~20s |
| Mel 频谱计算 | ~8s |
| FCPE ONNX 推理 | **~2.4s** |
| DSP 后处理 | ~0.4s |
| **总计** | **~31s** |

> ONNX 推理比同一 CPU 上的 PyTorch eager 模式**更快**。

---

## 📜 许可证

本项目仅供个人和学习使用。FCPE 模型来自 [torchfcpe](https://github.com/CNChTu/FCPE)——商业使用请查看其许可证。

---

## 🙏 致谢

- [FCPE](https://github.com/CNChTu/FCPE) — 音高检测模型
- [Tauri](https://tauri.app/) — 跨平台桌面应用框架
- [ONNX Runtime](https://onnxruntime.ai/) — 高性能推理引擎
- [rodio](https://github.com/RustAudio/rodio) — Rust 音频播放
- [symphonia](https://github.com/pdeljanov/Symphonia) — Rust 音频解码
