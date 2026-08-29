# 模型文件

**注意**: 此目录下的模型文件（`*.onnx`, `*.json`）**不会被提交到 Git**，因为它们太大了。

## 需要的文件

请在此目录下放置以下文件：

1. `fcpe.onnx` - FCPE 模型文件（约 69MB，输入 mel (1,T,128) → 输出 latent (1,T,360)）
2. `fcpe_config.json` - 模型配置（cent_table，360 项）

## 获取方式

### 方式 1: 从 .pt 权重导出（推荐，本仓库自带脚本）

支持 CNChTu/FCPE 发布的 `.pt` 权重（TransformerF0BCE 格式，键为 `model`/`config`），
也兼容 torchfcpe 打包格式（键为 `model`/`config_dict`）：

```bash
# 需要 torch + torchfcpe + onnx + onnxruntime 的 Python 环境
D:/Agent/pitch/.venv/Scripts/python.exe scripts/export_fcpe_onnx.py \
    --pt models/fcpe.pt --out-dir models
```

脚本会自动做 torch↔onnx 数值校验（偏差应 < 1e-3）。

### 方式 2: 下载预导出模型

从项目发布页下载或从其他来源获取。

## ONNX Runtime

- Windows: `src-tauri/resources/onnxruntime.dll`（ort 2.0.0-rc.10 需要 **1.22.x**，
  可从 `pip install onnxruntime==1.22.0` 的 `onnxruntime/capi/` 中复制）。
- Linux: 设置 `ORT_DYLIB_PATH` 指向 `libonnxruntime.so` 1.22.x。

## 验证

```bash
# 一致性测试 (mel/latent 与 Python 参考对比, 需要 models/test_data.json)
cargo test --test integration

# 真实歌曲验收测试 (需要 models/fcpe.onnx + 根目录下的人声歌曲)
cargo test --release --test real_song_test -- --ignored --nocapture
```

文件放好后，目录结构应该是：

```
models/
├── README.md       (这个文件)
├── fcpe.onnx       (需要添加)
├── fcpe_config.json (需要添加)
└── test_data.json  (可选, 集成测试参考数据, scripts/gen_test_data.py 生成)
```
