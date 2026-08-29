# Pitch Analyzer：日语歌词 × 音高对齐重构任务书

> 面向仓库：`https://github.com/hydrogen1222/pitch-analyzer`  
> 审查基线：2026-08-29 `main`  
> 目标：解决普通 LRC 日语歌词在「汉字读音长度不定」「熟字训/特殊读法」「假名莫拉结构」「一个歌词单位对应多个音高（melisma/转音）」「无声化/闭塞导致无 F0」条件下的系统性错位，并保持 enhanced LRC、现有 NoteEvent 稳定化和旧工程兼容。

---

## 0. 最终结论：不要继续把 `LyricToken` 当作“语言单位 = 时间单位 = 音符单位”

当前最根本的问题不是 DP 权重还不够精巧，而是数据抽象把三种不同对象压成了一个：

```text
显示字/显示 token  <->  日语发音单位  <->  音符 NoteEvent
```

真实关系却是 many-to-many：

```text
LRC 原文字形
   │
   ▼
Display grapheme/span          “位” / “微笑” / “冷たく”
   │  1..N / N..1
   ▼
ReadingSpan                    くらい / ほほえみ / つめたく
   │
   ▼
MoraUnit                       く・ら・い / ほ・ほ・え・み / つ・め・た・く
   │
   ▼
PhonemeUnit                    k u / r a / i / ...
   │        由声学 forced alignment 决定时间
   ▼
[start, end, confidence]
   │
   ├──── 0..N ────► NoteEvent   （一个 mora 可转多个音）
   │
   └──── N..1 ────► NoteEvent   （多个 mora 也可能共享一个持续音）
```

因此，**Lindera + UniDic 只解决文本侧“唱的是什么音节/莫拉”问题；forced alignment 才解决音频侧“这些音素具体落在什么时候”问题。两者缺一不可。**

应当把 `primary_note` 降级为“紧凑 UI 的代表音”，而不是数据模型里的唯一真值。后端已经能够保存一个 token 的多个 `pitch_notes`，这一点应该保留并扩展。

---

# 1. 当前源码中已经确认的 P0 问题

## 1.1 `is_japanese_attach_char()` 有真实 Unicode code point 错误

当前 `src-tauri/src/lyrics.rs` 写成：

```rust
'\u{3083}' // ゃ
'\u{3084}' // ゅ   // 实际是 や
'\u{3085}' // ょ   // 实际是 ゅ
...
'\u{30E3}' // ャ
'\u{30E4}' // ュ   // 实际是 ヤ
'\u{30E5}' // ョ   // 实际是 ュ
```

正确关键点：

```text
Hiragana: U+3083 ゃ, U+3085 ゅ, U+3087 ょ
Katakana: U+30E3 ャ, U+30E5 ュ, U+30E7 ョ
```

所以目前代码会：

- 错把普通大小的 `や / ヤ` 当成附着小假名；
- 漏掉真正的小 `ょ / ョ`；
- 现有测试没有覆盖这个错误。

### P0 修复

不要只改注释，直接修 code point，并新增测试：

```rust
assert_eq!(tokenize("きょ"), vec!["きょ"]);
assert_eq!(tokenize("キャ"), vec!["キャ"]);
assert_eq!(tokenize("キョ"), vec!["キョ"]);
assert_eq!(tokenize("や"), vec!["や"]);
assert_eq!(tokenize("ヤ"), vec!["ヤ"]);
```

---

## 1.2 `ー` 被当作“不成拍”是概念错误

当前代码把小写拗音假名和长音符 `ー` 都塞进 `is_japanese_attach_char()`，并让 `token_weight()` 对它们增加 0；测试甚至明确断言：

```text
きゃ = 1 拍
ラー = 1 拍
りー = 1.0
```

这混淆了“显示上附着”和“莫拉计数”。

正确抽象应是：

- `きゃ`：1 mora；
- `っ`：1 mora；
- `ん`：1 mora；
- `ー`：长音第二部分，**另占 1 mora**；
- 因此 `スーパー` 的 mora 应为 `ス・ー・パ・ー = 4`。

可以为了 UI 继续把 `ー` 和前面的字显示在一个 `DisplayToken`，但绝不能因此把它从 `mora_count` 里删除。

### P0 修复

立即把“显示合并规则”和“莫拉解析规则”拆开：

```rust
fn group_display_tokens(...) -> Vec<DisplayToken>
fn parse_kana_moras(...) -> Vec<MoraUnit>
```

不要再让一个 `is_japanese_attach_char()` 同时决定两者。

必须加入：

```text
きゃ       -> 1 mora
がっこう   -> が / っ / こ / う = 4
スーパー   -> ス / ー / パ / ー = 4
ファ       -> 1 mora（外来音组合）
ティ       -> 1 mora（外来音组合）
```

`ファ / ティ / シェ / チェ / ウィ ...` 不适合用“凡小假名都机械附着”的代码解决，建议做一个明确的 kana→mora 状态机/组合表。

---

## 1.3 标点目前可能偷偷获得时长权重

`tokenize()` 把标点追加到上一个 token，例如 `た。`；但 `token_weight()` 对任何既非汉字、也非 attach char 的字符都 `+1.0`。

于是：

```text
た。 可能被算成 2 拍
```

这是显示层污染对齐层的典型例子。

### P0 修复

标点必须是 display-only metadata，或者至少在时间权重中严格为 0：

```rust
fn is_alignment_punctuation(c: char) -> bool;
```

推荐根本方案：`DisplayToken.text` 可以含标点，但 `MoraUnit` 根本不产生标点节点。

---

## 1.4 当前 DP 的“一个显示 token = 一个声学 segment”是结构性错误

现在 `dp_align_line()`：

1. `weights` 直接按 `line.tokens` 建；
2. DP 固定切出 `N_tokens - 1` 个边界；
3. 汉字用 1.8 拍近似；
4. F0 rounded MIDI 一变化就给 boundary `+1.5`。

这意味着它预设：

```text
一个屏幕上的字/token == 一个发音时段
```

但截图里的：

```text
悲 し い 位 に 見 つ め て た
```

其中 `位` 在该语境下很可能是 `くらい`，即 **一个显示汉字覆盖 3 个 mora**。现有 DP 的状态空间根本表达不了这件事，`1.8` 改成 `2.1`、`2.5` 都只是继续猜。

另一个更危险的问题：**换音不是可靠的歌词边界。** 在 singing melisma 中，同一个元音/莫拉内部发生 C4→D4→E4 完全正常；当前 DP 却把 rounded MIDI 改变当成强正向“切字”证据。也就是说，转音越明显，字符边界反而越可能被主动拉错。

### P0/P2 原则

歌词时间边界和音符边界必须分家：

```text
声学/音素边界：谱包络、辅音 onset、能量、静音、强制对齐模型
音高边界：NoteTracker / FCPE F0
```

F0 change 最多只能作为极弱辅助特征；生产方案里甚至可以完全不进入歌词 phoneme boundary 打分。

---

## 1.5 DP 把搜索窗口裁到 voiced F0 区域，对日语并不稳健

当前 `dp_align_line()` 先找首尾 voiced frame，再只保留 voiced 范围 ±80 ms。

对一般唱歌这能去掉行首尾空白，但对日语会有一个原则性风险：**语言单位可以真实存在，但没有独立可测 F0。** 例如清音环境下的母音无声化、促音闭塞等；这类区域在谱上仍有语言边界信息，却不能要求 FCPE 给出稳定 voiced pitch。

生产 forced aligner 应使用声谱/音素 emission，而不是把 F0 presence 当作硬搜索包络。

---

## 1.6 当前 pitch binding 可能把轻微边界误差放大成“空白音高”

现在 `bind_pitch_to_tokens()`：

```text
token 两端先各裁 edge_ignore_ms（默认 20 ms）
      ↓
note_events_in_range() 又要求 overlap >= edge_ignore_ms
```

所以一个本来只错几十毫秒的 token boundary，可能经过“两端裁剪 + 最小 overlap”后彻底匹配不到 NoteEvent，最终 UI 就出现没有音高 badge 的字符。

这与截图中 `く / 位 / て` 之类的空白现象**机制上完全吻合**，但没有该歌曲原始音频和中间调试数据，不能断言截图里的每一个空白都是由它造成；某些日语音段也确实可能没有独立 F0。

### P0 修复

不要 binary gate，改成软评分：

```rust
struct NoteBinding {
    note_event_index: usize,
    overlap_ms: f32,
    overlap_ratio_token: f32,
    overlap_ratio_note: f32,
    score: f32,
}
```

例如：

```text
score = 0.55 * overlap_ratio_token
      + 0.30 * overlap_ratio_note
      + 0.15 * confidence
```

允许一个很小的时间 tolerance（例如 20–30 ms），但只用于候选生成，不要“凭空继承”相邻音。

还要明确区分：

```rust
enum UnpitchedReason {
    NoVoicing,
    ClosureOrDevoicing,
    LowPitchConfidence,
    NoOverlappingNote,
    AlignmentMissing,
}
```

这样 UI 上的“空白”才知道是物理上无音高还是算法丢了。

---

## 1.7 转音其实已经被后端保存了，问题主要在“切窗”和“显示”

现有 `test_melisma_token_binding` 已经证明：一个 token 内若先 C4 后 E4，`pitch_notes` 可以保留两个，`primary_note` 再用“持续时间 × confidence”挑 E4。

因此不要推倒 NoteEvent/`select_primary_note()`；这部分方向是对的。

真正要改的是：

```text
错误 token 时间窗
      +
display token / mora / note 三层未分开
      +
前端只显示 primary_note
```

当前 `karaoke_display.ts` 每个 token 只造一个 `noteBox`，取：

```ts
const primary = token.primary_note ?? this.bestNote(token);
```

所以数据里即便有 `C4 + E4`，界面也只看见一个。

### UI 建议

保持默认“紧凑模式”只显示主音，同时新增可选“详细音高模式”：

```text
冷       た       く       微       笑
A#3      F4      G4→A4    G4       A#4
```

若一个单位含 3+ notes，可以显示：

```text
C4→D4→E4
```

或者一个主 badge + 较小副 badge，避免默认 UI 变得过重。

---

# 2. 新的日语文本层：Lindera + UniDic，但不要“每个汉字强拆一个读音”

## 2.1 推荐的模块边界

新增：

```text
src-tauri/src/japanese/
├── mod.rs
├── reading.rs       # Lindera/UniDic provider + override
├── normalize.rs     # 假名规范化
├── mora.rs          # kana -> mora
└── phoneme.rs       # mora -> phoneme sequence
```

定义 provider trait，避免将整个工程绑死在某一词典：

```rust
pub trait JapaneseReadingProvider: Send + Sync {
    fn analyze(&self, text: &str) -> anyhow::Result<Vec<ReadingSpan>>;
}
```

实现：

```text
LinderaUnidicProvider   # 正式模式
KanaOnlyProvider        # 纯假名快速路径/回退
HeuristicProvider       # 最后兜底，不作为推荐结果
```

---

## 2.2 `ReadingSpan` 必须保留原文 byte span

Lindera token 本身有 `surface / byte_start / byte_end / details`，非常适合做原文映射。

建议：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingSpan {
    pub surface: String,
    pub reading: String,       // canonical kana
    pub pronunciation: String, // 若字典提供
    pub byte_start: usize,
    pub byte_end: usize,

    pub display_start: usize,
    pub display_end: usize,    // 允许覆盖多个显示 grapheme

    pub mora_start: usize,
    pub mora_end: usize,

    pub source: ReadingSource,
    pub confidence: f32,
}
```

### 关键限制

**不要承诺每个汉字都能唯一拆成一段假名。**

形态素词典自然给的是 lexical/morphological span，而不是“这个汉字单独一定读这几个 mora”。`冷たく` 这种有送り仮名的词往往还能合理拆；但熟字训、当て字、特殊歌曲读法中，逐汉字拆分可能在语言学上就没有唯一答案。

因此：

```text
ReadingSpan 可以覆盖多个 display grapheme
```

UI 可以在播放时同时高亮这一 span；如果产品一定要逐字高亮，可做“presentation-only 推测拆分”，但必须标记低 confidence，不能把它当 ground truth 再喂回声学算法。

---

## 2.3 不要把 romaji 作为中间真值

内部建议：

```text
surface -> kana reading -> mora -> phoneme
```

不要：

```text
surface -> romaji -> 再猜 phoneme
```

罗马字对长音、促音、拗音、音系同化等都会额外引入表示歧义。Kana 是日语文本侧更自然的 canonical bridge，最终 forced aligner 再消费 phoneme sequence。

---

## 2.4 必须支持歌曲特殊读音 override

即使 UniDic 很强，歌曲仍存在：

```text
歌词写「運命」，实际唱「さだめ」
```

这种情况下词典默认 reading 再准确也会错。

因此 P1 就应把 override 接口设计进去：

```rust
struct ReadingOverride {
    line_id: ...,
    byte_start: usize,
    byte_end: usize,
    reading: String,
}
```

优先级：

```text
用户/歌词显式 reading override
    > ruby/furigana metadata（以后若支持）
    > UniDic
    > kana-only
    > heuristic fallback
```

以后若要做“自动候选读音 + 声学择优”，也可以在这个接口上扩展，不必重构模型。

---

# 3. Mora 不是最终声学对齐单位：生产版应做到 phoneme forced alignment

## 3.1 为什么先 mora、最后 phoneme

Mora 很适合：

- 日语节奏；
- 时间先验；
- UI 聚合；
- 汉字多读音映射。

但声学上真正可定位的往往是辅音/母音变化。比如 `く` 的 /k/ onset 与 /u/ vowel 可能有明显不同，甚至 /u/ 无声化后 F0 不存在。

因此最稳结构：

```text
ReadingSpan
  -> MoraUnit
     -> phonemes
        -> forced alignment 得到 phoneme [start,end]
     <- 聚合成 mora [start,end]
  <- 聚合/映射回显示 span
```

建议：

```rust
pub struct MoraUnit {
    pub kana: String,
    pub phonemes: Vec<String>,
    pub reading_span_id: usize,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    pub confidence: f32,
    pub note_bindings: Vec<NoteBinding>,
}
```

---

## 3.2 P2：先做“纯 Rust 声谱 DP”作为过渡方案

在引入新的 forced-alignment ONNX 模型前，可以先大幅提升当前 DP：

### 复用已有 `mel.rs`

仓库已经有：

```text
16 kHz
hop = 160 samples = 10 ms
128-bin log-mel
```

不用再写一套声谱提取。

### 新 DP 输入

把：

```text
line.tokens
```

改成：

```text
mora sequence（最好内部进一步知道 phoneme class）
```

边界特征改为：

```text
spectral flux / log-mel frame distance
energy onset/valley
voiced↔unvoiced（soft）
辅音/母音 duration prior
LRC line anchor
```

F0 note transition 权重应降到接近 0，甚至移除。

### P2 的定位

这个版本仍然不是“最终精确 forced aligner”，因此结果必须带：

```rust
AlignmentSource::MoraDp
alignment_confidence
```

而不是把它当成 ground truth。

---

## 3.3 P3：生产方案——已知歌词 phoneme sequence 下的 constrained forced alignment

Pitch Analyzer 并不需要完整 ASR，因为文本已经知道了。最适合的是：

```text
audio -> phoneme emission probabilities
known phoneme sequence -> constrained CTC/Viterbi or HMM/HSMM
-> precise phoneme intervals
```

然后再：

```text
phoneme -> mora -> ReadingSpan/DisplayToken -> NoteEvent binding
```

### 模型部署方向

项目已经依赖 `ort`，FCPE 也是 ONNX 路线，所以新 aligner 最理想仍是 ONNX：

```text
Japanese/singing phoneme model.onnx
        +
现有 ort runtime
        +
Rust constrained decoder
```

无需再带 Python runtime。

### 可作为研究/benchmark 的现成项目

**pyshiro**：明确是 Japanese singing voice forced alignment，采用 HMM/HSMM 两阶段对齐，适合作为“这个问题怎样才算正确处理”的 oracle / baseline；但项目许可证是 GPLv3，当前 Pitch Analyzer 是 MIT OR Apache-2.0，所以不要直接复制/静态整合代码，除非你主动接受许可证后果。

**SOFA**：Singing-Oriented Forced Aligner，MIT，架构上很值得借鉴，而且支持 custom G2P；但其仓库默认示例/字典并不是现成的日语 turnkey 模型，所以更适合借鉴架构、评估方式和 ONNX 方向，而不是直接说“拿来就能跑日语”。

**narabas**：MIT，日本语 phoneme forced alignment，使用日语 phoneme sequence，可作为 speech-domain Japanese baseline；它不是 singing-specific，因此不要把它直接等同于歌曲最终模型。

### 推荐策略

开发顺序：

```text
先用 pyshiro/narabas 离线做 benchmark
        ↓
证明新架构的 gold boundary 能显著优于当前 DP
        ↓
再决定训练/转换哪一个适合 shipping 的 ONNX phoneme aligner
```

不要一上来为了“全 Rust”自己造训练体系；先把数据抽象和评测做对。

---

# 4. 数据模型重构建议

建议不要删除旧字段，而是在保持旧工程读取的前提下扩展。

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlignmentSource {
    EnhancedLrc,
    ForcedAlign,
    MoraDp,
    WeightedFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteBinding {
    pub note_event_index: usize,
    pub overlap_ms: f32,
    pub overlap_ratio_token: f32,
    pub overlap_ratio_note: f32,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingSpan {
    pub surface: String,
    pub reading: String,
    pub pronunciation: String,
    pub byte_start: usize,
    pub byte_end: usize,
    pub display_start: usize,
    pub display_end: usize,
    pub mora_start: usize,
    pub mora_end: usize,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoraUnit {
    pub kana: String,
    pub phonemes: Vec<String>,
    pub reading_span_id: usize,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    pub confidence: f32,
    #[serde(default)]
    pub note_bindings: Vec<NoteBinding>,
}
```

现有 `LyricToken` 继续作为 **display view**：

```rust
pub struct LyricToken {
    pub text: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
    #[serde(default)]
    pub pitch_notes: Vec<PitchNote>,
    #[serde(default)]
    pub primary_note: Option<PitchNote>,

    // 新增
    #[serde(default)]
    pub byte_start: usize,
    #[serde(default)]
    pub byte_end: usize,
    #[serde(default)]
    pub reading_span_ids: Vec<usize>,
    #[serde(default)]
    pub alignment_confidence: f32,
    #[serde(default)]
    pub alignment_source: Option<AlignmentSource>,
}
```

`LyricLine` 新增：

```rust
#[serde(default)]
pub reading_spans: Vec<ReadingSpan>,
#[serde(default)]
pub moras: Vec<MoraUnit>,
```

如工程文件长期保存，建议同时增加：

```text
schema_version
```

并提供一次明确的 migration，而不是以后靠越来越多隐含 `serde(default)` 猜版本。

---

# 5. Enhanced LRC 要继续最高优先级，但映射方式必须重写

现在 `apply_word_timings()` 会：

```text
把 enhanced LRC chunk tokenize
若 items.len() != line.tokens.len()
直接整段放弃逐字时间
```

一旦引入 UniDic、ReadingSpan、display token 重构，这个等长假设会更加脆弱。

正确做法：

```text
Enhanced LRC chunk
   -> 原文 byte/grapheme span
   -> span anchor time
```

再由 `ReadingSpan/MoraUnit` 继承或在锚点之间插值。

也就是说 enhanced LRC 的 authoritative 信息应绑定**原始文字 span**，而不是绑定某一次 tokenizer 恰好产生的 token index。

推荐优先级仍保持：

```text
Enhanced LRC explicit timing
    > forced alignment
    > acoustic mora DP
    > weighted fallback
```

---

# 6. 两张截图如何解释

## 截图 A：`冷 た く 微 笑 ん だ`

`く` 没有音高 badge，不能仅凭截图认定 FCPE 漏检。至少存在三种可能：

```text
A. DP 把“く”的时间窗切偏；
B. 时间窗尚可，但 edge_ignore + minimum overlap 把 NoteEvent 过滤掉；
C. 该音段本身弱发声/无声化，没有独立稳定 F0。
```

新架构应通过 `alignment_confidence + UnpitchedReason` 区分，而不是都渲染为空白然后让用户猜。

## 截图 B：`悲 し い 位 に 見 つ め て た`

`位` 是最有价值的反例之一。

如果这里是 `くらい`，它展示为一个汉字，却是三个 mora：

```text
位
└── くらい
    ├── く
    ├── ら
    └── い
```

当前 `汉字 = 1.8` 只是在“这个 token 应该更长一点”层面修补，根本没有表达 3 个内部发音单位，因此无法稳健决定 `位` 的起止，更无法正确判断其内部是否跨 1 个、2 个或更多 NoteEvent。

这正是必须引入 `ReadingSpan -> MoraUnit -> NoteBinding` 的原因。

---

# 7. 文件级修改清单

| 文件 | 修改内容 |
|---|---|
| `src-tauri/src/lyrics.rs` | P0 Unicode 修复；标点不参与时间权重；把旧 `token_weight` 降为 fallback；enhanced LRC 改为 source-span anchor；逐步废弃 display-token DP |
| `src-tauri/src/models.rs` | 新增 `AlignmentSource`、`ReadingSpan`、`MoraUnit`、`NoteBinding`、confidence/source/reason 字段；保持 serde 兼容 |
| `src-tauri/src/japanese/mod.rs` | 新模块入口 |
| `src-tauri/src/japanese/reading.rs` | `JapaneseReadingProvider` + Lindera/UniDic + override |
| `src-tauri/src/japanese/mora.rs` | 正确的 kana→mora parser；长音、促音、撥音、拗音、外来音组合 |
| `src-tauri/src/japanese/phoneme.rs` | mora→phoneme，为 forced alignment 做统一输入 |
| `src-tauri/src/mel.rs` | 复用现有 log-mel；必要时增加 spectral flux/帧距离工具，不要复制 FFT 管线 |
| `src-tauri/src/note_tracker.rs` | 尽量不动核心稳定化；仅提供更易于 overlap binding 的事件接口 |
| `src/karaoke_display.ts` | 默认 primary badge 保持；详细模式渲染全部 `pitch_notes`；支持 ReadingSpan/group highlight；显示低 confidence/真正 unpitched 的区分 |
| `src/main.ts` | 保持当前 playback clock 同步；只改为消费新的 line/token timing 输出，不在前端重新估计时间 |
| `src-tauri/tests/lyrics_test.rs` | 删除/改写错误的 `ラー=1拍` 等测试；新增日语 mora、标点、Unicode、span mapping、melisma regression |
| `src-tauri/tests/real_song_test.rs` | 从“结构不崩”升级为 gold boundary accuracy + note-binding accuracy |

---

# 8. 测试：现有 test suite 为什么不足

当前 real-song test 主要验证：

```text
能 parse
时间单调
没有越界
歌词会推进
pitch binding 有一定 coverage
```

这些只能证明“程序没有炸”，不能证明“日语字/莫拉真的对齐”。一个系统性提前 200 ms 的对齐也可能全部通过。

必须新增小规模人工 gold corpus。

## 8.1 Gold corpus 最低建议

做 20–50 句，不需要整首歌人工标几小时。刻意包含：

```text
纯假名普通句
汉字 + 送り仮名
一个汉字多 mora（例如 位→くらい）
多汉字一个 ReadingSpan / 熟字训
ゃゅょ / ャュョ
ー
っ
ん
ファ / ティ / シェ 等外来音组合
清音母音无声化
一个 mora 两个以上 NoteEvent（melisma）
多个 mora 共用一个持续 NoteEvent
歌曲特殊读法 override
标点
Enhanced LRC
```

人工标注最好到 phoneme boundary；实在太费事，第一版至少标 mora boundary + note binding truth。

## 8.2 指标

建议 CI 输出：

```text
Boundary MAE / median absolute error
Boundary accuracy @ ±20 / ±50 / ±100 ms
ReadingSpan reading accuracy
Mora sequence exact match rate
Note-binding precision / recall / F1
Melisma note-set accuracy
Spurious blank rate
AlignmentSource fallback rate
Low-confidence rate
```

“blank rate”必须排除 gold 标记为 genuinely unpitched 的单位，否则会逼算法伪造音高。

---

# 9. 实施优先级

## P0 — 先把现有错误修掉，1 个 PR 内完成

完成：Unicode 小假名 code point、`ー` mora 语义、标点零时长、pitch overlap 软绑定、debug reason/confidence、enhanced LRC span 化基础。

这一步不引 UniDic 也值得立刻做，因为它修的是确定性 bug。

## P1 — 文本层正式重构

接 Lindera + UniDic，建立：

```text
source span -> reading -> mora -> phoneme
```

同时支持 reading override。此时现有汉字 1.8 仅保留为词典不可用时的 fallback。

## P2 — Mora/phoneme aware acoustic DP

复用 `mel.rs`，先获得比当前 DP 明显更稳的无模型 fallback。彻底弱化/移除 F0 change 作为歌词边界证据。

## P3 — Forced alignment ONNX

在 gold corpus 上先用 pyshiro/narabas/SOFA 思路做 benchmark，然后选一个 license 与部署都合适的 phoneme emission + constrained decoder 方案，最终放进现有 `ort` 路线。

## P4 — UI / 导出

默认主音不变；详细模式显示 `pitch_notes` 全序列，并把 ReadingSpan / mora timing 暴露给字幕导出，为以后自动生成“逐字音高测量视频”提供稳定数据。

---

# 10. 依赖、体积和许可证注意事项

当前 `Cargo.toml` 尚未依赖任何日语形态素分析器，因此 P1 是新增依赖。

Lindera 本身是 Rust 原生、MIT，且支持外部字典/embedded dictionary，方向非常适合 Tauri。但不要现在就把“UniDic 只增加约 50 MB”写进设计承诺：当前 NINJAL 官方 2025-12 contemporary written UniDic 的“轻量解析版”压缩包标为约 **664 MB**，full package 约 **2.8 GB**。Lindera 自己预编译后的具体 UniDic asset/安装体积可能不同，必须实际测量目标包，不能按官方原始包大小或想象值硬推。

推荐 packaging：

```text
主程序
  + optional/external Japanese dictionary resource
```

而不是默认全塞进主 exe。

可以考虑：

```text
安装包可选 Japanese Accuracy Pack
首次使用日语对齐时提示安装
资源缓存到 app data
版本固定 + hash 校验
```

如果确实坚持 all-in-one，再 benchmark Lindera `embed-unidic` 的 release 体积、启动内存和 mmap 行为。

UniDic 解析用版本是 GPL/LGPL/modified BSD 三许可证可选；若采用 BSD 路线，需要按官方 FAQ 带相应 BSD/AUTHORS 信息。发布前务必把第三方 NOTICE/credits 一起设计。

---

# 11. 开发 Agent 的硬性约束（不要走回头路）

1. **禁止继续把“汉字平均 1.8 mora”调参当成日语正式解决方案。** 只能保留 fallback。
2. **禁止把 display token 数量固定成 acoustic segment 数量。**
3. **禁止把 F0 换音当作强歌词边界。** 转音恰恰证明二者不是同一件事。
4. **禁止强制每个汉字唯一映射一段读音。** `ReadingSpan` 必须允许跨多个 grapheme。
5. **禁止强制每个歌词单位必须有音高。** 要区分 genuine unpitched 与 alignment failure。
6. **禁止丢弃 `pitch_notes` 只保留 `primary_note`。** `primary_note` 仅是显示/摘要字段。
7. **禁止因新 tokenizer 数量不同就丢掉 enhanced LRC。** 必须用原文 span 对锚点。
8. **禁止为了日语再复制一套音频特征提取。** 优先复用 `mel.rs`。
9. **禁止直接把 GPLv3 pyshiro 代码并入 MIT/Apache 主程序。** 可用于研究 baseline/离线评测。
10. **所有新模型/字典必须有版本、hash、license metadata，并支持缺失时 fallback。**

---

# 12. 验收标准

该重构不是“代码能编译”就算完成。至少满足：

```text
[Correctness]
- や/ヤ 不再错误附着，ょ/ョ 正确处理
- きゃ=1 mora，スーパー=4 mora，がっこう=4 mora
- 标点不增加声学时长
- 位→くらい 这类 1 glyph -> N mora 可显式表示
- 一个 mora/token 可以绑定 N 个 NoteEvent
- N mora 可以引用同一持续 NoteEvent
- genuine unpitched 与 missing alignment 有不同状态

[Timing]
- gold corpus 边界误差相对当前 main 显著下降
- 至少报告 @50 ms / @100 ms 准确率，而不是只看 coverage
- enhanced LRC 在任何重新分词后仍保持其显式时间锚点

[Regression]
- NoteEvent 稳定化/八度误判抑制不退化
- 英文/中文现有歌词加载不退化
- 旧工程 JSON 可读取
- 无 UniDic/无 aligner model 时仍能 fallback

[UI]
- 默认模式仍简洁显示 primary_note
- 详细模式能看见 C4→D4→E4 等 melisma
- 低 confidence / genuine unpitched 有可解释状态，不再统一“空白”
```

---

# 13. 推荐的第一批 Commit 拆法

```text
Commit 1: fix Japanese kana/mora correctness
  - Unicode codepoint
  - chōon mora semantics
  - punctuation zero-weight
  - tests

Commit 2: introduce reading/mora model without changing UI
  - ReadingSpan / MoraUnit / AlignmentSource
  - serde compatibility
  - kana-only provider

Commit 3: integrate Lindera + UniDic provider
  - byte-span mapping
  - reading override
  - dictionary resource abstraction

Commit 4: replace token DP with mora-aware acoustic DP
  - reuse mel.rs
  - remove F0-change strong boundary reward
  - confidence/source

Commit 5: soft NoteEvent many-to-many binding
  - overlap scores
  - unpitched reason
  - regression tests

Commit 6: frontend detailed melisma rendering + span highlight

Commit 7: gold corpus + quantitative alignment benchmark

Commit 8+: ONNX phoneme forced aligner, only after benchmark proves need/benefit
```

这样可以避免“一次性大爆炸重写后不知道究竟是哪一层改善/退化”。

---

# 14. 参考项目/资料

- Pitch Analyzer: https://github.com/hydrogen1222/pitch-analyzer
- Lindera: https://github.com/lindera/lindera
- Lindera Rust docs: https://docs.rs/lindera/
- UniDic (NINJAL): https://clrd.ninjal.ac.jp/unidic/
- NINJAL segment/phoneme manual: https://www2.ninjal.ac.jp/kikuo/segment.pdf
- TUFS Japanese pronunciation modules: https://www.coelang.tufs.ac.jp/mt/ja/pmod/
- pyshiro (Japanese singing voice forced alignment): https://github.com/wavtechyukky/pyshiro
- SOFA (Singing-Oriented Forced Aligner): https://github.com/qiuqiao/SOFA
- narabas (Japanese phoneme forced alignment): https://github.com/darashi/narabas

---

## 给开发 Agent 的一句话总纲

> **不要再优化“把显示字切成几段”这一层；先把日语原文解析成 reading span → mora → phoneme，再做声学 forced alignment，最后把 NoteEvent many-to-many 地绑定回来。Lindera+UniDic 解决“唱的字怎么读”，forced alignment 解决“什么时候唱到”，NoteTracker 解决“唱的是什么音高”，三者必须解耦。**
