# 运行指南

## ❌ 错误方式

**不要**直接在浏览器中打开 `index.html` 或用 `pnpm dev` 只启动 Vite！
- 前端依赖 Tauri 的 APIs（文件对话框、调用 Rust 后端等）
- 直接在浏览器运行会报错“无法导入文件”

## ✅ 正确方式

### 前置条件

1. **Node.js 20+** 和 **pnpm**
2. **Rust 1.75+** (安装方式: https://rustup.rs/)
3. **系统依赖** (根据你的 OS):
   - Ubuntu/Debian:
     ```bash
     sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
     ```
   - Arch Linux:
     ```bash
     sudo pacman -S webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg
     ```

### 设置 ONNX Runtime

应用需要 ONNX Runtime 库。设置方式：

**方式 1: 设置环境变量**
```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
```

**方式 2: 放置在常见位置**
- `/usr/lib/libonnxruntime.so`
- `/usr/local/lib/libonnxruntime.so`

### 放置模型文件

在项目根目录下的 `models/` 文件夹中放置:
- `fcpe.onnx`
- `fcpe_config.json`

### 启动应用

```bash
# 安装依赖
pnpm install

# 启动开发模式（这会同时启动 Vite 和 Tauri）
pnpm tauri dev
```

### 构建发布版本

```bash
pnpm tauri build
```

输出位置: `src-tauri/target/release/bundle/`
