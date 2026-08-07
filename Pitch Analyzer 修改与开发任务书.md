# Pitch Analyzer 修改与开发任务书

项目仓库：`https://github.com/hydrogen1222/pitch-analyzer`

## 一、开发目标

本轮修改不要扩展为大型音频工作站，也不要加入与核心目标无关的功能。

最终目标只有一个：

> 导入已经准备好的人声音频 + LRC 歌词后，程序尽可能自动、稳定地完成 F0 检测、异常音高清理、歌词逐字时间分配、每字主音高判断，并导出适合制作“音高测量视频”的字幕文件。

主要解决当前以下问题：

1. 某些歌词，尤其每句第一个字，音高会在几十毫秒内突然跳到非常离谱的高音或低音，然后迅速恢复正常。
2. 当前逐帧 F0 结果过于敏感，单帧或极短错误会直接表现成音高跳变。
3. 当前歌词与音高绑定过度依赖逐帧结果，缺少“稳定音符事件”层。
4. 普通 LRC 只有逐句时间，没有真正逐字时间，目前简单平均分配会导致字与音高错位。
5. 一个字包含多个检测音高时，不能简单使用第一个音作为字幕显示音高。
6. 前后端歌词、音高参数、项目状态可能存在不同步问题。
7. 最终缺少适合直接压制视频的 ASS 字幕输出。

---

# 二、修改项目总表

| 优先级 | 模块                    | 当前问题                                                                               | 修改要求                        | 建议实现                                                            | 验收标准                                                                   |
| --- | --------------------- | ---------------------------------------------------------------------------------- | --------------------------- | --------------------------------------------------------------- | ---------------------------------------------------------------------- |
| P0  | 句首/句尾异常音高清理           | `remove_short_pitch_islands()` 对 voiced segment 的首尾 run 存在跳过逻辑，导致句首几十毫秒的错误八度可能无法清除 | 修改首尾短音符处理逻辑                 | 不要简单删除首尾保护；增加“短暂异常 + 后续稳定主音”的判断。首段若持续很短、与后续主音差距大且置信度低，应修正为后续稳定音 | 构造 `E5 30ms → E4 500ms`，最终字幕不能把 E5 识别为有效主音；真实 `D5 100ms → E5` 倚音则应尽量保留 |
| P0  | 瞬时音高跳变                | FCPE 基本逐帧独立解码，一帧错误即可产生明显跳变                                                         | 增加时间连续性后处理                  | 在原始 F0 与最终显示音高之间增加 temporal stabilizer，不直接修改原始曲线                | 10–40 ms 的孤立八度错误基本消失；真实持续转音不能被全部抹平                                     |
| P0  | 字幕主音选择                | 一个字存在多个 `pitch_notes` 时，当前导出逻辑存在直接取第一个音的风险                                         | 新增明确的 `primary_note` 选择逻辑   | 默认根据“持续时间 × 平均置信度”评分；简单情况下等价于选 voiced frames 占比最高的音             | `F5 40ms + C4 400ms` 必须显示 C4，而不是 F5                                    |
| P0  | LRC 加载后状态             | 后端 rebind 后，前端可能仍取得 rebind 前的数据                                                    | 修正加载 LRC 后的数据返回顺序           | LRC 写入 state → rebind → 从更新后的 state clone → 返回前端                | 导入歌词后 GUI 和后端导出得到的 `pitch_notes` 完全一致                                  |
| P0  | confidence threshold  | 歌词绑定存在固定阈值如 `0.3` 的情况，与 GUI 分析参数可能不一致                                              | 全局只使用同一套当前分析参数              | `AnalysisParams` 或必要参数保存在共享 state 中；rebind 使用当前参数               | 调整 GUI confidence 后，曲线、歌词标注与导出使用同一阈值                                   |
| P0  | 工程加载                  | GUI 加载项目后可能恢复了前端数据，但后端 `AppState`/播放器状态没有同步                                        | 修正 project load             | 加载项目时统一恢复 track、lyrics、analysis params、必要音频路径和后端状态              | 加载项目后无需重新分析即可继续字幕导出；GUI 与 backend 状态一致                                 |
| P1  | 原始 Pitch 与字幕 Pitch 混用 | 当前容易把“真实连续 F0 曲线”和“字幕显示的音名”当成同一结果                                                  | 明确拆成两层                      | 保留 Raw/Clean Pitch Track；额外产生 Annotation/Note Track             | 平滑字幕音名不会破坏原始 vibrato、滑音曲线                                              |
| P1  | Note Event            | 缺少稳定的离散音符事件                                                                        | 从 clean F0 构建 Note Events   | 生成 `{start, end, midi/note, confidence}`；短暂跳音不生成独立事件            | GUI/字幕主要读取 Note Event，不直接逐帧读 F0                                        |
| P1  | debounce              | 音名在半音附近反复闪动                                                                        | 给 Annotation Track 增加切换确认时间 | 候选新音持续约 50–80 ms 后才承认切换；参数集中管理                                  | 单帧/数帧跳变不改变字幕；真实连续音符变化仍可检测                                              |
| P1  | octave error          | 起音、气声、MSST artifact 容易产生 ±12 半音错误                                                  | 对短暂八度跳变增加额外惩罚               | 若与前后稳定音恰差约 ±12 半音且持续很短，则优先视为 octave error                       | `C4 → C5(20ms) → C4` 自动修复                                              |
| P1  | 半音边界抖动                | vibrato 穿越 MIDI 半音边界可能造成 C4/C#4 快速闪动                                               | 增加 hysteresis               | 当前音符已有状态时，需要超过边界一定 cents 或持续一定时间才切换                             | vibrato 时字幕音名稳定，除非确实形成新的稳定音                                            |
| P1  | voiced segment 起止     | 辅音、气声、尾音阶段 F0 不稳定                                                                  | 对起音/尾音设置不同策略                | 起音优先使用后续稳定区域判断；尾音置信度下降后不要频繁产生新音符                                | 每句第一字与最后一字不再成为异常跳音重灾区                                                  |
| P1  | LRC 字符处理              | 普通 LRC 只有整句时间，目前平均切字过于粗糙                                                           | 重做逐字时间分配                    | 普通 LRC 仍作为句级约束，在句内根据人声音频特征优化字边界                                 | 长短音明显不均匀的歌词不能继续机械平均分配                                                  |
| P1  | 字级 pitch 绑定           | 字的时间边界有少量误差时容易吃到相邻音符                                                               | 绑定时使用中心稳定区域                 | 可忽略 token 开头/结尾极短 margin，优先统计字内部稳定 voiced frames                | 边界附近 20–30 ms 的邻字音符不会轻易改变主音                                            |
| P1  | ASS 导出                | SRT 难以实现字上方音高和 karaoke                                                             | 增加最小化 ASS exporter          | 输出歌词、字级 timing、音高文本和 karaoke timing；不要开发字幕编辑器                   | ASS 可直接通过 ffmpeg/libass 烧录且时间同步                                        |
| P2  | RMVPE 后端              | FCPE 对部分复杂人声可能仍有 octave error                                                      | 在保持 FCPE 的情况下增加可选 RMVPE     | 抽象统一 `PitchDetector` 接口；默认继续 FCPE，RMVPE 作为高精度可选后端               | 同一音频可选择 FCPE/RMVPE，输出统一 Track 格式                                       |
| P2  | 双模型复核                 | 少数疑难区域单模型仍可能失败                                                                     | 仅实现可疑点复核，不做全程复杂 ensemble    | 若两模型差异超过阈值或恰好差 12 半音，可将区域标记低置信度                                 | 不自动武断选择错误答案；可疑区域在 GUI 有明确标记                                            |
| P2  | GUI 稳定性               | GUI 可能存在状态不同步和异常操作问题                                                               | 针对核心工作流系统检查 GUI             | 重点检查导入、分析、歌词、工程保存/加载、导出，不重做 UI                                  | 主流程连续操作不会报错、卡死或状态错乱                                                    |
| P2  | Regression Tests      | 修改平滑算法容易误伤真实转音                                                                     | 建立小型回归测试集                   | 使用合成轨迹和少量固定音频测试核心逻辑                                             | 后续修改必须通过短异常、八度错误、倚音、滑音、vibrato 等测试                                     |

---

# 三、音高处理架构调整

建议明确分为三个层级，不要继续将所有功能混在一条 F0 数组中。

| 层级                    | 内容                                             | 用途              | 是否允许强力平滑 |
| --------------------- | ---------------------------------------------- | --------------- | -------- |
| Raw Pitch Track       | FCPE/RMVPE 原始逐帧 F0 + confidence                | 调试、查看模型真实输出     | 否        |
| Clean Pitch Track     | confidence mask、异常岛修复、Hampel/median 等清理后的连续 F0 | 绘制真实音高曲线        | 轻度       |
| Annotation Note Track | 离散稳定音符事件                                       | 歌词标注、ASS/SRT 导出 | 可以较强防抖   |

推荐数据结构示意：

```rust
struct PitchFrame {
    time: f32,
    f0_hz: Option<f32>,
    confidence: f32,
}

struct NoteEvent {
    start: f32,
    end: f32,
    midi: i32,
    note_name: String,
    confidence: f32,
}
```

不要为了字幕稳定而过度平滑 `Clean Pitch Track`。

字幕使用 `NoteEvent`。

---

# 四、Temporal Stabilizer 具体建议

本轮不要直接上特别复杂的完整乐谱转录系统。

先实现一个简单、可解释、容易调试的稳定器即可。

| 情况                            | 建议行为                                      |
| ----------------------------- | ----------------------------------------- |
| 新音仅持续 `< 30–40 ms`            | 默认认为异常，不形成 Note Event                     |
| 新音持续 `50–80 ms` 以上            | 可以确认成为新音                                  |
| 新音与前后音恰差约 `12 semitones`，持续极短 | 强烈怀疑 octave error                         |
| 新音与主音只差 1 semitone，且反复来回      | 使用 hysteresis，避免 vibrato 触发闪烁             |
| 连续稳定滑音                        | Clean Pitch 保留；Annotation 层根据持续时间形成少量实际音符 |
| voiced segment 开头短异常          | 允许参考后续稳定音修正                               |
| voiced segment 结尾短异常          | 允许参考前方稳定音修正                               |
| 真实短倚音                         | 如果持续足够长且 confidence 高，不应全部删除              |

参数不要散落硬编码。

建议集中：

```rust
struct NoteTrackingParams {
    min_note_duration_ms: f32,
    switch_confirm_ms: f32,
    octave_error_max_ms: f32,
    semitone_hysteresis_cents: f32,
    edge_ignore_ms: f32,
}
```

默认值可以先从以下范围试验：

| 参数                          |        初始建议 |
| --------------------------- | ----------: |
| `min_note_duration_ms`      |    40–50 ms |
| `switch_confirm_ms`         |    50–70 ms |
| `octave_error_max_ms`       |       80 ms |
| `semitone_hysteresis_cents` | 20–35 cents |
| `edge_ignore_ms`            |    15–30 ms |

不要把这些数字当成理论常数，需要通过测试集校准。

---

# 五、每字主音高算法

禁止继续使用：

```rust
pitch_notes.first()
```

作为字幕默认音高。

建议流程：

```text
Token 时间范围
    ↓
获取范围内所有 voiced frames / NoteEvents
    ↓
剔除非常短、低 confidence 区域
    ↓
按照 MIDI note 聚合
    ↓
计算每个候选音：
duration × mean_confidence
    ↓
最大者作为 primary_note
```

建议：

```rust
score = voiced_duration * mean_confidence;
```

如果一个字明显包含多个持续音，可以保留：

```rust
pitch_notes: Vec<NoteEvent>
```

但字幕默认只显示：

```rust
primary_note
```

不要默认输出大量箭头、转音信息。

保持简洁。

如果未来需要详细模式，可以后续再做，不属于本轮必须功能。

---

# 六、LRC 对齐修改

这是自动字幕可靠性最关键的部分之一。

## 当前问题

普通 LRC：

```text
[01:23.45]如果有一天我变得很有钱
```

只知道整句话开始时间。

不能直接认为：

```text
整句时长 / 字符数 = 每字时长
```

因为真实演唱通常是：

```text
如——果 有 一天—— 我 变得 很 有钱——
```

## 本轮目标

不要求开发复杂大型 ASR 系统。

只需要比平均切分明显更可靠。

建议方法：

| 信息                 | 用途                  |
| ------------------ | ------------------- |
| LRC line start/end | 限定搜索范围              |
| voiced/unvoiced    | 判断真正发声区域            |
| RMS/energy change  | 辅助识别起音              |
| F0 onset/change    | 辅助音节/字边界            |
| 字符数量               | 约束最终必须得到 N 个 token  |
| 最小字时长              | 防止出现极端 5 ms 字       |
| 动态规划/代价函数          | 从候选边界中选择 N-1 个最佳切分点 |

不要要求每个字一定对应一个新音符。

因为：

```text
一个音可以唱多个字
一个字也可以唱多个音
```

歌词边界与 Note Event 边界只能作为相关信息，不能强行一一对应。

---

# 七、LRC 支持范围

本轮只需要：

1. 普通逐句 LRC；
2. 如果已经支持 enhanced LRC，则保留；
3. 不需要增加大量歌词格式；
4. 不需要联网自动搜索歌词；
5. 不需要歌词数据库。

如果导入的歌词本身已经带逐字时间：

> 优先使用原始逐字时间，不要重新自动估计。

如果只有句级时间：

> 才启动自动字级时间估计。

---

# 八、ASS 字幕输出

ASS 是本项目制作音高测量视频最值得增加的一个输出。

不要开发复杂字幕 GUI。

只实现 exporter。

## 最低要求

支持：

```text
音高
歌词
```

例如视觉逻辑：

```text
      E4      F#4      G4
      我       想       飞
```

或者通过 ASS 的定位实现音高文字位于对应歌词上方。

要求：

1. 字级时间正确；
2. 支持 karaoke timing；
3. UTF-8；
4. 音高和歌词可分别设置基础字号；
5. 有默认字体/描边；
6. 导出的 `.ass` 能被 ffmpeg/libass 正常烧录。

不要增加：

- 视频编码；
- 视频预览器；
- 时间轴剪辑；
- 自动下载视频；
- 内置 ffmpeg 视频工作站。

这些不是 Pitch Analyzer 应负责的功能。

---

# 九、SRT 输出调整

保留 SRT，但作为简单输出。

建议每个 token 使用：

```text
歌词 [主音]
```

或者整句：

```text
我[E4] 想[F#4] 飞[G4]
```

必须使用 `primary_note`，不能直接使用第一个检测音。

SRT 无需支持复杂样式。

---

# 十、FCPE 部分

暂时不要删除 FCPE。

当前首先解决：

1. 后处理；
2. temporal continuity；
3. note event；
4. token primary note；
5. 字级对齐。

这些问题解决前，单纯换模型不会解决整体体验。

如果当前 FCPE decoder 是逐帧 argmax / 局部加权：

保留 Raw F0 输出，但在之后增加 temporal processing。

本轮没有必要马上实现复杂 Viterbi decoder。

如果后续发现 stabilizer 仍不足，再考虑：

```text
FCPE logits
↓
候选 pitch states
↓
Viterbi / dynamic programming
↓
最佳连续路径
```

不要第一版就过度复杂化。

---

# 十一、RMVPE 支持

RMVPE 属于 P2，可做，但不能拖累前面核心修复。

要求：

```rust
trait PitchDetector {
    fn analyze(...) -> PitchTrack;
}
```

后端：

```text
FCPE
RMVPE
```

输出统一格式。

GUI 只需要：

```text
Pitch engine:
[FCPE ▼]
```

不要给用户暴露几十个模型内部参数。

只保留真正有意义的：

- pitch engine；
- confidence threshold；
- minimum note duration；
- stabilization strength（如果需要）。

避免 GUI 参数爆炸。

---

# 十二、可疑区域标记

如果实现 RMVPE，可加入非常轻量的复核逻辑。

例如：

```text
FCPE = E4
RMVPE = E4
→ normal

FCPE = E5
RMVPE = E4
→ suspicious_octave

FCPE = C5
RMVPE = G4
→ low_confidence
```

GUI 只需要给这些 token 一个明显标记，例如：

```text
⚠
```

用户点击即可查看。

不要开发复杂人工标注系统。

目的只是：

> 自动分析完以后，只检查少量可疑字。

---

# 十三、GUI Bug 检查范围

本轮 GUI 不重做设计。

只检查核心状态和操作可靠性。

| 操作            | 必须检查                    |
| ------------- | ----------------------- |
| 打开音频          | 文件切换后旧 analysis 是否被正确清空 |
| 分析            | 参数是否真正传给 backend        |
| 修改 confidence | 曲线、歌词、导出是否一致            |
| 导入 LRC        | 是否立即 rebind             |
| 重新分析          | 旧歌词是否正确重新绑定             |
| 保存工程          | 是否保存必要状态                |
| 加载工程          | frontend/backend 是否都恢复  |
| 导出 SRT        | 是否使用当前最新歌词              |
| 导出 ASS        | 是否使用当前最新歌词              |
| 修改音频后加载旧工程    | 是否产生错误状态                |
| 无音频状态         | 按钮是否优雅报错                |
| 无歌词状态         | 字幕导出是否明确提示              |
| 无分析结果         | 不允许生成伪数据                |

特别检查：

```text
Frontend state
Backend AppState
AudioPlayer
ProjectData
```

四者之间是否一致。

---

# 十四、建议增加的自动测试

不要建立庞大测试系统。

只针对容易回归的音高算法做少量固定测试。

| Test            | 输入                      | 期望                       |
| --------------- | ----------------------- | ------------------------ |
| 短暂八度错误          | `C4 C4 C5(20ms) C4 C4`  | C5 被清除                   |
| 句首八度错误          | `C5(30ms) → C4(500ms)`  | primary = C4             |
| 真实倚音            | `D4(100ms) → E4(400ms)` | D4 可以保留为 event           |
| 半音抖动            | C4 附近 vibrato 穿越半音边界    | Annotation 不疯狂 C4/C#4 切换 |
| 真正换音            | `C4 300ms → E4 300ms`   | 两个 NoteEvent             |
| 极短噪声            | voiced 10ms             | 不生成 note                 |
| 尾音错误            | `E4 500ms → E5 20ms`    | E5 被清理                   |
| 字内错误首音          | `F5 40ms + C4 400ms`    | primary = C4             |
| 一个字两个真音         | `C4 180ms + D4 250ms`   | primary=D4，同时保留两个 event  |
| 无 voiced frames | 全 confidence 低          | token pitch = None       |

最好让测试直接针对 DSP/lyrics 模块，不依赖 GUI。

---

# 十五、建议代码组织

不要为了本轮任务大规模重构。

如果当前结构允许，可以增加：

```text
dsp.rs
    raw/clean pitch processing

note_tracker.rs
    Clean Pitch → NoteEvent

lyrics.rs
    token timing
    token ↔ NoteEvent binding
    primary note

export/
    srt.rs
    ass.rs
```

如果目前文件规模不大，也不必强行拆文件。

原则：

> 只在模块职责明显混乱时拆分。

---

# 十六、核心算法执行顺序

建议最终固定成：

```text
Audio
  ↓
Pitch Detector
  ↓
Raw Pitch Track
  ↓
Confidence Mask
  ↓
Short Error / Octave Error Repair
  ↓
轻度 Hampel / Median / Savitzky-Golay
  ↓
Clean Pitch Track
  ↓
Temporal Stabilizer
  ↓
Note Event Tracker
  ↓
LRC Line Timing
  ↓
Automatic Token Alignment
  ↓
Token ↔ Note Event Binding
  ↓
Primary Note Selection
  ↓
GUI
  ↓
SRT / ASS
```

其中：

```text
Clean Pitch Track
```

用于绘图。  

```text
Note Event Track
```

用于字幕。

不要反过来拿字幕平滑后的音符覆盖真实音高曲线。

---

# 十七、本轮明确不要做的功能

为了避免 Codex 把项目越做越胖，本轮明确不要增加：

| 不要做                    | 原因                       |
| ---------------------- | ------------------------ |
| MSST 人声分离集成            | 用户可以提前准备纯人声，不属于核心音高分析    |
| 视频导入/剪辑                | 与项目职责无关                  |
| 视频硬字幕压制                | ASS 输出后交给 ffmpeg 即可      |
| 自动上传 B 站               | 完全无关                     |
| 在线歌词搜索                 | 非核心                      |
| 歌词数据库                  | 非核心                      |
| 自动识别歌曲                 | 非核心                      |
| 大型 ASR 模型              | 当前字级对齐先使用音频特征即可          |
| 完整乐谱/MIDI 转录           | 当前只需要歌词音高标注              |
| 和弦检测                   | 无关                       |
| 调性检测                   | 无关                       |
| BPM 检测                 | 非本轮需要                    |
| 大量可视化特效                | 无关                       |
| 复杂字幕编辑器                | 不需要                      |
| 内置播放器大改版               | 只修 bug                   |
| 大量可调 DSP 参数            | 会增加使用负担                  |
| 强制 FCPE+RMVPE ensemble | 计算和复杂度不值得                |
| 第一版直接 Viterbi 全重写      | 优先简单 temporal stabilizer |
| 自动删除所有短音               | 会误删真实倚音/转音               |

---

# 十八、推荐开发顺序

严格按以下顺序推进。

## Stage 1：修复确定性问题

1. 修复首尾 short pitch island。
2. 修复字幕 `first pitch` 选择问题。
3. 修复 LRC rebind 返回旧数据。
4. 去掉歌词绑定中的硬编码 confidence。
5. 修复加载工程时 frontend/backend state 不同步。
6. 添加对应单元测试。

完成后先测试现有程序。

---

## Stage 2：Annotation Note Track

实现：

1. `NoteEvent`；
2. minimum duration；
3. debounce；
4. hysteresis；
5. short octave suppression；
6. primary note。

目标：

> 不改变真实 F0 曲线的情况下，使字幕音高稳定。

这是解决“音高疯狂跳动”的核心阶段。

---

## Stage 3：改进 LRC 字级对齐

把：

```text
line duration / character count
```

从默认算法中移除。

实现：

```text
LRC line constraint
+
voiced region
+
energy/onset
+
pitch change
+
字符数量约束
```

得到更合理的 token timing。

保留平均分配作为：

```text
fallback
```

而不是主要算法。

---

## Stage 4：ASS

实现简单 ASS exporter：

```text
字级 timing
音高
歌词
karaoke
```

能够直接：

```bash
ffmpeg ... subtitles=xxx.ass ...
```

即可。

不要继续开发视频功能。

---

## Stage 5：可选 RMVPE

仅在前面全部稳定以后：

1. 抽象 PitchDetector；
2. FCPE 保持默认；
3. 增加 RMVPE；
4. 必要时标记模型分歧区域。

---

## Stage 6：GUI 核心流程回归测试

完整走一遍：

```text
打开音频
→ 分析
→ 导入 LRC
→ 自动逐字
→ 查看结果
→ 保存工程
→ 重启
→ 加载工程
→ 导出 ASS
```

所有状态必须一致。

---

# 十九、最终验收场景

使用一首已经通过 MSST 等工具分离出的完整人声 WAV，加普通 LRC。

用户操作应尽量只需要：

```text
1. 打开 vocals.wav
2. 点击 Analyze
3. 导入 lyrics.lrc
4. 点击 Auto Align（如果不是自动执行）
5. 检查少量 ⚠ 项
6. 导出 ASS
```

最终字幕应该做到：

- 大部分字自动获得合理时间；
- 大部分有声音的字自动获得正确主音；
- 每句第一个字不再经常显示几十毫秒的离谱八度；
- vibrato 不造成字幕音名疯狂闪烁；
- 真正持续的换音仍能识别；
- 一个字的短暂错误起音不会抢走真正主音；
- GUI 显示内容与导出的字幕一致；
- 保存并重新打开工程后结果一致；
- ASS 可以直接用于视频硬字幕。

---

# 二十、开发原则

本轮最重要的原则：

> **优先修算法链路和状态一致性，不追求功能数量。**

优先级：

```text
正确
>
稳定
>
自动化
>
速度
>
功能数量
```

尤其不要为了“音高稳定”直接对 F0 做非常猛烈的平滑。

真实歌声中：

- vibrato；
- portamento；
- 倚音；
- 转音；
- 快速换音；

都是真实信息。

正确架构应该是：

```text
真实 Pitch Curve 尽量忠实
+
字幕 Annotation Track 尽量稳定
```

二者分离处理。

---

# 二十一、Codex 开发要求

请在修改前先完整阅读当前仓库代码，并根据现有实现确认以上问题是否仍存在。

不要因为本任务书中提到某个函数名，就在代码已经变化的情况下机械修改。

开发时：

1. 尽量沿用现有架构；
2. 避免无必要的大规模重构；
3. 每完成一个 Stage 即运行现有测试/构建；
4. 新增的 DSP 逻辑必须有单元测试；
5. 不删除现有可用功能；
6. 不新增与本任务书核心目标无关的功能；
7. 不因为修字幕音高而破坏真实 F0 曲线；
8. 所有默认参数集中管理，避免散落 magic numbers；
9. 如果发现本任务书描述与当前代码不符，以实际代码为准，但保持上述功能目标；
10. 修改完成后输出：
    - 修改文件列表；
    - 修复的问题；
    - 新增算法；
    - 默认参数；
    - 测试结果；
    - 仍然存在的已知限制。

最终项目定位保持为：

> **专注于歌声音高分析和歌词音高字幕生成的轻量工具。**

不要将其扩展成 DAW、视频编辑器、源分离平台或完整乐谱转录软件。
