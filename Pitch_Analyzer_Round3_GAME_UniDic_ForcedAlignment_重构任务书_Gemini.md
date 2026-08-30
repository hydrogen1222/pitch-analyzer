# Pitch Analyzer Round 3：GAME × UniDic × Forced Alignment × Mora–Note Matching 重构任务书

> 面向开发 Agent：Gemini / OpenCode
>
> 仓库：`https://github.com/hydrogen1222/pitch-analyzer`
>
> 本轮性质：**架构级重构，不是继续调几个阈值**。
>
> 核心目标：把“连续 F0”“音乐意义上的音符”“歌词发音时间”“显示字形”四件事彻底解耦，再在最后一层做可解释、单调、可调试的绑定。

---

# 0. 给开发 Agent 的执行约定（先读）

这份任务书希望尽量一次性把需求讲清楚。请不要把用户当“传话筒”，除非遇到真正无法由工程判断解决的问题，否则请自行完成调查、实现、测试、修正和文档更新。

## 0.1 本地 HEAD 优先，不要盲目照搬旧代码片段

开始前必须先确认本地仓库的真实状态：

```bash
git status
git rev-parse HEAD
git log -1 --oneline
git branch --show-current
```

然后重点审计：

```text
src-tauri/src/analyzer.rs
src-tauri/src/note_tracker.rs
src-tauri/src/lyrics.rs
src-tauri/src/models.rs
src-tauri/src/mel.rs
src-tauri/src/lib.rs
src/karaoke_display.ts
src/main.ts
src/models_lyrics.ts
src-tauri/tests/
models/
src-tauri/resources/
```

公开 GitHub 页面/搜索缓存可能比用户本地 HEAD 滞后。**若本地已有上一轮的新数据结构、新 UI、新测试，则保留并迁移，不准为了套本任务书而回退。**

本任务书描述的是“目标架构、原则、验收标准”，不是要求机械复制某段旧实现。

## 0.2 禁止“接口已预留 = 功能已完成”

上一轮最大的问题之一是可能出现：

```text
enum 里有 ForcedAlign
trait 里有 ReadingProvider
注释里写支持 UniDic
UI 里有详细音高
```

但真正运行时仍走旧 fallback。

本轮严格规定：

- 只有真实数据流跑通，才能标记为 implemented。
- 只有真实模型被加载并参与推理，才能写“GAME 已接入”。
- 只有汉字真正获得词典读音，才能写“UniDic 已接入”。
- 只有音频 + 已知歌词真正经过声学模型得到 mora/phoneme 时间，才能写“Forced Alignment 已完成”。
- 只定义 enum / struct / TODO / placeholder / mock 均不算完成。

## 0.3 不要继续通过无限调参修架构问题

禁止把本轮退化为继续尝试：

```text
60 ms → 80 ms → 120 ms
25 cents → 40 cents
汉字 1.8 → 1.7 → 2.0
edge_ignore 20 ms → 30 ms
```

这些参数可以在新架构里保留为局部调节，但不能再承担核心语义。

---

# 1. 当前问题的最终诊断

用户当前可见症状包括：

1. 详细音高模式曾出现“几乎每个字都有多个转音”。
2. 后续 suppression 后，假转音减少，但仍存在：
   - 漏音高；
   - 音高明显绑错字；
   - 某些字显示多个音但听感并非真正 melisma；
   - 汉字附近尤其容易错位；
   - 一些句子后半段越对越歪。
3. 用户希望真正保留一字多音，例如中文流行歌曲里一个“我”字可以清晰唱成多个目标音，而不是把 vibrato / glide 都当成换音。

这些现象不是单一模块造成，而是以下四层被耦合后的结果：

```text
A. Continuous F0 estimation
B. Musical note transcription
C. Lyrics pronunciation + acoustic timing
D. Display glyph ↔ notes aggregation
```

旧架构在不同程度上让 FCPE F0 同时承担 A+B，并让字符权重 + 声学峰值猜 C，最后通过时间重叠直接做 D。

这是目前精度天花板的根本来源。

---

# 2. 本轮最重要的架构决策：三条“真值轨”

最终必须明确三条彼此独立的数据流。

```text
                   ┌──────────────────────┐
音频 ─────────────→│ Continuous F0 Track  │
                   │ FCPE                 │
                   └──────────┬───────────┘
                              │
                              └── 用于真实 F0 曲线 / 当前 MIDI 数值

                   ┌──────────────────────┐
音频 ─────────────→│ Musical Note Track   │
                   │ GAME                 │
                   └──────────┬───────────┘
                              │
                              └── 用于 C4 / D4 / E4 等“音乐音符事件”

日语歌词 ─→ UniDic / override
             │
             ↓
        Reading / Mora / Phoneme
             │
             ↓
        Forced Alignment
             │
             ↓
        Mora Timing Track
             │
             └──────────────┐
                            ↓
                    Mora ↔ Note Matcher
                            ↓
                       Display Token
```

必须把下面这条原则写进代码注释和设计文档：

> **FCPE 回答“这一帧真实 F0 是多少”；GAME 回答“音乐意义上这里是什么音符”；Forced Aligner 回答“这一个发音单位什么时候唱到”；Matcher 才回答“屏幕上这个字显示什么音”。**

任何一个模块不允许越权“猜”另一个模块的语义。

---

# 3. 非目标（本轮不要无边界扩需求）

以下内容本轮不要求：

- 不做完整 DAW / Melodyne 编辑器。
- 不做自动修音。
- 不做歌手识别。
- 不做完整伴奏分离系统重构；若现有输入是混音，先允许 GAME 直接工作，未来再增加可选 vocal separation。
- 不为了 forced alignment 引入完整 Whisper ASR 转录链。
- 不让 ASR 重写用户给定歌词。
- 不做日语语法教学或 pitch accent 词典。
- 不要求自动解决所有当て字 / 熟字训 / 作词人特殊读法；这类必须由 override 兜底。
- 不要求中文、英文在本轮达到日语同等级的 G2P/forced-alignment 质量；但数据结构必须语言无关。

---

# 4. Phase 0：建立可复现 baseline，禁止“凭截图调算法”

在大改前先建立对照基线。

## 4.1 新建 benchmark / fixture 目录

建议：

```text
benchmarks/
  synthetic/
  japanese_private/
  chinese_private/
  expected/
  scripts/
```

注意：版权歌曲音频不要提交进公开 Git 仓库。

可以使用：

- 用户本地私有路径；
- `.gitignore` 的 private fixtures；
- 自己录制的短样本；
- 程序生成的合成 F0 / sine / vowel 测试。

## 4.2 记录现有结果

至少保存：

```text
legacy_note_events.json
legacy_token_timing.json
legacy_token_notes.json
```

每条包含：

```json
{
  "line": "流れる人の群れ",
  "tokens": [
    {
      "text": "流",
      "start": 0.0,
      "end": 0.0,
      "notes": []
    }
  ]
}
```

后续每个 Phase 都必须能 A/B 比较。

---

# 5. Phase 1：先修现有确定性问题（P0）

这些修复与 GAME / UniDic 是否完成无关，应该先做。

## 5.1 日语 small kana Unicode 必须单测

不要手写错误 codepoint。

应覆盖：

```text
ゃ U+3083
ゅ U+3085
ょ U+3087
ャ U+30E3
ュ U+30E5
ョ U+30E7
ぁぃぅぇぉ
ァィゥェォ
ゎ ヮ
```

建议不要再用散落的十六进制 magic number + 错误注释；可以直接使用字符 literal。

必须加入测试：

```text
きゃ → 1 mora
しゅ → 1 mora
ちょ → 1 mora
キャ → 1 mora
```

## 5.2 `ー` 的“显示合并”和“mora 数”必须分离

旧逻辑容易把：

```text
ラー
```

视为一个拍。

这是错误的。

正确区分：

```text
UI tokenization:
ラ + ー 可以显示为一个视觉 token

Mora:
ラ = 1
ー = 1
```

例如：

```text
スーパー = ス・ー・パ・ー = 4 mora
```

数据结构上禁止用 `merge_japanese_attach_chars()` 的视觉合并结果直接等价为 mora 数。

## 5.3 lyric boundary score 中移除“换音 = 强歌词边界”的假设

若现有 DP 仍有类似：

```rust
if rounded_midi_changed {
    score += ...;
}
```

应删除或大幅降权。

原因：

```text
一个 mora 可以跨多个音（melisma）
一个音也可跨多个 mora（连唱/弱音）
F0 change ≠ phonetic boundary
```

DP fallback 最多使用：

- silence / VUV transition；
- energy valley；
- spectral flux；
- onset-like feature；

但以后 forced alignment 完成后，DP fallback 降级为兜底。

## 5.4 Binder 必须禁止 phantom note

正式 `PitchNote` 必须有真实时间交集。

禁止：

```text
token 没有真实 overlap
只因为 note 在 ±30 ms 附近
→ 直接赋给 token
```

容差可以用于“候选发现”，不能凭空生成正式绑定。

建议统一计算：

```text
overlap_abs = intersection duration
overlap_token_ratio = overlap / token_duration
overlap_note_ratio = overlap / note_duration
```

正式入选至少满足之一，例如：

```text
overlap_abs >= 30 ms
OR overlap_token_ratio >= 0.20
```

具体阈值可以用 benchmark 调，但必须有**正 overlap**。

## 5.5 详细音高必须按时间排序

任何通过 `HashMap` / set 聚合后的 notes，都必须最后：

```text
sort_by(start_time)
```

详细 UI 的：

```text
A3 → C4 → D4
```

必须表示真实时间顺序，不能受容器迭代顺序影响。

## 5.6 空音高必须有 reason code

不要只剩 `None`。

内部增加：

```rust
enum NoteBindingStatus {
    Matched,
    NoVoicing,
    NoMusicalNote,
    AlignmentLowConfidence,
    NoTemporalOverlap,
    SuppressedTransition,
    Unknown,
}
```

普通 UI 可以继续不显示，但 Debug UI 必须能看见原因。

---

# 6. Phase 2：数据模型重构

## 6.1 Continuous F0 Track

保留当前 FCPE 输出，不要为了 GAME 改掉真实曲线。

推荐：

```rust
struct ContinuousPitchTrack {
    times: Vec<f32>,
    frequencies: Vec<f32>,
    midis: Vec<f32>,
    confidences: Vec<f32>,
    rms: Vec<f32>,
}
```

## 6.2 Musical Note Track

不要再把 `NoteEvent` 默认解释成 FCPE round 后的 run。

建议：

```rust
enum MusicalNoteSource {
    Game,
    LegacyFcpeTracker,
    ImportedMidi,
}

struct MusicalNoteEvent {
    id: u32,
    start: f32,
    end: f32,

    // GAME 可能给浮点 pitch，必须保留
    midi_float: f32,
    midi_rounded: i32,
    note_name: String,

    confidence: f32,
    source: MusicalNoteSource,

    // 可选，用于调试/未来 word-conditioned GAME
    boundary_confidence: Option<f32>,
    is_slur: Option<bool>,
}
```

核心原则：

> **存储时保留 float pitch；显示 badge 时才 round 到最近半音。**

## 6.3 Reading / Mora / Phoneme

建议：

```rust
struct ReadingSpan {
    display_start: usize,
    display_end: usize,
    surface: String,
    reading: String,
    pronunciation: String,
    source: ReadingSource,
    confidence: f32,
}

enum ReadingSource {
    UniDic,
    KanaLiteral,
    UserOverride,
    Fallback,
}

struct MoraUnit {
    id: u32,
    text: String,
    reading_span_id: u32,
    display_start: usize,
    display_end: usize,

    start: Option<f32>,
    end: Option<f32>,
    alignment_confidence: Option<f32>,
}
```

必要时再下沉：

```rust
struct PhonemeUnit {
    symbol: String,
    mora_id: u32,
    start: Option<f32>,
    end: Option<f32>,
}
```

## 6.4 Display token 只负责“显示”

`LyricToken` 不再同时承担“发音基本单位”。

应允许：

```text
愛
└── ReadingSpan: あい
    ├── mora: あ
    └── mora: い

二人
└── ReadingSpan: ふたり
    ├── ふ
    ├── た
    └── り
```

同样允许：

```text
運命
reading override: さだめ
```

不要强迫：

```text
運 = さ
命 = だめ
```

因为这个逐汉字映射在语言学上未必唯一。

## 6.5 ProjectData 版本迁移

现有项目文件不能突然炸掉。

增加：

```text
schema_version
```

加载旧项目时：

- FCPE pitch track 可保留；
- legacy note events 标记 `LegacyFcpeTracker`；
- 旧 token timing 可以加载，但若重新分析则使用新 pipeline；
- 不要 silently reinterpret old note events 为 GAME notes。

---

# 7. Phase 3：GAME 成为 canonical Musical Note Engine

## 7.1 选型

采用：

**OpenVPI GAME — Generative Adaptive MIDI Extractor**

官方仓库：

`https://github.com/openvpi/GAME`

原因：

- 专门面向 singing voice → note sequence；
- 自带 boundary extraction；
- 可输出 note onset / offset / pitch；
- 支持已知 word boundary 条件；
- MIT；
- 官方支持 ONNX export；
- 2026 v1.0.3 Release 提供 opset 17 ONNX 模型；
- 比“FCPE 连续 F0 + 自己猜 note boundary”更符合任务定义。

## 7.2 不要一开始就用 Rust 盲猜 ONNX tensor

实现顺序必须是：

### Stage A：建立官方 Python reference

用 GAME 官方 inference 在本地 fixture 上输出：

```text
game_reference.json
```

格式统一为：

```json
[
  {
    "start": 1.23,
    "end": 1.64,
    "midi_float": 69.05
  }
]
```

这是 Rust ONNX 实现的 golden reference。

### Stage B：Rust ONNX native

项目已经使用 `ort`，优先复用。

不要引入 Python 作为最终强依赖，除非 ONNX native 短期确实不可行。

官方/相关实现显示 GAME 模型目录通常包含：

```text
config.json
encoder.onnx
segmenter.onnx
bd2dur.onnx
dur2bd.onnx
estimator.onnx
```

**请以 GAME v1.0.3 实际 release asset + deployment API 为准，不准凭文件名猜输入输出 shape。**

可参考：

- GAME `deployment/api.py`
- GAME `preprocessing/api.py`
- GAME `inference/api.py`
- OpenVPI `dataset-tools` 的 `GameInfer`

`dataset-tools` 是 Apache-2.0，可作为实现思路参考。

## 7.3 模块建议

```text
src-tauri/src/note_engine/
  mod.rs
  game.rs
  game_config.rs
  game_preprocess.rs
  game_decode.rs
  legacy_fcpe.rs
```

trait：

```rust
trait MusicalNoteEngine {
    fn transcribe(
        &self,
        audio: &[f32],
        sample_rate: u32,
        optional_boundaries: Option<&[f32]>,
    ) -> Result<Vec<MusicalNoteEvent>>;
}
```

## 7.4 GAME 模型管理

不要把超大模型硬编码进 exe。

建议沿用现有模型管理逻辑：

```text
models/GAME/
  config.json
  encoder.onnx
  ...
```

UI 应明确状态：

```text
FCPE: loaded
GAME: loaded / missing
Japanese aligner: loaded / optional
```

若 GAME 缺失：

- 连续 F0 仍可使用；
- note badge 可回退 Legacy；
- UI 明确 `Legacy note engine`，不能悄悄冒充 GAME。

## 7.5 Execution Provider

第一目标是稳定，不要先优化到爆炸。

顺序：

1. CPU ONNX baseline 必须工作。
2. Windows 可选 DirectML/CUDA（若现有 runtime 架构方便）。
3. Linux CPU/CUDA 再优化。

测试必须覆盖 CPU。

## 7.6 GAME 输出后不要再二次 `round-run` 重分割

GAME 已经做 musical note transcription。

允许的后处理：

- 删除明显无效极短 event（仅有充分理由）；
- octave sanity；
- merge 完全同 pitch 且间隔极短的重复 event；
- 显示时 round pitch。

禁止重新：

```text
GAME note → 转成 frame → midi.round → legacy NoteTracker 再切一次
```

否则等于把新架构又打回旧架构。

## 7.7 FCPE NoteTracker 降级

旧 `note_tracker.rs` 不需要删。

改名/定位：

```text
LegacyFcpeNoteTracker
```

用途：

- GAME 模型缺失时 fallback；
- A/B benchmark；
- regression reference。

但默认 canonical note source = GAME。

---

# 8. Phase 4：真正接入 UniDic，而不是汉字≈1.8

## 8.1 Rust 原生优先

推荐 Lindera + UniDic。

2026-08 可用版本包括 Lindera 6.x，`lindera-unidic` 专门提供 UniDic。

参考：

`https://docs.rs/lindera/latest/lindera/`

`https://docs.rs/lindera-unidic/latest/`

版本策略：

- 优先 pin 一个具体 minor/patch；
- 若最新 6.x 在当前 toolchain 构建不稳定，允许 pin 5.3.x；
- 不要写模糊 `*`。

## 8.2 UniDic 字段选择

UniDic 中有多种 reading/pronunciation 字段。

对“实际发音序列”优先使用 pronunciation/phonological surface form；若缺失，再 fallback 到 reading。

不要只取 lemma reading，否则活用/音便场景可能不自然。

实际索引必须根据 Lindera 返回的 UniDic feature schema 验证后编码，不要凭本任务书写死数字。

## 8.3 Reading pipeline

流程：

```text
raw display text
  ↓
Unicode normalization
  ↓
Lindera segmentation
  ↓
(surface, byte range, reading/pronunciation)
  ↓
ReadingSpan
  ↓
normalize kana
  ↓
Mora parser
```

## 8.4 Mora parser 必须正确处理

至少：

```text
普通假名         = 1 mora
きゃ / しゅ / ちょ = 1 mora
っ                = 1 mora
ん                = 1 mora
ー                = 1 mora
小写母音与前音组合需按实际音节规则处理
```

测试：

```text
愛       → あ・い            = 2
二人     → ふ・た・り        = 3
心       → こ・こ・ろ        = 3
嵐       → あ・ら・し        = 3
流れる   → な・が・れ・る    = 4
群れ     → む・れ            = 2
スーパー → ス・ー・パ・ー    = 4
きょう   → きょ・う          = 2
がっこう → が・っ・こ・う    = 4
```

## 8.5 Display mapping 采用 span，不强拆汉字

例如：

```text
二人
surface byte span = [x..y]
reading = ふたり
moras = ふ, た, り
```

这三个 mora 可以都关联到 display span `二人`。

最后 UI 若仍想逐“字”排版，可以做 display-level aggregation，但不要伪造语言学上不存在的逐汉字 reading。

## 8.6 Reading override

必须真正在 pipeline 中生效。

建议项目数据：

```json
{
  "reading_overrides": [
    {
      "line_index": 12,
      "surface": "運命",
      "reading": "さだめ"
    }
  ]
}
```

优先级：

```text
UserOverride > UniDic > KanaLiteral > Fallback
```

## 8.7 `汉字 = 1.8` 只能作为最后 fallback

保留是可以的，但必须满足：

```text
UniDic 未加载 / 解析失败
AND 无 override
```

Debug 中显示：

```text
alignment degraded: heuristic kanji weight
```

不能默默使用。

---

# 9. Phase 5：Forced Alignment，彻底替换“谱通量猜歌词边界”

## 9.1 定义

这里不是 ASR。

输入：

```text
已知歌词 reading / mora / phoneme
+
音频
```

输出：

```text
每个 phoneme/mora 的起止时间
```

模型不允许改写歌词文字。

## 9.2 优先按 LRC 行做受约束 alignment

已有 LRC 行时间是宝贵先验。

不要整首歌完全自由 align。

对于一行：

```text
line_start - pre_margin
→
line_end + post_margin
```

在局部窗口做 forced alignment。

优点：

- 大幅降低跳行；
- 降低重复副歌匹配歧义；
- 计算更省；
- 失败可局部 fallback。

## 9.3 实现路线分两层

### 路线 A：先用成熟 aligner 做 accuracy oracle

可用：

#### pyshiro

`https://github.com/wavtechyukky/pyshiro`

优点：

- 专门针对 Japanese singing voice；
- HSMM forced alignment；
- 很适合验证“理论上能不能对齐好”。

缺点：

- GPLv3。

因此：

> 可以用于本地 benchmark / oracle，不建议直接把其代码嵌入 MIT/Apache 核心。

### 路线 B：CTC forced alignment 做产品实现

可以先参考 MMS-300m forced aligner / karaoke-jp 的做法。

注意：常见 `mms-300m-1130-forced-aligner` 权重为 CC-BY-NC 4.0。

因此若使用：

- 不要把权重直接作为 MIT/Apache 项目资产混在一起；
- 采用可选 download-on-demand；
- UI/README 清晰标注模型许可证；
- 项目未来若考虑商用，需要可替换 permissive model。

不要为了快速跑通而隐藏 license 风险。

## 9.4 推荐工程策略

### P5a：先跑通 Python helper prototype

例如：

```text
scripts/forced_align_jp.py
```

输入 JSON：

```json
{
  "audio_path": "...",
  "line_start": 120.0,
  "line_end": 123.5,
  "reading": "こころをひきさくあらしのなか",
  "moras": ["こ", "こ", "ろ", "を", "ひ", "き", ...]
}
```

输出：

```json
{
  "units": [
    {"mora":"こ", "start":120.12, "end":120.31, "confidence":0.91}
  ]
}
```

### P5b：建立 fixture 验证后，再决定是否 native ONNX

如果 Python helper 能显著优于旧 DP：

- 保留其作为实验 backend；
- 再评估转 ONNX/Rust；
- 数据模型不要绑定 Python 实现。

这样不会一上来花一周重写 ONNX，最后才发现模型本身不适合 singing。

## 9.5 Forced alignment 失败必须优雅降级

每行记录：

```rust
enum AlignmentSource {
    EnhancedLrc,
    ForcedAlign,
    AcousticMoraDp,
    WeightedFallback,
}
```

优先级：

```text
Enhanced LRC precise timing
>
Forced Align
>
Mora-aware acoustic DP
>
Weighted fallback
```

并记录 confidence。

---

# 10. Phase 6：Mora ↔ GAME Note 的单调 many-to-many Matcher

这是最终准确性的关键，不允许再简单“字符时间窗口里有什么 NoteEvent 就全拿来”。

## 10.1 基本事实

真实歌唱存在：

```text
1 mora → 1 note
1 mora → N notes     (melisma / 转音)
N moras → 1 note     (同音连唱)
```

所以是一种受约束 many-to-many 映射。

## 10.2 输入

```text
MoraSpan[]
MusicalNoteEvent[] from GAME
```

它们都按时间单调排序。

## 10.3 推荐匹配原则

首先通过真实时间 overlap 生成候选。

score 建议包含：

```text
absolute overlap
mora coverage ratio
note coverage ratio
center distance
boundary distance
alignment confidence
GAME note confidence
```

但匹配结果必须满足全局单调：

```text
后面的 mora 不得绑定到更早的 note，除非共享同一个跨越 note
```

可以实现为：

- bounded greedy monotonic；或
- 小型 dynamic programming。

先求可解释稳定，不需要做复杂神经网络。

## 10.4 真 melisma 的定义

一个 mora 显示多个 note，必须是 GAME 给出的多个独立 musical events，并且：

- 与 mora 有显著 overlap；
- 时间顺序明确；
- 各 event 自身达到 minimum evidence；
- 不是 FCPE frame quantization 的产物。

于是：

```text
我
G4 → A4 → G4
```

是合法结果。

而 vibrato：

```text
G#4 / G4 / A♭4 在连续 F0 中摇摆
```

只要 GAME 仍认为是一个 G4，就只显示 G4。

## 10.5 Glide 的语义

若 GAME 输出：

```text
A4 → B4
```

中间 FCPE 曲线连续扫过 A#4：

```text
详细 badge = A4 → B4
曲线仍展示 A4→A#4→B4 的连续变化
```

绝不添加假的 A#4 note badge。

## 10.6 Primary note

普通模式 `primary_note` 可继续按：

```text
effective overlap duration × note confidence
```

但候选必须来自最终 matcher 的合法匹配，不能从全局 note events 直接抢。

若一字真的有两个几乎等长音：

- 普通模式显示占比最高；
- 详细模式显示全部。

---

# 11. Phase 7：UI 重构——三档语义，而不是一个“详细音高”开关包打天下

建议：

## 11.1 普通模式

显示：

```text
愛   が   終   わ   る
D4  A#4  A#3 ...
```

每个 display token 只显示 primary musical note。

## 11.2 详细音高模式

显示真正 musical events：

```text
我
G4 → A4 → G4
```

只来源于 GAME/matched note track。

不要展示所有 FCPE frame label。

## 11.3 Debug 模式

点击/hover 某个 token 显示：

```text
surface: 心
reading span: 心
pronunciation: ココロ
moras: [こ, こ, ろ]
reading source: UniDic

line: 132.410 – 135.030
mora timing source: ForcedAlign
alignment confidence: 0.88

mora #1 こ: 132.48 – 132.76
mora #2 こ: 132.76 – 133.12
mora #3 ろ: 133.12 – 133.55

GAME candidates:
F4  132.46 – 132.90 conf=...
G4  132.91 – 133.38 conf=...

matched notes:
F4, G4
primary: G4

binding status: Matched
```

对于空 badge：

```text
binding status: NoMusicalNote
```

或者：

```text
AlignmentLowConfidence
```

这样以后不再靠截图猜问题在哪一层。

## 11.4 可选显示振假名

本轮可以把 reading 放 Debug。

若实现成本低，可以加一个隐藏/开发选项显示 ruby；但不是 P0。

---

# 12. Phase 8：测试设计——这轮必须靠测试结束“玄学调参”

## 12.1 Synthetic note transcription tests

### Case A：vibrato

生成 1 s A4：

```text
69.0 MIDI + sinusoidal ±0.4 semitone
```

期望：

```text
GAME / canonical musical notes = [A4]
```

详细 badge 不得：

```text
G#4 → A4 → A#4 → A4 ...
```

### Case B：glide

```text
A4 stable 300 ms
linear glide A4→B4 250 ms
B4 stable 400 ms
```

期望 musical notes：

```text
A4 → B4
```

不得因为路径经过 A#4 自动生成 A#4 note。

### Case C：true melisma

```text
A4 stable
B4 stable
A4 stable
```

三个稳定目标，每段足够长。

期望：

```text
A4 → B4 → A4
```

若三个 note 属于同一 mora，则详细模式必须保留三个。

## 12.2 Japanese reading regression

必须至少钉死：

```text
愛       → あ・い
二人     → ふ・た・り
心       → こ・こ・ろ
嵐       → あ・ら・し
流れる   → な・が・れ・る
群れ     → む・れ
スーパー → ス・ー・パ・ー
きょう   → きょ・う
がっこう → が・っ・こ・う
```

同时测试：

```text
運命 + override(さだめ)
```

确保 override 真走进 alignment。

## 12.3 Screenshot-derived regression phrases

用户已经观察到的句子，加入文字和人工时间标记 fixture：

```text
流れる人の群れ
心を引き裂く嵐の中
知らない二人にもどるのね
愛が終わるのを見とどけている
```

不要把原歌曲音频提交；可以在本地 private benchmark 用原音频。

## 12.4 中文真转音 fixture

用户提到陶喆《就是爱你》“说你会永远陪着我，陪着我”中的“我”有明显转音。

不要把版权音频提交。

测试方式：

- 用户本地 private fixture；或
- 录制一个模拟“一字三音”的自有音频。

验收：

```text
同一个 token 可以稳定显示 N>1 GAME notes
```

## 12.5 Binder invariants

自动测试：

- note list 时间递增；
- note event `end > start`；
- mora time 单调；
- token time 单调；
- 正式绑定必须 `overlap > 0`；
- 不允许后 token 绑定到前一个已经结束很久的 note；
- 没证据时宁可 blank，不准 hallucinate；
- enhanced LRC 逐字时间不能被自动 alignment 覆盖。

---

# 13. 建议量化指标

不能只说“肉眼看起来更好了”。

## 13.1 Note transcription

私有人工标注若有：

- pitch accuracy（±50 cents / semitone exact）；
- onset MAE；
- offset MAE；
- note precision / recall / F1。

初始可用容差：

```text
onset tolerance: 80 ms
offset tolerance: 120 ms
pitch: nearest semitone exact
```

后续再严格。

## 13.2 Lyrics alignment

至少记录：

```text
mora onset MAE
mora end MAE
boundary median error
95th percentile error
alignment failure rate
```

目标建议：

- 中位边界误差 < 80 ms；
- 95% 尽量 < 200 ms；
- 不发生跨行跳跃。

## 13.3 最终 token quality

记录：

```text
blank token rate
obviously-wrong-note rate (人工抽查)
false multi-note rate
true melisma recall
```

用户目前最敏感的是：

> “假转音太多”和“明明唱了却空”。

所以这两个必须纳入 scoreboard。

---

# 14. A/B Benchmark 工具

建议加 CLI 或测试脚本：

```bash
pnpm benchmark:pitch --audio PRIVATE.wav --lrc PRIVATE.lrc
```

或 Rust/Python 脚本均可。

输出：

```text
Legacy FCPE Tracker:
  notes: ...
  blank tokens: ...
  multi-note tokens: ...

GAME:
  notes: ...
  blank tokens: ...
  multi-note tokens: ...

Old DP:
  timing ...

ForcedAlign:
  timing ...
```

最好输出 JSON + HTML/Markdown summary。

---

# 15. 代码目录建议

最终建议整理成：

```text
src-tauri/src/
  pitch/
    mod.rs
    fcpe.rs

  note_engine/
    mod.rs
    game.rs
    game_preprocess.rs
    game_decode.rs
    legacy_fcpe.rs

  lyrics/
    mod.rs
    lrc.rs
    display_token.rs
    alignment.rs
    binder.rs

  japanese/
    mod.rs
    reading.rs
    unidic.rs
    mora.rs
    overrides.rs
    phoneme.rs

  forced_align/
    mod.rs
    backend.rs
    external_helper.rs   # 若 prototype 需要

  models.rs
```

不要一次大搬家导致 diff 难审。

推荐：

1. 先加新模块；
2. 测试通过；
3. 再把旧 `lyrics.rs` 拆分；
4. 最后删 dead code。

---

# 16. 前后端 API 建议

## 16.1 Backend result

前端不要只拿：

```text
pitch_notes[]
primary_note
```

建议增加：

```ts
interface LyricTokenAnalysis {
  primaryNote?: MusicalNote;
  notes: MusicalNote[];
  bindingStatus: string;
  alignmentSource: string;
  alignmentConfidence?: number;
  reading?: string;
  moras?: MoraDebugInfo[];
}
```

Debug 字段可以 `serde(default)`，不影响旧项目。

## 16.2 详细模式禁止读 raw F0 labels

前端数据源明确：

```text
普通 badge       ← matched MusicalNoteEvent primary
详细 badge       ← matched MusicalNoteEvent[]
底部实时 MIDI    ← ContinuousPitchTrack
钢琴卷帘曲线      ← ContinuousPitchTrack
```

这是非常重要的 UI 语义约束。

---

# 17. 模型/许可证策略

## 17.1 GAME

GAME code：MIT。

官方支持 ONNX，v1.0.3 release 提供 ONNX 模型。

模型文件的具体附带许可仍应在集成时核对 release/model metadata，并在 README 列明来源、版本、hash。

推荐写：

```text
GAME model version: 1.0.3
opset: 17
sha256: ...
source: official GitHub Release
```

## 17.2 Lindera / UniDic

检查并保留对应 dictionary license / attribution。

词典数据不要假设与代码 crate license 完全相同。

## 17.3 pyshiro

GPLv3。

默认只作为 benchmark/oracle，不嵌入核心发行物。

## 17.4 MMS forced aligner

常见 300M forced-aligner 权重存在 CC-BY-NC 4.0 限制。

若采用：

- 标记为 optional non-commercial backend；
- download-on-demand；
- 不把模型许可证伪装成项目 MIT/Apache；
- 架构必须允许替换其他 aligner。

---

# 18. 性能和模型加载

## 18.1 不要每行重复创建 ONNX Session

GAME Session 和 forced aligner session 全局/任务生命周期缓存。

## 18.2 分段

整首 4 分钟歌可以：

- GAME 整首或 chunk inference；
- forced alignment 按 LRC line/window；
- 输出统一绝对时间。

chunk 拼接必须处理 overlap，不能产生重复 note。

## 18.3 Progress

前端至少显示：

```text
1/4 FCPE
2/4 GAME note transcription
3/4 Japanese reading
4/4 Lyrics forced alignment / binding
```

不要让用户以为卡死。

## 18.4 Cache

ProjectData 可以缓存：

- continuous pitch；
- GAME notes；
- readings；
- forced alignment；

当只修改歌词字体时不重算。

当修改 reading override 时，只从 alignment 下游重算。

---

# 19. 失败与 fallback 策略

完整 pipeline：

```text
GAME available?
  yes → GAME notes
  no  → LegacyFcpe notes + warning

Japanese line?
  yes → UniDic reading
  no  → existing language tokenization

Forced aligner available?
  yes → ForcedAlign
  no  → Mora-aware acoustic DP

Enhanced LRC token timing present?
  yes → always prefer explicit timing
```

UI 可显示小状态：

```text
Pitch: FCPE
Notes: GAME
Lyrics timing: ForcedAlign
Japanese: UniDic
```

便于排查。

---

# 20. 对旧 DP 的处理

不要直接删除 `dp_align_line`。

它作为 fallback 仍有价值。

但重命名/注释清楚：

```text
Acoustic heuristic aligner (fallback)
```

并修改权重输入：

```text
如果有真实 mora count → 使用真实 mora duration prior
如果无 reading → 才用 legacy token_weight
```

其输出不能声称“phoneme forced alignment”。

---

# 21. 对旧 NoteTracker 的处理

旧 tracker 同理：

```text
LegacyFcpeNoteTracker
```

不要再继续投入大量时间实现复杂 vibrato/glide 分类。

若新 GAME 路线达到目标：

- legacy 保留 fallback；
- 只修 crash / deterministic bug；
- 不再作为主算法开发。

原因是 musical note transcription 本身已经有更合适的专用模型。

---

# 22. 开发提交顺序（建议严格按此拆 commit）

## Commit 1 — Baseline + deterministic fixes

内容：

- benchmark fixture framework；
- Unicode small kana 修复；
- long mark mora 语义修复；
- binder overlap invariants；
- note 排序；
- debug status 基础字段。

验收：旧功能不退化，测试全绿。

## Commit 2 — Data model split

内容：

- ContinuousPitchTrack / MusicalNoteTrack；
- source enum；
- schema_version migration；
- 前端仍可使用 legacy source。

验收：UI 行为基本不变。

## Commit 3 — GAME Python reference + benchmark

内容：

- scripts/reference_game.py；
- private fixture tooling；
- GAME 输出统一 JSON；
- A/B report。

验收：证明 GAME 对真实歌曲 note event 明显优于 legacy。

## Commit 4 — Native GAME ONNX

内容：

- Rust GAME engine；
- official v1.0.3 ONNX pipeline；
- 与 Python golden 输出对比；
- fallback。

验收：同一 fixture notes 数量、边界、pitch 在允许误差内与官方 reference 一致。

## Commit 5 — UniDic real reading

内容：

- Lindera；
- reading spans；
- mora parser；
- overrides；
- debug reading。

验收：指定 regression 全过；运行中汉字不再默认 1.8。

## Commit 6 — Forced alignment prototype

内容：

- line-window forced alignment backend；
- JSON interface；
- confidence / source；
- fallback DP。

验收：日语 private fixture timing 显著优于 old DP。

## Commit 7 — Mora–Note matcher

内容：

- monotonic matcher；
- many-to-many；
- primary note；
- blank reason。

验收：不存在 phantom/cross-order note。

## Commit 8 — UI semantic cleanup

内容：

- 普通/详细/Debug 三档；
- data source labels；
- debug overlay；
- progress。

## Commit 9 — Final benchmark + docs

内容：

- scoreboard；
- model setup；
- licenses；
- architecture diagram；
- migration notes。

---

# 23. Definition of Done

本轮只有以下条件全部满足，才叫完成。

## Note engine

- [ ] 默认 musical note 来源已切到 GAME。
- [ ] FCPE 仍保持连续 F0 曲线。
- [ ] vibrato 不再因为半音 crossing 自动变成多个 badge。
- [ ] glide 中间经过的半音不会自动出现为 note。
- [ ] 真 melisma 能保留多 note。
- [ ] GAME 缺失时 legacy fallback 清晰可见。

## Japanese

- [ ] UniDic 真正运行。
- [ ] `愛→あい`、`二人→ふたり` 等真实 reading 可在 debug 中查看。
- [ ] mora 从 reading 生成，不从 surface 汉字直接猜。
- [ ] `ー` mora 正确。
- [ ] user reading override 真正影响 alignment。
- [ ] 汉字 1.8 只作为明确 degraded fallback。

## Alignment

- [ ] Forced alignment 至少有一个真实 backend 跑通。
- [ ] 每行有 alignment source + confidence。
- [ ] enhanced LRC 始终最高优先级。
- [ ] 失败可局部 fallback，不崩整个歌曲。

## Binder

- [ ] notes 单调排序。
- [ ] 正式绑定必须真实 overlap。
- [ ] many-to-many 支持。
- [ ] 不再出现 proximity-only phantom note。
- [ ] 空音有 reason code。

## UI

- [ ] 普通模式 = primary musical note。
- [ ] 详细模式 = true musical note events。
- [ ] 连续 F0 只用于曲线/实时数值。
- [ ] Debug 可追踪 reading → mora → timing → notes。

## Testing

- [ ] synthetic vibrato/glide/melisma tests。
- [ ] Japanese reading regression。
- [ ] binder invariants。
- [ ] project migration tests。
- [ ] benchmark report。

---

# 24. 必须避免的“伪解决方案”清单

不要：

1. 再把所有 F0 frame `round()` 后称为 musical note transcription。
2. 通过把 `switch_confirm_ms` 调得巨大来消除 vibrato。
3. 通过把阈值调巨大把真正短转音也全部吞掉。
4. 把 spectral flux peak 当成日语 mora boundary 真值。
5. 把每个汉字固定当 1~2 mora。
6. 把 `surface` 直接传给 mora parser 期待它认识汉字。
7. 把“定义 UniDic provider 接口”写成“已支持 UniDic”。
8. 把 `AlignmentSource::ForcedAlign` enum 写进去就宣布 forced alignment 完成。
9. 让 proximity-only note 变成正式 token note。
10. 让 HashMap 顺序决定 `A→B→C` 的显示顺序。
11. 用 ASR 识别出的文字替换用户 LRC。
12. 为了赶进度把 CC-BY-NC 模型权重打包后仍宣称整个发行物都是 MIT/Apache。
13. 一次性删除 legacy pipeline，导致新模型加载失败时软件不可用。
14. 为了“看起来更准”硬编码用户截图中的具体歌词或音符。

---

# 25. 开发 Agent 可自行决定的事项（不要回来问用户）

以下问题请工程上自行选择合理方案：

- Rust 文件如何拆模块；
- matcher 用 greedy 还是 DP；
- debug overlay 用 popup/sidebar；
- internal ID 用 usize/u32；
- cache 文件放 project JSON 还是 sidecar；
- GAME model manager 的具体 class 名；
- CPU inference thread 数；
- logging crate；
- tests 放 unit 还是 integration；
- UniDic pin 6.x 还是 5.3.x（以能稳定构建为准）；
- Python forced align prototype 的 CLI/JSON 细节；
- UI wording 的小调整。

只在以下情形才需要用户决策：

1. 必须下载数 GB 模型且没有替代；
2. 模型许可证会改变用户公开发布/商用能力；
3. 需要用户提供私有版权音频做最终 benchmark；
4. 需要把项目从纯 Rust/Tauri 改成强制依赖 Python runtime；
5. 发现用户最新 HEAD 与本任务书描述的核心架构完全不同。

其他不要反复提问。

---

# 26. 推荐参考项目/资料

## GAME

- `https://github.com/openvpi/GAME`
- `https://github.com/openvpi/GAME/blob/main/ALGORITHMS.md`
- Releases v1.0.3：官方 ONNX opset 17 models，包含 improved V/UV 与 word-note alignment。

## OpenVPI dataset-tools

- `https://github.com/openvpi/dataset-tools`
- 其中 `GameInfer` 已存在 GAME 多 ONNX 模型 pipeline，可作为 C++/ONNX 工程参考。

## Lindera / UniDic

- `https://docs.rs/lindera/latest/lindera/`
- `https://docs.rs/lindera-unidic/latest/`

## Japanese singing forced alignment benchmark

- `https://github.com/wavtechyukky/pyshiro`
- GPLv3，仅建议 benchmark/oracle 或明确隔离使用。

## 类似 end-to-end 架构参考

- `https://github.com/Lanternko/karaoke-jp`

值得参考的不是它的 UI，而是它把：

```text
morphological reading
CTC forced alignment
musical note extraction
mora→note monotonic matching
```

分成独立 stage 的方法论。

注意不要未经检查直接复制其模型、私有 checkpoint 或许可证不兼容资产。

---

# 27. 最终期望的用户体验

用户导入歌曲 + LRC 后：

```text
Analyzing pitch (FCPE)...
Transcribing musical notes (GAME)...
Parsing Japanese pronunciation (UniDic)...
Aligning lyrics to vocals...
Matching notes to lyrics...
Done.
```

播放时：

### 普通模式

```text
流  れ  る  人  の  群  れ
A#3 F4 G4 A4 C#4 F4 E4
```

每个字/显示 token 只有代表音。

### 真转音

```text
我
G4 → A4 → G4
```

只有确实存在多个 musical note events 才显示箭头。

### Vibrato

下面的青色连续 F0 曲线仍然会自然波动，但上方 badge：

```text
A4
```

不会变成：

```text
G#4 → A4 → A#4 → A4 → G#4
```

### 日语

程序内部真正知道：

```text
心 = こころ = こ・こ・ろ
二人 = ふたり = ふ・た・り
愛 = あい = あ・い
```

然后根据声学 alignment 决定它们的时间，而不是继续用“汉字大约 1.8 拍”猜。

---

# 28. 一句话总结本轮方向

本轮不要再试图“把旧算法调得更聪明”。

应该把问题拆回它本来的四个科学/工程任务：

```text
FCPE        → 连续 F0
GAME        → 音乐音符
UniDic + FA → 日语发音及时间
Matcher     → 歌词与音符对应
```

当这四层各自有可信真值后，所谓“日语歌词每个字到底该标什么音”才会从一个巨大的启发式问题，变成一个可测量、可调试、可逐层验证的问题。

**完成前请以 benchmark 和真实数据流为准，不以 TODO、enum、接口、截图或“感觉应该好了”为准。**
