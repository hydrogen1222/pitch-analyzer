# Pitch Analyzer：NoteTracker v2 × 日语歌词对齐二阶段重构任务书

> 面向开发 Agent 的一次性实施文档  
> 仓库：`https://github.com/hydrogen1222/pitch-analyzer`  
> 审查基线：2026-08-30 `main` 分支当前源码  
> 目标：解决“详细音高几乎每个字都出现大量假转音”“紧凑模式漏音/错音”“日语汉字读音无法可靠映射到莫拉”“歌词时间与 NoteEvent 绑定仍可能串音”等问题。

---

## 0. 先读这一段：本轮不是继续调参数，而是修正两条流水线的定义

本轮开发不要再围绕 `switch_confirm_ms=60` 改成 80/100/150 ms、`汉字权重 1.8` 改成 2.0、`30 ms tolerance` 改成 20 ms 这类局部数字来回试。

当前症状来自两条本应独立、现在仍部分混杂的流水线：

```text
A. 音高侧
FCPE 连续 F0
  -> Clean Pitch Track
  -> [当前问题核心] NoteEvent 离散化
  -> Musical Note / 转音 / 颤音 / 滑音

B. 歌词侧
LRC 原文
  -> Display Token
  -> ReadingSpan
  -> Mora
  -> Phoneme
  -> 歌词时间对齐

最后一步才应该：
歌词时间区间 <-> Musical NoteEvent many-to-many 绑定
```

必须坚持三个原则：

1. **连续 F0 不是音符序列。** 人声颤音、滑音、scoop、fall、音准漂移都会跨过半音边界，但不意味着每次跨界都产生新音符。
2. **歌词字不是日语发音单位。** `愛 -> あ・い`、`二人 -> ふ・た・り`、`位 -> く・ら・い` 都说明 Display Glyph 与 Mora 不是一一对应。
3. **音高变化不能反过来决定日语音素边界。** 真转音可以发生在一个 mora 内；一个 mora 也可能无稳定 F0。

本轮希望得到的是一个可长期维护的架构，而不是“针对这两首歌看起来好一点”的 patch。

---

# 1. 当前源码状态：哪些已经做对，哪些还只是骨架

## 1.1 已经完成且不要推倒重写的部分

当前 `main` 已经完成若干重要修正，应当保留：

- `PitchTrack` 已保留连续 `midis/frequencies/confidences`，同时另存 `note_events`，方向正确。
- `PitchTrack` 已有 `rms` 与 `flux`；`analyzer.rs` 的 spectral flux 来自 log-mel 相邻帧差，可继续作为歌词声学边界证据。
- 日语 tokenizer 已把“显示层”和部分 mora 规则区分开，小假名 Unicode 错误已修复。
- `mora.rs` 已按 mora 规则处理：
  - `きゃ` = 1 mora；
  - `っ` = 1 mora；
  - `ん` = 1 mora；
  - `ー` = 1 mora；
  - `スーパー` = 4 mora。
- `ReadingSpan / MoraUnit / AlignmentSource / UnpitchedReason / NoteBinding` 等数据结构骨架已经存在。
- 当前 mora-aware DP 已经不再把 F0 换音作为歌词边界特征，而使用 voicing/RMS/flux，这是正确方向。
- Enhanced LRC 的逐字时间应继续拥有最高优先级，不要被自动算法覆盖。
- `primary_note` 与 `pitch_notes` 分离的思想正确：紧凑 UI 可以只显示一个代表音，而内部必须保留真正的一字多音。

## 1.2 当前仍然存在的关键缺口

### 缺口 A：`note_tracker.rs` 仍然“先整数化，再补救”

当前第一步直接：

```rust
rounded = midi.round() as i32
```

然后相邻帧只要整数 MIDI 不同就先切成不同 `RawRun`。后续 60 ms debounce 只是在已经切碎之后尝试合并。

这意味着真实 A4 上的 vibrato 只要跨越 69.5/70.5 之类的量化边界，就天然会产生：

```text
A4 -> A#4 -> A4 -> G#4 -> A4 ...
```

只要某段超过 `switch_confirm_ms` 就会留下来。

此外当前 `semitone_hysteresis_cents=25` 实际比较的是整数 `rounded` 的差值，因此并没有形成真正的连续 cents Schmitt trigger。参数名字叫 hysteresis，但算法语义并不是连续音高上的滞回。

**这就是“详细音高满屏转音”的第一根因，而且和日语无关。**

---

### 缺口 B：详细模式目前只是把 `token.pitch_notes` 全部倒出来

前端 `karaoke_display.ts` 当前逻辑就是：

```text
详细模式开启
  -> token.pitch_notes
  -> 每个 PitchNote 转音名
  -> 用 “->” 连接
```

因此只要 NoteTracker 过分割，UI 会忠实地把算法噪声全部显示出来。

详细模式应该展示的是**整理后的 MusicalNoteEvent**，不是原始量化抖动。

---

### 缺口 C：Binder 仍可能产生 phantom note / 邻字串音

当前 `lyrics.rs` 的候选绑定虽然已经区分真实 overlap 与 ±30 ms tolerance，但仍存在两个原则性问题：

1. **零真实重叠的 tolerance candidate 仍可成为正式绑定。**
   当 `real_cands` 为空、`tol_cands` 非空时，代码会直接把邻近 NoteEvent 填给 token，并把整个 token 时间伪造成这个 note 的时间。
2. **任何 `ov_real > 0` 的 NoteEvent 都能进入正式候选。**
   没有最小绝对 overlap / 最小相对 overlap 准入门槛，边界偏几十毫秒时很容易把隔壁音的尾巴收进来。

另外，当前真实候选排序后直接：

```text
primary_note = pitch_notes[0]
```

也就是以当前 soft overlap score 最高者作为主音，而不是重新按“实际覆盖时长 × 置信度 × 稳定性”选择。

---

### 缺口 D：UniDic 仍未真正接入

`japanese/reading.rs` 已经有 `JapaneseReadingProvider`、`KanaOnlyProvider`、`ReadingOverride`，但当前 `build_japanese_layers()` 仍固定：

```rust
let provider = KanaOnlyProvider;
```

所以汉字仍然没有真实 reading，只能回退到 `汉字约 1.8 mora` 的统计启发式。

因此当前架构虽然“允许”表达：

```text
位 -> くらい -> く・ら・い
愛 -> あい   -> あ・い
二人 -> ふたり -> ふ・た・り
```

但实际运行时并未得到这些读音。

---

### 缺口 E：未来接 UniDic 时，`build_japanese_layers()` 还有一个确定性 bug

当前代码在 ReadingSpan 有 `reading` 后，真正拆 mora 的对象却仍然是原始 surface：

```rust
let span_text = 原文[span.char_start..span.char_end];
mora::parse_kana_moras(&span_text)
```

一旦未来出现：

```text
surface = "愛"
reading = "あい"
```

程序仍会拿 `愛` 去解析 kana mora，结果仍然是 0。

必须改为：

```text
normalize(span.pronunciation or span.reading)
  -> parse moras
```

而且注意：**reading 内部的字符下标不能直接加到 surface 的 char_start 上。**

`二人 -> ふたり`、`今日 -> きょう`、`大人 -> おとな`、熟字训/当て字根本不存在可靠的逐汉字 reading 对齐。不要伪造 `ふ -> 二`、`た -> 人` 这种映射。

---

### 缺口 F：Forced Alignment 仍只是枚举项

`AlignmentSource::ForcedAlign` 已存在，但源码注释仍明确是 P3 尚未实现。

现在的 mora DP 是很有价值的 fallback，但它仍然是：

```text
文本时长先验 + voicing/RMS/flux 边界证据
```

不是 phoneme acoustic forced alignment。

---

# 2. 目标架构：把“音符语义”和“歌词语义”彻底拆开

推荐最终数据流：

```text
                         ┌─────────────────────────────┐
                         │        Audio / FCPE         │
                         └──────────────┬──────────────┘
                                        │
                         continuous f0 / midi / conf
                                        │
                         ┌──────────────▼──────────────┐
                         │       Clean Pitch Track     │
                         │ 永远保留，不为 UI 美化而改写 │
                         └──────────────┬──────────────┘
                                        │
                  ┌─────────────────────▼─────────────────────┐
                  │          Musical Note Tracker v2           │
                  │ stable plateau / transition / vibrato      │
                  └──────────────┬─────────────────────────────┘
                                 │
                     MusicalNoteEvent[] + PitchGesture[]
                                 │
                                 │
LRC Surface ─> ReadingSpan ─> Mora ─> Phoneme ─> Alignment
                                 │
                                 ▼
                            MoraInterval[]
                                 │
                 ┌───────────────▼───────────────┐
                 │     Note <-> Mora Binder      │
                 │          many-to-many         │
                 └───────────────┬───────────────┘
                                 │
                       aggregate to DisplayToken
                                 │
                 compact / detailed / debug UI
```

最重要的是：

- `PitchGesture` 不等于 `NoteEvent`；
- `Mora` 不等于 `DisplayToken`；
- `NoteEvent boundary` 不等于 `Phoneme boundary`。

---

# 3. Phase A：最高优先级——重写 NoteTracker v2

这是下一轮首先要做的事情。即使暂时完全不碰日语，只把 NoteTracker 修好，当前“人人都在转音”的截图就应该有肉眼级改善。

## 3.1 不要再使用“先 round() 再分段”作为主算法

主算法必须直接在连续 MIDI / cents 空间工作。

保留 `PitchTrack.midis` 原始/clean 连续数据；离散音符只作为派生层。

建议新增内部表示：

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PitchGestureKind {
    Stable,
    Vibrato,
    Glide,
    Scoop,
    Fall,
    Unvoiced,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitchGesture {
    pub start: f32,
    pub end: f32,
    pub kind: PitchGestureKind,
    pub center_midi: Option<f32>,
    pub from_midi: Option<f32>,
    pub to_midi: Option<f32>,
    pub depth_cents: Option<f32>,
    pub rate_hz: Option<f32>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicalNoteEvent {
    pub start: f32,
    pub end: f32,
    pub midi: i32,
    pub center_midi: f32,
    pub note_name: String,
    pub confidence: f32,
    pub stable_duration: f32,
    #[serde(default)]
    pub gestures: Vec<PitchGesture>,
}
```

为了减少一次性迁移风险，可以第一阶段仍保留现有 `NoteEvent` 名称，但至少增加：

```text
center_midi
stable_duration
tracker_version
```

并在内部引入 `Plateau / Transition`，等 API 稳定后再正式改名。

## 3.2 推荐的 v2 实现：稳定平台驱动，而不是半音边界驱动

### Step A：有效帧筛选

输入：

```text
times[]
continuous_midis[]
confidences[]
rms[]        // 可选但推荐
flux[]       // 可选，主要用于短倚音/onset 证据
```

规则：

- `NaN` / confidence 太低 / RMS 明显静音：标为 invalid；
- 不要直接把一个 10 ms invalid frame 当硬边界；
- 对很短的 unvoiced gap 允许 bridge，但必须要求两侧音高一致或连续。

初始建议：

```text
bridge_gap_ms: 20~30 ms
```

这是可调起点，不是音乐学常数。

### Step B：单独建立“用于事件识别”的稳健 baseline

不要覆盖 UI 原始 F0。

可以对连续 MIDI 建一个小窗口 weighted median / robust median：

```text
raw clean midi ──────────────> 绘图
       │
       └─ robust baseline ───> 事件识别
```

建议窗口先用约 30~50 ms 测试。

重点：不要把窗口拉到 150~300 ms，否则真实快速倚音/melisma 会被抹掉。

### Step C：检测 Stable Plateau

一个新的 musical target 不应该因为 F0 越过 `.5 semitone` 就成立。

推荐状态机：

```text
NoState
  -> CandidateStable
  -> Stable(current_center)
       ├─ small deviation -> 仍属于当前 Stable
       ├─ periodic deviation -> Vibrato gesture，仍属于当前音
       └─ sustained deviation -> CandidateNewTarget
                                  ├─ 不够稳定 -> Transition/return
                                  └─ 足够稳定 -> 确认新音符
```

`current_center` 不要锁死在整数 MIDI，而应来自当前稳定段的 robust median / trimmed mean，再在输出阶段决定 nearest note。

### Step D：建立真正的 cents hysteresis

建议维护两个阈值：

```text
stay_radius_cents
switch_deviation_cents
```

例如第一版可以从：

```text
stay_radius_cents       = 40 c
switch_deviation_cents  = 65~75 c
```

开始回归测试。

含义：

- 在当前中心 ±40 cents 内，绝不因为四舍五入结果改变而换音；
- 超出 40 cents 也不立即换音；
- 只有偏离足够大，并在新目标附近持续形成稳定平台，才确认新音。

这些值必须放入 `NoteTrackingParams`，但不暴露为普通用户必须理解的十几个滑块。高级/开发模式可见即可。

### Step E：确认新目标必须看“持续 + 稳定”，不是只看持续

当前算法只有 duration debounce。v2 至少要同时满足：

```text
candidate duration
candidate pitch dispersion (MAD / robust std)
candidate confidence
pitch separation from old target
```

普通新音建议起始阈值：

```text
normal stable duration: 70~100 ms
```

但绝不能把所有 70 ms 以下的音都删掉，因为倚音可能更短。

短倚音例外：

```text
45~70 ms
+ 高 confidence
+ 与相邻 target 有明显音程
+ flux/onset 证据较强（如果可用）
```

则允许保留。

这比简单把 `min_note_duration_ms` 拉到 150 ms 合理得多。

---

## 3.3 明确定义三种最重要的人声现象

### A. Vibrato：一个音，不拆

例如中心 A4：

```text
69.0 MIDI
  ↗ 69.45 ↘ 68.60 ↗ 69.40 ...
```

即使跨过半音 round 边界，只要整体围绕同一 target 周期摆动，就应：

```text
MusicalNoteEvent: A4
Gesture: Vibrato
```

而不是：

```text
G#4 -> A4 -> A#4 -> A4 -> ...
```

v1 不需要把 vibrato rate/depth 做到声乐科研级准确；只要它**不触发新音符**即可。

如果实现 gesture 分类，可选检测：

- 残差围绕同一中心正负交替；
- 没有形成另一个稳定平台；
- 可用自相关/zero crossing 辅助判断 3~9 Hz 区间，但不要把频率范围做成换音硬门槛。

### B. Glide / portamento：中间经过的半音不是新音

```text
A4 stable
   / 连续滑升
             B4 stable
```

输出：

```text
[A4, B4]
Gesture between them: Glide
```

不得输出：

```text
A4 -> A#4 -> B4
```

判断关键：中间区域呈连续/近似单调趋势，但没有 A#4 稳定平台。

### C. True Melisma：必须保留

例如用户提到的真实“一字多音”场景，一个汉字/音节内若出现：

```text
G4 stable -> A4 stable -> G4 stable
```

三个区域都形成真实稳定目标，就必须输出：

```text
G4 -> A4 -> G4
```

这就是详细模式存在的价值。

因此不能用“一个字最多一个音”来掩盖过分割问题。

---

## 3.4 NoteTracker v2 必须有的合成回归测试

新增例如：

```text
src-tauri/tests/note_tracker_v2_test.rs
```

至少钉死：

### Test 1：纯稳音

```text
A4, 1.0 s, 少量 ±10c noise
=> exactly [A4]
```

### Test 2：跨 round 边界的 vibrato

```text
A4 center, 1.0 s
5~7 Hz sinusoidal ±50~60 cents
=> exactly [A4]
=> 允许 gesture=Vibrato
```

这是当前版本最关键的回归测试。

### Test 3：稳定音靠近半音边界

```text
69.45 ~ 69.55 MIDI 来回抖 1 s
=> 不得 A4/A#4 疯狂切换
=> 应稳定为一个主目标
```

### Test 4：glide

```text
A4 stable 300 ms
linear glide 120 ms
B4 stable 300 ms
=> [A4, B4]
=> 不得出现 A#4 event
```

### Test 5：真 melisma

```text
A4 stable 150 ms
B4 stable 150 ms
A4 stable 180 ms
=> [A4, B4, A4]
```

### Test 6：短倚音

```text
G4 55~65 ms + strong evidence
A4 400 ms
=> 保留 G4 + A4
```

### Test 7：假 flicker

```text
A4 300 ms
A#4 excursion 20~30 ms
A4 300 ms
=> exactly [A4]
```

### Test 8：octave glitch

```text
A4 -> A5 20~40 ms -> A4
=> exactly [A4]
```

### Test 9：短静音

```text
A4 -> 20ms invalid -> A4
=> 仍为一个 A4（条件：两侧连续且同 target）
```

### Test 10：真正重发同音

如果未来使用 flux/onset：

```text
A4 stable -> 明确断音/onset -> A4 stable
```

可以选择保留为两个事件，但 UI 若只展示音名可以视觉合并。此项不要阻塞 v2 第一版。

---

# 4. Phase B：Binder v2——不再用邻居 NoteEvent “补空白”

NoteTracker 修正之后，再修 `lyrics.rs` 的 Note <-> Token/Mora 绑定。

## 4.1 候选发现与正式准入必须拆开

允许 `±30 ms` tolerance 做：

```text
debug candidate discovery
```

但**零真实 overlap 的候选不能自动成为正式 badge**。

因此改成：

```text
CandidateDiscovery
  ├─ real overlap > threshold -> EligibleBinding
  └─ zero real overlap -> NearBoundarySuggestion only
```

如果一个 token 只有 `NearBoundarySuggestion`：

```text
primary_note = None
unpitched_reason = NoOverlappingNote
```

Debug Overlay 可以显示“最近有 D4，距离 18 ms，但未正式绑定”。

不要为了 UI 好看而制造 phantom note。

---

## 4.2 正式 binding 必须有动态 overlap 门槛

不建议用一个死的 `20 ms` 处理所有 token。

第一版可采用：

```text
min_overlap = clamp(
    0.25 * min(token_duration, note_duration),
    10 ms,
    30 ms
)
```

并同时要求：

```text
overlap >= min_overlap
AND
(overlap_ratio_token >= 0.12 OR overlap_ratio_note >= 0.12)
```

这只是工程起点，需要测试集调优。

目标是同时满足：

- 长 NoteEvent 横跨多个短 mora/token 时仍可以 many-to-many 共享；
- 隔壁音仅漏进来 5~15 ms 时不能成为正式候选。

---

## 4.3 真的去使用 `NoteBinding`

当前模型已经定义 `NoteBinding`，但应把它真正填充进 mora/token 结果，而不是只生成 `PitchNote`。

推荐每次 binding 保留：

```text
note_event_index
overlap_ms
overlap_ratio_token/mora
overlap_ratio_note
score
accepted/rejected reason（debug 数据可单独保存）
```

最好先在 **Mora 层绑定**，然后向 DisplayToken 聚合。

原因：

```text
一个 display token -> 多 mora
一个 mora -> 多 musical notes（真 melisma）
一个 long note -> 多 mora
```

Mora 是比汉字更自然的时间绑定单位。

---

## 4.4 `primary_note` 的选择标准

紧凑模式的主音应更接近：

```text
primary_score = true_overlap_duration
              * event_confidence
              * stability_weight
```

如果 MusicalNoteEvent v2 已经都是稳定事件，`stability_weight` 可先为 1 或来自 `stable_duration/event_duration`。

不要简单使用“候选排序中的 overlap geometry soft score 第一名”。

规则建议：

1. 先过滤未达到 binding admission threshold 的候选；
2. 对保留下来的事件计算 token/mora 内实际覆盖时长；
3. `duration × confidence × stability` 最大者为 primary；
4. 若两个稳定事件覆盖接近，则 primary 只影响紧凑 UI，详细模式仍显示两者。

---

## 4.5 无音高必须有原因，而不是默默空白

继续使用 `UnpitchedReason`，但补全语义：

```text
AlignmentMissing
NoVoicing
LowPitchConfidence
NoOverlappingNote
ClosureOrDevoicing
```

其中 `ClosureOrDevoicing` 不要根据“没有 F0”随便猜。

只有有语言学/声学证据时再判：

- forced alignment 明确是 `cl`（促音闭塞）等非周期区；
- 或后续有可靠的 devoiced-vowel 判据。

`ん/N` 不能因为是特殊 mora 就默认无声，它通常可以有 voicing。

---

# 5. Phase C：真正完成日语 Reading -> Mora

这是解决“汉字对应多少拍”问题的核心，而不是继续调 `1.8`。

## 5.1 接入 Lindera + UniDic

当前 Cargo 里仍没有 Lindera。

推荐实现：

```rust
pub struct LinderaUnidicProvider { ... }

impl JapaneseReadingProvider for LinderaUnidicProvider {
    fn analyze(&self, text: &str) -> Result<Vec<ReadingSpan>, String> { ... }
}
```

以当前生态为基线，可评估 Lindera 6.x + `lindera-unidic`。

但**不要把 crates.io package source size 当作最终字典体积**。实际安装包/资源大小必须 release build 后测量。

UniDic 官方解析字典本身可达数百 MB，所以资源策略必须单独设计。

推荐：

```text
Pitch Analyzer 主程序
+
Japanese Accuracy Pack / Japanese Dictionary Resource
```

可以有三种部署方式，开发 Agent 选最适合 Tauri release 的一种：

1. installer resource；
2. 首次启用日语精确对齐时本地安装；
3. app data 目录外置 dictionary。

要求：

- 离线可用；
- 有 version/hash；
- 缺词典时不崩溃，自动退化到 `KanaOnlyProvider`；
- tokenizer/provider 在 AppState 中缓存，不要每行歌词重新初始化巨大词典。

### License

项目当前是 `MIT OR Apache-2.0`。

UniDic 官方提供 GPL v2 / LGPL v2.1 / 修正 BSD 等许可选项。若要跟当前项目友好分发，优先选择并完整遵循官方给出的 permissive/修正 BSD 路线，并把对应 LICENSE/AUTHORS/NOTICE 一并打包。具体采用哪个 UniDic 包时必须再次确认该包附带的许可证文本。

不要直接复制 GPLv3 forced-aligner 的源码进主程序。

---

## 5.2 修 `build_japanese_layers()`：必须从 reading 拆 mora

当前：

```text
ReadingSpan.reading 已存在
但 mora parser 读取 surface
```

必须改为：

```rust
let phonetic_text = if !span.pronunciation.is_empty() {
    normalize_reading(&span.pronunciation)
} else {
    normalize_reading(&span.reading)
};

let parsed = mora::parse_kana_moras(&phonetic_text);
```

但不要再做：

```rust
mora.char_start = span.char_start + reading_char_offset;
```

因为 reading offset 与 surface offset 没有普遍一一对应关系。

---

## 5.3 不伪造逐汉字 reading 映射

以下必须作为设计测试：

```text
愛   -> あい
二人 -> ふたり
今日 -> きょう
大人 -> おとな
一昨日 -> おととい
```

不要强行回答：

```text
二 = ふ？
人 = たり？
```

这种拆法在通用情况下没有唯一语言学答案。

推荐让一个 `ReadingSpan` 覆盖多个 display char：

```text
ReadingSpan("二人", reading="ふたり")
    mora[0] = ふ
    mora[1] = た
    mora[2] = り
```

每个 Mora 只需要知道：

```text
reading_span_id
mora_index_in_span
```

若现有 `MoraUnit.char_start/char_end` 必须保留兼容，可在无法精确映射时让每个 mora 暂时继承**整个 ReadingSpan 的 surface span**，并明确注释这是 coarse display span，不是“这个 mora 对应某个具体汉字”。

更干净的长期方案是给 `MoraUnit` 新增：

```rust
pub reading_offset_start: usize,
pub reading_offset_end: usize,
pub display_start: usize,
pub display_end: usize,
```

其中 display span 可覆盖多字。

---

## 5.4 `ReadingOverride` 必须真正进入 pipeline

优先级建议固定为：

```text
Enhanced LRC timing（时间权威）

读音：
user/song ReadingOverride
> embedded ruby/furigana（未来若支持）
> UniDic
> KanaOnly
> heuristic
```

需要实现歌曲特殊读法，例如：

```text
運命 -> さだめ
```

不能指望普通词典自动知道作词人这一次唱的是特殊 reading。

Override 至少保存：

```text
line identifier / original span
surface text
reading
```

不要只依赖脆弱的全局 char index；歌词重新载入后应尽可能可重定位。

---

## 5.5 日语 reading/mora 单元测试

必须增加：

```text
愛       => あ・い            (2)
二人     => ふ・た・り        (3)
今日     => きょ・う          (2)
大人     => お・と・な        (3)
学校     => が・っ・こ・う    (4)
スーパー => ス・ー・パ・ー    (4)
きゃ     => 1 mora
りー     => 2 mora
```

以及：

```text
override: 運命 -> さ・だ・め
```

测试重点不是只 assert mora count；还要 assert：

- ReadingSpan surface 不丢；
- reading 正确；
- mora_start/mora_end 连续；
- display span 不因多字 reading 被错误拆散；
- token ↔ ReadingSpan 关联稳定。

---

# 6. Phase D：Mora 级时间对齐，然后再决定是否上 Forced Aligner

## 6.1 当前 mora DP 保留为 fallback

当前已有的：

```text
voicing change
RMS variation
spectral flux
时长先验
```

可以继续维护。

它的职责应明确为：

```text
快速、离线、无额外模型的 heuristic fallback
```

不是“最终精准日语 forced alignment”。

优化当前 DP 时，不要重新引入 F0 note change 作为强边界特征。

因为：

```text
一个 mora 内可以转音
歌词边界也可以在同音高上发生
```

---

## 6.2 P3 定义统一 ForcedAligner 接口

先定义接口，再选模型：

```rust
pub trait ForcedAligner: Send + Sync {
    fn name(&self) -> &'static str;

    fn align(
        &self,
        audio: &AudioSlice,
        phonemes: &[PhonemeUnit],
        line_window: (f32, f32),
    ) -> Result<Vec<PhonemeInterval>, AlignError>;
}
```

输出至少：

```rust
struct PhonemeInterval {
    phoneme: String,
    start: f32,
    end: f32,
    confidence: f32,
    skipped: bool,
}
```

再按：

```text
phoneme -> mora -> ReadingSpan -> DisplayToken
```

逐层聚合。

---

## 6.3 评估路线，而不是立即锁死某个第三方项目

### pyshiro

优点：

- 明确面向 Japanese singing voice；
- HSMM forced alignment；
- 有 HMM -> HSMM 两阶段思路；
- 对 Japanese singing 的工程参考价值很高。

缺点：

- 当前仓库许可证为 GPLv3；
- 不建议直接把它的代码复制/链接进当前 MIT/Apache 主程序。

建议用途：

```text
offline benchmark / oracle / 研究参考
```

### SOFA

优点：

- Singing-Oriented Forced Aligner；
- 有 ONNX inference/export；
- 支持 custom G2P；
- ONNX 路线与项目现有 `ort` 技术栈匹配。

但必须先验证：

- 是否有适合日语的现成模型；
- phoneme inventory 是否匹配；
- 日语训练数据/模型许可是否适合分发。

### narabas

可作为 Japanese speech phoneme alignment baseline，但不要因为“是日语”就默认它会比 singing-specific 方法更好。歌声的时值、拉长、音高运动与普通语音不同。

---

## 6.4 P3 的上线策略

建议分三步：

```text
D1. 独立 benchmark script
    同一批人工标注片段跑 MoraDP / pyshiro / SOFA候选 / 其他模型

D2. 选定可分发的模型
    导出/直接使用 ONNX
    接入现有 ort

D3. 正式生产
    ForcedAlign 成功且 confidence 足够 -> 使用
    否则自动 fallback MoraDp
```

不要为了“功能表上有 Forced Align”而在低 confidence 时强行输出伪精确时间。

---

# 7. Phase E：UI 重新定义——三层信息，不再只有“简单/把全部倒出来”

建议把当前 UI 逻辑改成三档：

## 7.1 Compact（默认）

只显示：

```text
primary musical note
```

例如：

```text
我
A4
```

无音时不随便拿邻居补，Debug/tooltip 可解释原因。

## 7.2 Detailed Musical Notes

只显示真正的 `MusicalNoteEvent[]`：

```text
我
G4 -> A4 -> G4
```

适用于真 melisma。

以下不应展开成多个 badge：

```text
vibrato
细小 pitch drift
glide 中经过的中间半音
FCPE 量化边界抖动
```

可选的轻量表达：

```text
A4 ~       vibrato
A4 ↗ B4    glide
```

但这是锦上添花，不要阻塞核心功能。

## 7.3 Debug Overlay（强烈要求本轮实现）

这不是“冗余功能”，而是后续不再靠截图猜 bug 的关键工具。

点击/悬停 token 时显示：

```text
Surface: 愛
Reading: あい
Moras: [あ, い]
Phonemes: [...]

Token time: 135.220 - 135.610
Alignment source: MoraDp / ForcedAlign / EnhancedLrc
Alignment confidence: 0.xx

Accepted notes:
  D4  overlap=286ms  token_ratio=0.73  note_ratio=...

Rejected near-boundary candidates:
  D#4 distance=18ms  reason=zero_real_overlap

Primary: D4
Unpitched reason: ...

Pitch gesture:
  vibrato / glide / stable
Tracker version: 2
```

最好提供一个“复制调试 JSON”按钮，方便今后提交 bug 时直接粘贴数据，而不是只看截图。

---

# 8. 数据模型与工程文件兼容

这是容易被忽略但非常重要的一部分。

## 8.1 不要破坏旧工程

新字段全部：

```rust
#[serde(default)]
```

推荐加入版本：

```rust
pub analysis_version: u32,
pub note_tracker_version: u32,
pub alignment_version: u32,
pub reading_pipeline_version: u32,
pub dictionary_version: Option<String>,
```

或把版本集中到 `ProjectMetadata`。

## 8.2 旧 `note_events` 应允许自动重算

旧工程若包含连续：

```text
PitchTrack.times
PitchTrack.midis
PitchTrack.confidences
```

则：

```text
tracker_version < 2
=> 从已有 clean track 重新生成 MusicalNoteEvent
```

不需要重新跑 FCPE。

歌词方面：

- Enhanced LRC 时间必须保留；
- `MoraDp/WeightedFallback` 自动时间允许重新计算；
- 用户手工修正（若未来支持）必须有独立 source，不被自动覆盖。

## 8.3 不要为了让 UI 变干净而改写原始连续 F0

任何时候都要保留：

```text
continuous raw/clean contour
```

NoteTracker v2 是 annotation layer，不是 DSP destruction layer。

---

# 9. 建议的具体文件修改清单

## `src-tauri/src/models.rs`

新增/调整：

- `PitchGestureKind`
- `PitchGesture`
- `MusicalNoteEvent` 或升级现有 `NoteEvent`
- `tracker_version`
- 必要的 `NoteBinding` debug 信息
- Mora 与 ReadingSpan 的非一一 display mapping 字段
- 项目版本字段
- 所有新增持久化字段 `#[serde(default)]`

不要删除当前连续 F0 字段。

---

## `src-tauri/src/note_tracker.rs`

**本轮最大重构点。**

拆成若干小函数，不要再写成一个 200 行 `build_note_events()`：

```text
prepare_valid_frames()
suppress_short_octave_glitches()
bridge_micro_gaps()
robust_pitch_baseline()
detect_stable_plateaus()
classify_transitions()
merge_false_splits()
build_musical_note_events()
```

如果 gesture classification 第一轮太大，至少先实现：

```text
stable plateau + transition
```

并确保 vibrato/glide 不制造中间 NoteEvent。

---

## `src-tauri/src/analyzer.rs`

- 给 NoteTracker v2 提供必要的 `rms/flux`（如算法选择使用）；
- 不重复 FFT，复用当前 mel/RMS；
- FCPE 本身先不换；
- 保证 old project 可只从 PitchTrack 重建 note events。

---

## `src-tauri/src/lyrics.rs`

重点：

1. Binder v2：
   - tolerance 仅发现，不正式零 overlap 绑定；
   - 动态 overlap admission；
   - explicit `NoteBinding`；
   - primary 按真实覆盖/稳定性选。
2. `build_japanese_layers()`：
   - provider 不再写死 `KanaOnlyProvider`；
   - mora 必须从 `span.pronunciation/reading` 展开；
   - 禁止把 reading offset 假装成 surface char offset；
   - Mora 按 ReadingSpan 聚合。
3. 保留 Enhanced LRC 优先级。
4. MoraDP 继续作为 fallback。

---

## `src-tauri/src/japanese/reading.rs`

实现：

- `LinderaUnidicProvider`
- reading normalization
- provider fallback
- `ReadingOverride` 真正接 pipeline
- dictionary metadata/version

建议增加：

```text
reading_provider.rs / unidic_provider.rs
```

如果单文件过大，不要把所有字典初始化和业务逻辑塞一起。

---

## `src-tauri/src/japanese/mora.rs`

当前核心规则基本正确，优先保持稳定。

只做：

- reading normalization 后的兼容测试；
- 补缺少的现代片假名组合；
- 不要再次把 `ー` 当成 0 mora。

---

## `src-tauri/src/japanese/phoneme.rs`

当前手写映射可继续用于 P2/P3 prototype。

P3 选定模型后再对齐该模型的 phoneme inventory，不要现在凭空扩一套“看起来很全面”的标签集。

需要明确处理：

```text
っ / cl
ん / N
ー / vowel length
pause / breath（forced aligner 如支持）
```

---

## `src-tauri/src/lib.rs`

- Japanese dictionary/provider 放入 AppState，缓存初始化；
- forced aligner 同理按需 lazy load；
- 添加 debug JSON/export command；
- 旧项目 load 后按版本决定是否重建 note events / auto alignment；
- 不重复加载几百 MB 词典或 ONNX 模型。

---

## `src/karaoke_display.ts`

从：

```text
compact = primary
Detailed = pitch_notes 全部拼箭头
```

改成：

```text
compact = primary MusicalNoteEvent
Detailed = curated MusicalNoteEvent[]
Debug = raw contour/event/binding/alignment 信息
```

必要时把 debug overlay 拆独立组件，不要继续让 `renderLyrics()` 膨胀。

---

## TypeScript models

所有后端新增字段同步更新，不要大量 `any`。

旧工程缺字段时前端必须能安全 fallback。

---

# 10. 测试语料：必须同时有“合成信号”和“真实歌曲”

只拿真实歌调参数会很容易过拟合，也很难知道 ground truth。

## 10.1 Synthetic Pitch Corpus

程序生成 MIDI contour 即可，不涉及版权。

必须覆盖 Phase A 的 10 组测试。

建议把合成器做成测试 helper：

```rust
fn synth_midi_segment(...)
fn add_vibrato(...)
fn add_glide(...)
fn add_noise(...)
fn add_unvoiced_gap(...)
```

这样以后每次改 NoteTracker 都能自动 regression。

## 10.2 Japanese Reading Corpus

至少覆盖：

```text
愛
二人
今日
大人
学校
一昨日
スーパー
きゃ
りー
```

以及真实歌词片段：

```text
知らない 二人 にもどるのね
愛が終わるのを見とどけている
悲しい位に見つめてた
冷たく微笑んだ
```

这里测试的是：

```text
surface -> reading -> mora -> token aggregation
```

而不是歌曲音高本身。

## 10.3 True Melisma Corpus

至少手工标 10~20 个明确“一字/一音节多稳定音高”的片段。

用户举出的中文测试点可以作为 regression：

```text
“说你会永远陪着我，陪着我。”中的“我”
```

只在本地使用用户自己的音频，不把受版权保护的歌曲音频提交进仓库。

需要人工标注：

```text
token start/end
人工认定 musical notes
是否 vibrato/glide/melisma
```

## 10.4 Sustained/Vibrato Corpus

反过来再找 10~20 个明显长音 + vibrato，但音乐意义上只有一个 target note 的片段。

否则算法很容易为了保留 melisma 又重新过分割。

---

# 11. 验收标准：没有这些指标就不要宣布“修好了”

## 11.1 NoteTracker v2 硬性验收

Synthetic：

```text
稳音             -> 1 event
vibrato          -> 1 event
glide A4->B4     -> 2 events，无 A#4 中间 event
true A4->B4->A4  -> 3 events
20~30ms flicker  -> 不成 event
短 octave glitch -> 不成 event
```

真实人工小语料目标：

- sustained/vibrato false split rate <= 5%；
- 明确 true-melisma 的额外稳定 target recall >= 90%；
- 不能靠把最小时长拉高到导致大量短真音消失来换取 false split 下降。

这些是项目内部验收目标，不是宣称科研 benchmark。

## 11.2 Binder 硬性验收

- 正式 binding 中 `real_overlap == 0` 的数量必须为 0；
- debug 可以保留 near-boundary suggestion，但不得渲染正式 badge；
- 隔壁 NoteEvent 只漏入几毫秒时不得绑定；
- 一个长音跨多个 mora 时允许共享；
- 一个 mora 的 true melisma 允许多个 MusicalNoteEvent；
- primary 必须是实际占据该 token/mora 最主要的稳定音，而不是边界小碎音。

## 11.3 Japanese Reading 硬性验收

至少：

```text
愛       2 mora
二人     3 mora
今日     2 mora
大人     3 mora
学校     4 mora
スーパー 4 mora
```

而且多汉字 ReadingSpan 不得伪造错误逐字 reading。

## 11.4 Alignment 分阶段目标

MoraDP：

- 作为 fallback，不要求做到 phoneme model 水准；
- 明显比 weighted uniform 好；
- 不能因 note change 造成字边界乱跳。

ForcedAlign P3：

建议在人工标注小语料上使用：

```text
median absolute boundary error <= 50~60 ms
90% boundary error <= 100~120 ms
```

若候选模型长期达不到，就保持 `MoraDp` 生产可用、`ForcedAlign` experimental，禁止为了完成 roadmap 强行上线。

## 11.5 UI 验收

当前截图那种：

```text
几乎每一个假名字上都有 A#4->A4->... 六七个箭头
```

必须显著消失。

详细模式应达到直觉：

- 普通稳定音：大多数 token 仍只有一个 badge；
- 真转音：少数 token 出现 2~N 个稳定音；
- vibrato：不展开为一串相邻半音；
- compact：不因零 overlap 邻音强行补错音；
- 无音 badge 有可查原因。

---

# 12. Debug/日志：这部分优先级比继续加 UI 功能更高

实现一个内部分析导出，例如：

```text
Debug -> Export segment analysis JSON
```

对当前播放点 ±2~5 s 导出：

```json
{
  "pitch_frames": [],
  "robust_baseline": [],
  "plateaus": [],
  "gestures": [],
  "musical_note_events": [],
  "line": {
    "reading_spans": [],
    "moras": [],
    "tokens": []
  },
  "bindings": [],
  "rejected_candidates": []
}
```

以后出现“这个字明显音高不对”时，开发者只需要这份 JSON + 时间戳，就能判断：

```text
FCPE 错？
NoteTracker 错？
Reading 错？
Alignment 错？
Binder 错？
```

而不再靠肉眼盯截图猜。

---

# 13. 性能与资源约束

## NoteTracker

应尽量保持：

```text
O(N) 或 O(N * small_window)
```

不要引入全局 O(N²) segmentation。

## MoraDP

当前 line-level DP 若存在 frames² 项，应考虑 duration prior 限定搜索带宽/beam，避免长句性能恶化。

## UniDic

- provider/tokenizer 初始化一次并缓存；
- 不要每行构建字典；
- release 后实际记录磁盘与内存占用。

## Forced Aligner

- 模型 lazy load；
- 只对需要自动对齐的行/时间窗运行；
- 现有 `ort` 优先复用，避免再引入一整套 Python runtime 到桌面程序。

---

# 14. 本轮明确不要做的事情

以下方案禁止作为“完成修复”的主要实现：

1. 单纯把 `switch_confirm_ms` 从 60 提到 150/200 ms。
2. 单纯把 `min_note_duration_ms` 拉高到所有短真转音都消失。
3. 继续以 `midi.round()` 的整数变化作为事件初始分割定义。
4. 把每次 F0 跨半音边界当成新音。
5. 为了少显示箭头，直接让一个 token 最多保留一个音。
6. 用邻居 NoteEvent 的 ±30 ms tolerance 给没有真实 overlap 的字补 badge。
7. 继续用汉字 `1.8` 权重冒充真实日语 reading。
8. 把 `span.reading` 取到了，却仍然拿 surface 去拆 mora。
9. 强行把熟字训/多字词的 mora 一一映射到单个汉字。
10. 把 NoteEvent boundary 当作 phoneme/mora boundary。
11. 为了 UI 干净而把连续真实 F0 曲线本身量化/覆盖。
12. 在没有对比基准的情况下换掉 FCPE。当前首要问题不是证明 FCPE 不准，而是其连续输出被错误离散化。
13. 直接复制 pyshiro GPLv3 代码进入 MIT/Apache 主项目。
14. 一次塞入大量与此次问题无关的功能。

---

# 15. 推荐实施顺序与 Git 提交拆分

用户需要的是“一次性规划”，但代码不要做成一个巨大不可审查 commit。

推荐按以下顺序：

## Commit / PR A0 — Regression harness + Debug foundation

- Synthetic note contour test helper；
- 当前算法先跑出 baseline；
- 加 segment debug JSON 数据结构；
- 不改变生产结果。

**验收：** 测试框架可稳定复现当前 vibrato 过分割。

## A1 — NoteTracker v2 stable plateau engine

- 去掉 `round-first segmentation`；
- 连续 cents hysteresis；
- stable candidate state machine；
- octave glitch/micro-gap 处理。

**验收：** Synthetic 1~9 全过。

## A2 — Gesture / transition classification

- glide 不生成中间音；
- vibrato 记录为 gesture；
- true stable multi-target 保留。

**验收：** Detailed note count 明显回归正常。

## B1 — Binder admission hardening

- 零 overlap 不正式绑定；
- 动态 admission threshold；
- primary 重新评分；
- explicit NoteBinding。

**验收：** phantom binding = 0。

## B2 — Mora-level binding + aggregation

- MusicalNoteEvent <-> Mora many-to-many；
- 再向 token 聚合；
- 绑定 debug 数据完整。

## C1 — Lindera/UniDic provider

- 词典初始化/cache/fallback；
- Cargo/resource/license；
- reading tests。

## C2 — Fix ReadingSpan -> Mora pipeline

- 从 reading/pronunciation 拆 mora；
- 修多字 span mapping；
- 接 ReadingOverride。

**验收：** `愛/二人/今日/大人/学校` 全过。

## D1 — Forced align benchmark harness

- 定义统一接口；
- 人工标注 mini corpus；
- 比较 MoraDP / singing align candidates。

## D2 — Production forced aligner（若达标）

- ONNX/runtime 集成；
- confidence + fallback；
- packaging。

## E1 — UI semantic modes + Debug Overlay

- Compact；
- Detailed Musical Notes；
- Debug Overlay；
- rejected candidate/UnpitchedReason 可见。

UI 可以在 A/B 后先做一版，P3 不必等完才开始。

---

# 16. 每个阶段必须跑的质量门

每个 PR/阶段必须至少：

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
pnpm exec tsc --noEmit
pnpm build
```

若项目现有实际脚本不同，以仓库 `package.json/Cargo.toml` 为准调整，但最终必须同时覆盖 Rust test + TS typecheck + frontend build。

并保留一份自动生成的 regression summary，例如：

```text
Synthetic NoteTracker: 10/10 pass
Japanese Reading: 9/9 pass
Zero-overlap bindings: 0
Manual corpus false-split: x%
Manual melisma recall: x%
Alignment MAE: xx ms
```

不要只以“肉眼看截图好像舒服了”作为验收。

---

# 17. 遇到实现分歧时的决策原则——不要再让用户当传话筒

开发 Agent 遇到以下细节，直接按工程原则做决定，不需要每个参数都回来询问：

- Rust 内部 struct 是拆文件还是放同文件；
- 35/40/45 cents 哪个初始值；
- median window 是 3 帧还是 5 帧；
- Debug Overlay 是 tooltip 还是 side panel；
- 单元测试 helper 的具体目录；
- Lindera API 的具体构造方式；
- 某个字段是否需要 `Option<T>`。

决策标准只有：

```text
可测试
可解释
不伪造数据
不破坏连续 F0
不把歌词/音高边界混为一谈
旧工程可迁移
无额外不必要功能
```

只有遇到以下情况才需要停下来询问用户：

- 必须更改整个项目许可证；
- 必须下载/分发数百 MB 或 GB 级新模型且没有合理可选安装方案；
- 必须改变用户现有项目文件且无法兼容迁移；
- 需要付费/闭源服务；
- 发现 FCPE 本身在人工 ground truth 上存在系统性错误，必须替换主模型。

其余问题由 Agent 自己做技术判断并在 PR 说明中记录。

---

# 18. “完成”的定义

这一轮真正完成，不是指：

```text
截图里箭头少了
```

而是下面五件事同时成立：

1. **NoteTracker 不再由整数半音 crossing 驱动。**
2. **普通 vibrato/glide 不再伪装成大量转音，真实 melisma 仍能保留。**
3. **Binder 不再靠零 overlap 邻音填空，错音/漏音有可解释证据。**
4. **日语汉字真正经过 reading -> mora，`愛/二人/位` 不再靠 1.8 猜拍数。**
5. **每个错误可以通过 Debug Overlay/JSON 定位到 FCPE、NoteTracker、Reading、Alignment 或 Binder 中的一层。**

P3 Forced Alignment 如果还需要更长时间训练/评估，可以作为随后一阶段；但 A+B+C 做完以后，当前截图中的两个最大症状——“假转音爆炸”和“汉字对齐仍靠猜”——应该已经发生结构性改善。

---

# 19. 参考资料（供开发 Agent 调研，不代表要求直接复制实现）

## 当前项目源码

- Repository: https://github.com/hydrogen1222/pitch-analyzer
- `note_tracker.rs`: https://raw.githubusercontent.com/hydrogen1222/pitch-analyzer/main/src-tauri/src/note_tracker.rs
- `lyrics.rs`: https://raw.githubusercontent.com/hydrogen1222/pitch-analyzer/main/src-tauri/src/lyrics.rs
- `models.rs`: https://raw.githubusercontent.com/hydrogen1222/pitch-analyzer/main/src-tauri/src/models.rs
- `japanese/reading.rs`: https://raw.githubusercontent.com/hydrogen1222/pitch-analyzer/main/src-tauri/src/japanese/reading.rs
- `karaoke_display.ts`: https://raw.githubusercontent.com/hydrogen1222/pitch-analyzer/main/src/karaoke_display.ts

## Note event 与连续 pitch contour 分离

- Spotify Basic Pitch `note_creation.py`：note/onset 与 contour/pitch bend 分层思想  
  https://github.com/spotify/basic-pitch/blob/main/basic_pitch/note_creation.py

- Pitch contour segmentation（steady / transitory / vibrato）可用于理解“音符”和“音内表现”的区别；不要求照搬模型。

## Japanese morphological analysis

- Lindera: https://docs.rs/crate/lindera/latest
- lindera-unidic: https://docs.rs/crate/lindera-unidic/latest
- UniDic official downloads/license: https://clrd.ninjal.ac.jp/unidic/back_number.html

截至本任务书审查时，Lindera/lindera-unidic 当前可见为 6.0.0；具体 API 和 feature 以开发时实际锁定版本为准。

## Singing forced alignment

- pyshiro — Japanese singing voice HSMM forced alignment  
  https://github.com/wavtechyukky/pyshiro  
  注意：仓库 LICENSE 为 GPLv3，主要作为 benchmark/研究参考。

- SOFA — Singing-Oriented Forced Aligner，含 ONNX inference/export  
  https://github.com/qiuqiao/SOFA

- narabas — Japanese phoneme forced alignment baseline  
  https://github.com/darashi/narabas

---

# 20. 最终给开发 Agent 的一句话

**不要再“优化半音 run 合并”和“调汉字权重”。先把连续 F0 中的稳定 musical target 正确抽出来，再把日语 surface 真正展开为 reading/mora/phoneme，最后在时间轴上做 many-to-many 绑定。Vibrato/Glide 是音符内部或音符之间的 pitch gesture，Melisma 才是同一歌词单位上的多个稳定音符；这三者必须在数据模型和测试中明确区分。**

