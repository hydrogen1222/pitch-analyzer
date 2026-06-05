# 模型文件

**注意**: 此目录下的模型文件（`*.onnx`, `*.json`）**不会被提交到 Git**，因为它们太大了。

## 需要的文件

请在此目录下放置以下文件：

1. `fcpe.onnx` - FCPE 模型文件（约 43MB）
2. `fcpe_config.json` - 模型配置文件

## 获取方式

### 方式 1: 从 Python 导出

如果你有原始的 `torchfcpe` 包：

```bash
cd /path/to/original/pitch/project
uv run export_fcpe_onnx.py --output-dir /path/to/this/project/models
```

### 方式 2: 下载预导出模型

从项目发布页下载或从其他来源获取。

## 验证

文件放好后，目录结构应该是：

```
models/
├── README.md       (这个文件)
├── fcpe.onnx       (需要添加)
└── fcpe_config.json (需要添加)
```
