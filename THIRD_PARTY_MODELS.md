# Third-Party Models / Resources — License Audit Ledger

Round4 任务书 §30 要求: 引入任何 G2P / FA 资源前必须记录以下信息。
**代码仓库的 license (Apache/MIT) 不代表模型权重可以随应用分发。**

## 已随应用分发

| 资源 | 用途 | License | 再分发 | 备注 |
| --- | --- | --- | --- | --- |
| UniDic (via lindera `embed-unidic`) | 日语形态分析/读音 | UniDic 使用条款 (BSD-3 类, 需保留出处) | 允许 (保留声明) | 编译期嵌入, 见 lindera 文档 |
| FCPE ONNX 模型 (用户自备/仓库 models/) | 连续 F0 | FCPE 仓库 (MIT, 待核对权重授权) | 待核对 — 默认由用户自行获取 | 启动时校验文件存在 |
| GAME-1.0.3-small ONNX (用户自备) | 离散音符转写 | OpenVPI GAME (MIT 代码) | 代码 MIT; 模型包随分发渠道而定 | config.json 的 languages 动态读取 |

## 计划接入 / 实验骨架 (未分发)

| 资源 | 用途 | 状态 | 待办 |
| --- | --- | --- | --- |
| torchaudio MMS-FA (`scripts/legacy_mms_worker.py`) | Legacy Experimental FA | 运行时由用户 Python 环境下载权重 | torch/torchaudio 需锁版本 (2.8 起 FA API deprecated, 2.9 移除); requirements-fa-legacy.txt 待补 |
| OpenVPI HubertFA / dataset-tools 体系 | Production singing FA | `singing_fa.rs` 骨架 (诚实返回 DependencyMissing) | 核对 dataset-tools 代码 license 与模型/词典 license 是否一致; 评估再分发、模型体积、CPU 推理速度、Windows/Linux 打包 |
| 中文 LyricFA | 中文 baseline FA | 未接入 | 同上 |

## 规则

1. 任何新模型/词典接入前, 在本表登记 license、再分发条款、商用条款、署名要求。
2. license 不允许再分发 → 实现 Model Manager 让用户自行下载 (不打包进安装产物)。
3. 不允许静默加载错误版本 config + onnx 混搭 (Model Registry 属于后续工作)。
