# Pitch Analyzer Round 3 最终源码审计报告 + Round 3.1 完成性修复任务书

> 仓库：`https://github.com/hydrogen1222/pitch-analyzer`  
> 审计对象：2026-08-30 当前 GitHub `main`  
> 目的：核验“Round 3 已完整执行并通过全量测试”的真实性，并给开发 Agent 一份可直接执行的收尾任务书。  
> **本文件不是新一轮功能扩张计划。Round 3.1 的唯一目标，是把 Round 3 已承诺但实际上仍未接通的主链路真正做完，并修复本轮审计发现的确定性缺陷。**

---

# 0. 结论先行

## 审计结论：Round 3 **不能判定为完成**

当前仓库确实有一批**真实进展**：

- Lindera + embedded UniDic 已作为真实依赖加入；
- `LinderaUnidicProvider` 已经真实加载词典；
- `愛 → あ・い`、`二人 → ふ・た・り`、`心 → こ・こ・ろ` 等 reading regression tests 已存在；
- `note_engine` 抽象、`MusicalNoteEvent`、`MusicalNoteSource` 已建立；
- `forced_align` 的统一 trait 与数据结构已建立；
- 旧 NoteTracker v2 仍可作为 fallback；
- Binder 的“任意一丁点重叠就录取”问题相比旧版已有改善；
- Debug 数据结构与一部分调试导出已经存在。

这些都不是“零进展”。

但是，Round 3 最核心的三件事中：

1. **GAME 没有真实推理；当前 `GameNoteEngine::transcribe()` 是占位实现。**
2. **Forced Alignment 没有任何真实 backend；当前只有 trait。**
3. **程序运行时主链路仍然是 `FCPE → NoteTracker → MoraDP → Binder`，并没有切换成 Round 3 规定的 `FCPE + GAME + UniDic/FA + matcher`。**

此外还发现一个很严重的日语 mapping 问题：

4. **UniDic 的多字符 ReadingSpan 会让同一组 mora 在 `build_align_units()` 中被重复加入多次。**

以及一个已经连续多轮出现、仍未修复的确定性 UI/数据问题：

5. **`pitch_notes` 由 HashMap 无序迭代生成，详细模式又直接按 Vec 顺序 `→` 输出，因此箭头顺序不保证按时间。**

所以当前状态最准确的描述不是：

> Round 3 complete

而应该是：

> **Round 3 的接口/骨架与 UniDic reading 部分已落地，但 GAME、Forced Alignment、canonical note track、真实 matcher 和端到端验收尚未完成。**

---

# 1. P0：GAME 当前是“伪实现”，不是 ONNX 推理

文件：

```text
src-tauri/src/note_engine/game.rs
```

当前 `GameModelPaths` 只记录：

```rust
config_path
encoder_path
estimator_path
```

但官方 GAME 是三阶段架构：

```text
Encoder
  ↓
Segmenter
  ↓ boundaries
Estimator
  ↓
pitch / presence
```

官方 OpenVPI `dataset-tools` 的 GAME ONNX 运行目录目前要求：

```text
config.json
encoder.onnx
segmenter.onnx
bd2dur.onnx
dur2bd.onnx
estimator.onnx
```

参考：

- `https://github.com/openvpi/GAME`
- `https://github.com/openvpi/GAME/blob/main/ALGORITHMS.md`
- `https://github.com/openvpi/dataset-tools`

而当前工程的：

```rust
GameNoteEngine::try_load()
```

只真正创建了：

```rust
encoder_session
```

更关键的是：

```rust
fn transcribe(...)
```

**根本没有调用 ONNX session。**

现在的实际行为是：

```rust
let dur = audio.len() as f32 / sample_rate as f32;

if dur > 0.1 {
    events.push(MusicalNoteEvent {
        start: 0.0,
        end: dur,
        midi_float: 60.0,
        midi_rounded: 60,
        note_name: midi_to_note_name(60.0),
        confidence: 0.9,
        source: MusicalNoteSource::Game,
        ...
    });
}
```

也就是说：

> **任意长度 > 0.1 s 的输入音频，都会被“GAME”转录成一个覆盖整段音频的 C4。**

这是 Round 3 任务书里“禁止伪完成”条款所禁止的典型情况：

- enum 有了；
- trait 有了；
- class 有了；
- session 字段有了；
- 甚至输出 source 还写着 `Game`；
- 但模型实际上没有参与推理。

### 这一项必须按 P0 blocker 处理

在它真正修完之前：

```text
MusicalNoteSource::Game
```

绝对不能出现在这种占位输出上。

如果 GAME 运行失败，应明确：

```text
GameUnavailable
```

然后退回：

```text
LegacyFcpeTracker
```

而不是产生“来源标记为 GAME 的 C4 假结果”。

---

# 2. P0：运行时主链路根本没有使用 GAME

文件：

```text
src-tauri/src/analyzer.rs
```

当前实际执行：

```text
audio
 ↓
16 kHz mono
 ↓
Mel
 ↓
FCPE ONNX
 ↓
F0 / confidence
 ↓
DSP
 ↓
note_tracker::build_note_events(...)
 ↓
PitchTrack.note_events
```

关键代码仍然是：

```rust
let note_events =
    note_tracker::build_note_events(&times, &midis, &confidences, &config.note_tracking);
```

然后：

```rust
Ok(PitchTrack {
    times,
    frequencies,
    midis,
    confidences,
    rms,
    note_events,
    flux,
})
```

这意味着：

## 当前默认 badge 的 canonical musical note source 仍然是旧 NoteTracker

并非 GAME。

---

## `src-tauri/src/lib.rs` 也没有接通 GAME

当前 `analyze_audio()`：

```rust
analyzer.analyze(&audio_path, &config, ...)
```

得到的就是上面的 FCPE `PitchTrack`。

随后：

```rust
align_token_times(...)
rebind_lyrics(...)
```

整个 orchestration 中没有：

```rust
GameNoteEngine
```

也没有：

```rust
MusicalNoteEngine::transcribe(...)
```

所以 `note_engine/game.rs` 目前即使存在，也只是一个**没有进入生产路径的孤立模块**。

---

# 3. P0：Forced Alignment 仍然只有 trait，没有 backend

文件：

```text
src-tauri/src/forced_align/mod.rs
```

当前只有：

```rust
pub struct AlignedMora { ... }

pub struct LineAlignmentResult { ... }

pub trait ForcedAlignBackend {
    fn name(&self) -> &'static str;

    fn align_line(...) -> Result<LineAlignmentResult, String>;
}
```

没有：

```text
onnx backend
CTC backend
MMS backend
pyshiro bridge
SOFA backend
narabas backend
任何一个真实声学 backend
```

而 `models.rs` 里甚至仍明确写着：

```rust
AlignmentSource::ForcedAlign
// 音素级 forced alignment (P3, 尚未实现)
```

这已经足以说明：

> **Forced Alignment 尚未实现。**

---

# 4. P0：活跃歌词时间对齐仍然是旧 MoraDP

文件：

```text
src-tauri/src/lyrics.rs
```

当前：

```rust
pub fn align_token_times(...)
```

核心逻辑仍然是：

```rust
if dp_align_line(line, track, start, end, params) {
    ...
} else {
    distribute_line_times(...)
}
```

也就是：

```text
Enhanced LRC（若已有）
否则
RMS / voicing / spectral flux MoraDP
否则
weighted fallback
```

没有：

```text
ForcedAlignBackend
```

参与。

这与 Round 3 目标：

```text
EnhancedLRC
  >
ForcedAlign
  >
MoraDP
  >
WeightedFallback
```

并不一致。

目前真实优先级其实仍然是：

```text
EnhancedLRC
  >
MoraDP
  >
WeightedFallback
```

---

# 5. P0：发现一个新的严重日语 mapping bug——mora 会重复进入 DP

这是本轮审计里最值得优先修的一个算法 bug。

## 5.1 当前 UniDic 层的设计本身是合理的

`build_japanese_layers()` 对词典读音采用：

```rust
let exact = phonetic == span.surface;

if exact {
    // kana 可精确对应 surface char
} else {
    // 词典 reading 与 surface 不同
    (span.char_start, span.char_end)
}
```

例如：

```text
surface = 二人
reading = ふたり
```

由于无法诚实地声称：

```text
二 = ふ？
人 = たり？
```

所以每个 mora 都继承完整 coarse span：

```text
ふ : char 0..2
た : char 0..2
り : char 0..2
```

**这一步本身是正确的。**

它避免了伪造“逐汉字唯一读音映射”。

---

## 5.2 但 `build_align_units()` 随后把这个正确的 coarse span 用错了

当前实现：

```rust
for (ti, token) in line.tokens.iter().enumerate() {
    let token_moras = line
        .moras
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            token.char_start < m.char_end &&
            m.char_start < token.char_end
        })
        .map(|(mi, _)| mi)
        .collect();

    for mi in token_moras {
        units.push(AlignUnit {
            token_idx: ti,
            mora_idx: Some(mi),
            ...
        });
    }
}
```

问题来了。

假设显示 tokenizer 将：

```text
二人
```

拆成：

```text
二  token span 0..1
人  token span 1..2
```

而 UniDic 得到三个 mora：

```text
ふ 0..2
た 0..2
り 0..2
```

那么：

### token `二`

与三个 mora 都相交：

```text
ふ
た
り
```

加入 3 个 AlignUnit。

### token `人`

也与三个 mora 都相交：

```text
ふ
た
り
```

又加入 3 个 AlignUnit。

最终：

```text
真实 mora 数 = 3
DP unit 数    = 6
```

同一组 mora 被重复对齐了两遍。

---

## 5.3 对 `流れる` 更危险

如果 UniDic 把某一 span 分析为：

```text
surface = 流れる
reading = ながれる
```

并且这些 mora 都继承整个 surface span，那么当前按 display token overlap 展开时，很可能产生：

```text
流 -> な が れ る
れ -> な が れ る
る -> な が れ る
```

真实：

```text
4 mora
```

可能膨胀为：

```text
12 AlignUnit
```

这会直接扭曲：

- duration prior；
- DP 边界数量；
- token start/end；
- 后续 mora-note binding；
- “某个字为什么空音高”；
- “为什么一个字抢了隔壁字的时间”。

---

# 6. P0 修复原则：Alignment Unit 必须以 mora 为主体，只能出现一次

禁止继续：

```text
Display token
  ↓ overlap
把 mora 复制进 token
```

正确方向：

```text
ReadingSpan
  ↓
Mora sequence
  ↓
每个 mora 作为唯一 AlignUnit
  ↓
Forced Alignment / MoraDP
  ↓
mora 获得唯一时间
  ↓
再把时间/音符聚合回 display layer
```

例如：

```rust
struct AlignUnit {
    mora_idx: Option<usize>,
    reading_span_id: Option<usize>,
    fallback_surface_span: Option<(usize, usize)>,
    weight: f32,
    known: bool,
}
```

### 已知 reading 的部分

直接：

```rust
for (mi, mora) in line.moras.iter().enumerate() {
    units.push(AlignUnit {
        mora_idx: Some(mi),
        reading_span_id: Some(mora.reading_span_id),
        ...
    });
}
```

每个：

```text
mora_idx
```

在整行 unit sequence 中只能出现一次。

---

## 6.1 display glyph 不一定能拥有唯一 mora，这是语言事实，不要强行伪造

例如：

```text
二人 → ふたり
今日 → きょう
大人 → おとな
```

很多情况下不存在可靠的：

```text
每个汉字 ↔ 某几个 mora
```

唯一划分。

因此建议将数据关系明确为：

```text
DisplayToken
    ↕
ReadingSpan
    ↕
Mora[]
```

而不是：

```text
DisplayToken
    ↕
Mora[]
```

---

## 6.2 对 UI 有三个可接受方案

### A. 推荐：ReadingSpan 作为 timing group

例如：

```text
二 人
└───┘
 ふたり
```

`二`、`人` 在显示上仍是两个字，但：

```text
reading/timing group
```

是整个 `二人`。

音符 badge 可以：

- 显示在 span 上方；
- 或在 span 宽度中央；
- 或由 span 的 mora timing 再做视觉分布。

这是语言上最诚实的方案。

### B. 可选：对 span 内 display chars 做后处理视觉分配

仅在：

```text
mora timing 已经得到以后
```

再使用 heuristic 把 span 时间视觉分到各 glyph。

这个 heuristic：

> 只能影响 UI highlight 布局，不能重新进入 acoustic alignment。

### C. 高级方案：surface-reading alignment

以后可以专门做：

```text
orthography ↔ reading
```

对齐，例如结合：

- okurigana；
- kana 已知字符；
- UniDic morphology；
- 最小编辑/假名匹配；
- lexical rules。

但这不是 Round 3.1 必须完成的条件。

Round 3.1 优先保证：

> **不重复 mora、不扭曲声学时间。**

---

# 7. P0：canonical note 数据模型没有真正进入 PitchTrack

当前 `models.rs` 的：

```rust
PitchTrack
```

仍只有：

```rust
times
frequencies
confidences
midis
rms
note_events: Vec<NoteEvent>
flux
```

没有：

```rust
musical_notes: Vec<MusicalNoteEvent>
musical_note_source
game_model_version
```

因此整个旧 Binder 仍围绕：

```rust
pitch_track.note_events
```

工作。

---

# 8. P0：Binder 仍绑定旧 NoteEvent，不是 GAME MusicalNoteEvent

`bind_pitch_to_tokens()`：

```rust
let has_events = !pitch_track.note_events.is_empty();
```

mora binding：

```rust
admit_events_for_window(&pitch_track.note_events, ...)
```

token binding：

```rust
admit_events_for_window(&pitch_track.note_events, ...)
```

所以：

> **即使未来单独调用了 `GameNoteEngine`，只要 GAME 输出不进入 canonical track，歌词 badge 仍然不会使用它。**

Round 3 要求的：

```text
GAME musical notes
       +
aligned mora timing
       ↓
monotonic mora-note matcher
```

目前没有成为生产主链。

---

# 9. P0：详细音高箭头仍可能乱序

当前 mora 聚合阶段：

```rust
let mut event_best: HashMap<usize, f32> = HashMap::new();
```

随后：

```rust
for (idx, total_ov) in event_best {
    notes.push(...)
}
```

`HashMap` 的迭代顺序不保证时间顺序。

后面虽然：

```rust
order.sort_by(...)
```

但这个 `order` 只用来选择：

```text
primary
```

并没有用于重新排序：

```rust
token.pitch_notes
```

最终：

```rust
token.pitch_notes = notes;
```

前端则：

```ts
token.pitch_notes
  .map(...)
  .join("→")
```

直接显示。

所以：

```text
A#3→C4→D4
```

不能保证真的是时间顺序。

---

## 修复

所有展示前的 `pitch_notes` 至少：

```rust
notes.sort_by(|a, b| {
    a.start_time
        .partial_cmp(&b.start_time)
        .unwrap_or(std::cmp::Ordering::Equal)
});
```

注意 primary index 会因排序改变，所以：

- 要么排序后重新寻找 primary；
- 要么保存 primary 的稳定 ID；
- 不要依赖旧 Vec index。

最好给 musical note 有稳定：

```text
event_id
```

---

# 10. P0：现有“全量测试通过”并不能证明 Round 3 完成

这是本轮审计里第二个非常重要的问题。

## 10.1 `note_engine_test.rs` 根本没测试 GAME

当前文件只测试：

```rust
LegacyFcpeNoteTracker
```

包含：

- stable A4；
- vibrato 不应碎片化。

但没有：

```rust
GameNoteEngine
```

更没有：

```text
真实 GAME ONNX fixture
```

因此：

> `cargo test` 通过，与 GAME 是否能推理没有关系。

---

## 10.2 UniDic reading tests 是真的，但测试范围太窄

这部分值得保留。

当前确实测试：

```text
愛   → あ・い
二人 → ふ・た・り
心   → こ・こ・ろ
...
```

说明：

```text
dictionary reading layer
```

确实比上一轮前进了。

但是这些测试只证明：

```text
文本 → reading/mora
```

没有验证：

```text
reading span
 ↓
display tokens
 ↓
AlignUnit
```

所以前述 `二人 = 3 mora → 6 AlignUnit` 的重复 bug 完全可以在 reading tests 全绿的情况下存在。

---

## 10.3 real song acceptance 被 `#[ignore]` 掉了

当前：

```rust
#[test]
#[ignore]
fn real_songs_acceptance()
```

并且缺模型时：

```rust
eprintln!("Skipping ...");
return;
```

没歌曲时：

```rust
return;
```

所以正常的：

```bash
cargo test
```

根本不会跑它。

---

## 10.4 `real_lrc_full_chain` 同样被 ignore

而且默认 LRC 路径还是开发机器的：

```text
C:\Users\tp798\Documents\Lyrics\岡村孝子 - ドラマ (636813).lrc
```

如果不存在：

```text
Skipping
return
```

因此它也不是可重复的 CI acceptance test。

---

## 10.5 real_song_test 当前走的仍是 FCPE pipeline

其：

```rust
run_pipeline()
```

调用：

```rust
PitchAnalyzer::analyze(...)
```

而当前 `PitchAnalyzer` 是：

```text
FCPE → NoteTracker
```

并不是 GAME。

所以即使手动：

```bash
cargo test -- --ignored
```

让 real song 测试跑起来，也不能证明 GAME 主链完成。

---

# 11. P1：GitHub CI 目前没有 Round 3 correctness gate

当前 `.github/workflows/publish.yml`：

- tag 触发；
- Linux / Windows；
- 安装 Node / Rust；
- 下载 ONNX Runtime；
- 直接 build / package / release。

没有看到：

```text
cargo test
pnpm test
GAME fixture
Forced Alignment fixture
Round3 acceptance
```

作为发布门禁。

因此：

> “本地所有测试通过”与“GitHub 上持续保证主链正确”是两回事。

Round 3.1 应增加：

```text
.github/workflows/ci.yml
```

至少在 push/PR 上跑：

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm install --frozen-lockfile
pnpm build
```

另设一个：

```text
round3-integration
```

job，获取 pinned model/fixture 后跑真实 GAME / FA integration。

---

# 12. GAME 的正确实施路线

## 12.1 不要自己猜 ONNX workflow

OpenVPI GAME 官方 README 明确：

> 官方仓库支持 ONNX export，但不直接提供 ONNX inference implementation。

所以 Rust 侧实现时应优先参考：

```text
openvpi/dataset-tools
```

其 GAME Model 已有真实多 ONNX 推理工程。

当前官方模型目录包括：

```text
config.json
encoder.onnx
segmenter.onnx
bd2dur.onnx
dur2bd.onnx
estimator.onnx
```

具体以你选择的官方 v1.0.3 ONNX asset 为准，不要写死未经验证的输入输出 tensor shape。

---

# 13. GAME Phase 必须实现的内容

建议内部结构：

```rust
pub struct GameModelPaths {
    pub dir: PathBuf,
    pub config_path: PathBuf,
    pub encoder_path: PathBuf,
    pub segmenter_path: PathBuf,
    pub estimator_path: PathBuf,
    pub bd2dur_path: Option<PathBuf>,
    pub dur2bd_path: Option<PathBuf>,
}
```

然后：

```rust
pub struct GameNoteEngine {
    config: GameConfig,
    encoder: Mutex<Session>,
    segmenter: Mutex<Session>,
    estimator: Mutex<Session>,
    bd2dur: Option<Mutex<Session>>,
    dur2bd: Option<Mutex<Session>>,
}
```

但具体是否必须加载 `bd2dur/dur2bd`，以官方当前 ONNX workflow 为准。

---

## 13.1 `transcribe()` 必须真实执行

不得存在：

```rust
midi_float: 60.0
confidence: 0.9
```

这种固定输出。

最低实现应真实完成：

```text
audio
 ↓
GAME required preprocessing
 ↓
encoder
 ↓
segmenter
 ↓
boundary decode / diffusion sampling
 ↓
region map
 ↓
estimator
 ↓
pitch + presence
 ↓
MusicalNoteEvent[]
```

---

## 13.2 必须支持 GAME 的两种关键工作模式

### Mode A：raw singing transcription

```rust
transcribe(audio, sr, None)
```

GAME 自己预测 note boundaries。

用于：

```text
没有歌词
中文歌
普通音高分析
```

### Mode B：external boundaries / alignment-assisted

当 forced alignment 已经给出：

```text
word / mora / phonetic boundaries
```

时，应评估官方 GAME estimator 的 external boundary 模式。

这是 GAME 官方架构的重要能力。

---

# 14. GAME 输出必须与 FCPE 完全分层

最终应明确：

```text
FCPE
 ↓
ContinuousPitchTrack
```

负责：

- 青色连续 F0 曲线；
- cents；
- vibrato/glide 可视化；
- 实际演唱音准细节。

而：

```text
GAME
 ↓
MusicalNoteTrack
```

负责：

- 离散音乐 note；
- onset；
- offset；
- MIDI pitch；
- slur / presence；
- badge；
- ASS/SRT musical note annotation。

禁止再让：

```text
FCPE NoteTracker
```

冒充默认 canonical musical-note engine。

旧 NoteTracker 只允许：

```text
fallback/debug/legacy
```

---

# 15. 推荐的数据模型修改

建议不要继续把不同语义塞进一个：

```rust
PitchTrack.note_events
```

可以：

```rust
pub struct PitchTrack {
    // FCPE continuous track
    pub times: Vec<f32>,
    pub frequencies: Vec<f32>,
    pub confidences: Vec<f32>,
    pub midis: Vec<f32>,
    pub rms: Vec<f32>,
    pub flux: Vec<f32>,

    // canonical musical note result
    #[serde(default)]
    pub musical_notes: Vec<MusicalNoteEvent>,

    #[serde(default)]
    pub musical_note_source: MusicalNoteSource,

    #[serde(default)]
    pub musical_note_model: Option<String>,

    // backward compatibility / debug only
    #[serde(default)]
    pub legacy_note_events: Vec<NoteEvent>,
}
```

也可以抽成：

```rust
pub struct AnalysisResult {
    pub continuous_pitch: ContinuousPitchTrack,
    pub musical_notes: MusicalNoteTrack,
}
```

后者语义更干净，但迁移改动更大。

开发 Agent 可自行选择，但必须满足：

> **生产 Binder 不能再把 legacy NoteEvent 当 canonical GAME notes。**

---

# 16. GAME fallback 规则必须明确

推荐：

```text
GAME model available + inference success
    ↓
MusicalNoteSource::Game

GAME unavailable / fails
    ↓
LegacyFcpeNoteTracker
    ↓
MusicalNoteSource::LegacyFcpeTracker
```

UI / debug / export 必须能够看见：

```text
note_source = GAME
```

还是：

```text
note_source = LegacyFcpeTracker
```

不允许静默 fallback 后仍显示成 GAME。

---

# 17. Forced Alignment：Round 3.1 至少必须有一个真实 backend

不再接受：

```rust
trait ForcedAlignBackend
```

本身作为“完成”。

开发 Agent 可以在以下路线中自行选择最稳定的一条：

## 方案 A：独立 helper backend

先用 Python / 子进程把真实 FA 跑通：

```text
Rust/Tauri
  ↓ JSON / temp wav / stdin
helper process
  ↓
aligned mora/phoneme timing JSON
```

优点：

- 最快验证算法；
- 可以直接与成熟 Python implementation 对照；
- 避免一开始就在 Rust 重造 preprocessing/CTC decode。

等准确性通过后再考虑 native ONNX。

## 方案 B：native ONNX CTC aligner

若模型 license、tokenizer、runtime 都合适，可以直接：

```text
audio
 ↓
speech encoder / CTC
 ↓
known phoneme/kana targets
 ↓
forced alignment DP
 ↓
phoneme/mora times
```

但必须有：

```text
真实模型 inference
```

不是 feature scaffold。

---

# 18. 不建议把 GPL 代码直接揉进核心

`pyshiro` 仍然很适合作为：

```text
Japanese singing alignment oracle / benchmark
```

但若当前项目希望保持更宽松许可，不应直接复制其 GPLv3 源码到核心。

可以：

- 本地 benchmark；
- 开发期 oracle；
- 对同一 wav + lyrics 比较边界；
- 不把 GPL implementation 分发进主程序。

---

# 19. Forced Alignment 的生产调用点

当前：

```rust
load_lyrics_lrc()
```

只会：

```text
parse
→ MoraDP
```

应修改 orchestration。

推荐分析顺序：

```text
1. audio loaded
2. FCPE continuous F0
3. GAME musical notes
4. LRC parse
5. UniDic reading/mora/phoneme
6. for each LRC line:
       EnhancedLRC ?
         yes -> use anchors
         no  -> real ForcedAlign
                  success -> ForcedAlign
                  fail    -> MoraDP
7. mora-note matching
8. display aggregation
```

如果用户先加载歌词、后加载音频：

```text
audio analysis finish
→ invalidate auto timing
→ rerun FA
→ rematch
```

如果先音频、后歌词：

```text
load lyrics
→ run FA immediately
→ rematch
```

---

# 20. Forced Alignment 的 acceptance 定义

一个 backend 只有同时满足以下条件才允许汇报：

```text
Forced Alignment implemented
```

- 实际加载一个声学模型；
- 输入真实音频；
- 输入已知 reading/phoneme sequence；
- 输出逐 mora/phoneme timing；
- timing 单调；
- 行内不越界；
- `AlignmentSource::ForcedAlign` 真正出现在运行结果；
- Debug JSON 能看到 FA model/backend 名称；
- 至少有一个固定 fixture 与人工/外部 oracle timing 对比；
- MoraDP 只在 FA 不可用/失败时触发。

---

# 21. 实现真正的 mora ↔ musical-note matcher

当前 Binder 本质仍然是：

```text
time overlap
```

针对旧 NoteEvent 的 many-to-many 聚合。

Round 3 的目标应该明确为：

```text
AlignedMora sequence
+
GAME MusicalNoteEvent sequence
↓
monotonic matching
```

---

## 21.1 必须保留的音乐语义

允许：

```text
1 mora → 1 note
```

普通情况。

允许：

```text
1 mora → N notes
```

真正 melisma / 转音。

允许：

```text
N mora → 1 held note
```

连唱、快速 syllable movement、同一音高跨多个 mora。

---

## 21.2 基本约束

matcher 至少使用：

- 时间 overlap；
- onset proximity；
- mora interval；
- note interval；
- monotonicity；
- GAME `is_slur`；
- note confidence/presence；
- FA confidence。

禁止：

```text
因为 MIDI pitch 变化，所以歌词边界一定变化
```

歌词与音乐 note 是两种独立 sequence。

---

# 22. Display aggregation：不要再把“一个汉字”当 acoustic primitive

目标层级：

```text
LyricLine
 ├─ DisplayToken[]
 ├─ ReadingSpan[]
 │    └─ Mora[]
 └─ Alignment
```

建议 Badge 首先绑定到：

```text
ReadingSpan / mora group
```

再投影到 UI。

对于简单映射：

```text
愛 → あ・い
```

一个显示 token 上可显示：

```text
D4
```

或：

```text
D4→E4
```

如果两 mora 对应不同 note。

对于：

```text
二人 → ふ・た・り
```

不要让每个汉字各复制 3 mora。

---

# 23. P0 回归测试：必须专门钉死“mora duplication”

新增测试，测试不只是：

```text
二人 reading == ふたり
```

而是测试内部 alignment sequence。

例如提供：

```rust
#[test]
fn japanese_align_units_do_not_duplicate_moras() {
    let line = parse_line("二人");

    assert_eq!(line.moras.len(), 3);

    let units = build_align_units_for_test(&line);

    let ids: Vec<_> = units
        .iter()
        .filter_map(|u| u.mora_idx)
        .collect();

    assert_eq!(ids, vec![0,1,2]);
}
```

还要：

```text
愛
心
二人
流れる
群れ
心を引き裂く嵐の中
流れる人の群れ
知らない二人にもどるのね
```

断言：

```text
每个 mora_idx 在 acoustic unit sequence 中出现且只出现一次
```

---

# 24. P0 GAME integration test

必须有一个**真实模型 fixture 测试**。

不能测试：

```text
GameNoteEngine::name()
```

不能只测试：

```text
GameModelPaths
```

必须测试：

```text
wav
 ↓
actual ONNX GAME
 ↓
MusicalNoteEvent[]
```

最低断言：

```text
events.len() > 1
not all C4
time monotonic
0 <= start < end <= audio duration
source == Game
pitch range physically plausible
```

再与官方 Python / dataset-tools 输出 fixture 比较：

```text
note count difference <= tolerance
onset MAE <= tolerance
pitch agreement >= tolerance
```

阈值可以先通过样例定标，但必须写进 test。

---

# 25. P0：必须专门写“反 C4 占位实现”测试

这个测试非常便宜，却能防止这轮同样的事情再次发生。

例如用两个明显不同的 synthetic melody / fixture：

```text
fixture A：A3 / C4 / E4
fixture B：A4 / G4 / E4
```

若 GAME 输出：

```text
[whole-file C4]
```

测试必须直接失败。

---

# 26. P0 Forced Alignment integration test

准备一个很短、可合法分发/自行生成的日语人声 fixture。

例如：

```text
audio: 2~5 s
known text: あい
reading: あい
expected rough mora boundaries:
  あ: ...
  い: ...
```

或项目自己的测试录音。

必须：

```text
backend loads
model actually runs
AlignmentSource == ForcedAlign
mora times monotonic
```

核心 acceptance job 中：

> 模型缺失应 FAIL，而不是 `Skipping; return`。

---

# 27. P0 End-to-End Round 3 acceptance fixture

需要至少一条真正从头走到尾：

```text
audio
 ↓
FCPE
 ↓
GAME
 ↓
LRC
 ↓
UniDic
 ↓
Forced Alignment
 ↓
Matcher
 ↓
UI/export model
```

最后明确断言：

```text
note source == GAME
alignment source == ForcedAlign
reading source == UniDic
musical notes non-empty
mora timings monotonic
pitch_notes chronological
no duplicate mora alignment units
```

---

# 28. `#[ignore]` 的正确使用方式

大型真实歌曲测试可以继续：

```rust
#[ignore]
```

因为模型和音频很大。

但：

> 不能把被 ignore 的测试称为“全量自动验收已经通过”。

建议拆成：

```text
Unit tests
Integration tests
Round3 acceptance tests
Manual corpus tests
```

并在 README/CI 中写清楚。

例如：

```bash
cargo test
cargo test --test round3_game_integration -- --ignored
cargo test --test round3_fa_integration -- --ignored
cargo test --test round3_e2e -- --ignored
```

然后 CI 的专用 integration job **显式运行这些 ignored tests**。

---

# 29. 禁止“缺模型就 return = pass”

当前真实歌曲测试大量使用：

```rust
if !model.exists() {
    eprintln!("Skipping");
    return;
}
```

这对：

```text
developer convenience
```

可以接受。

但对：

```text
acceptance gate
```

不接受。

应有两个层级：

### Dev smoke

缺资源可 skip。

### Release/Round3 acceptance

通过环境变量：

```text
ROUND3_ACCEPTANCE=1
```

或独立 test target。

此时：

```text
模型缺失 = test failure
fixture 缺失 = test failure
FA backend 未初始化 = test failure
GAME fallback 到 legacy = test failure
```

---

# 30. UI 语义仍需收口

当前 `KaraokeDisplay` 还是：

```text
detailedPitch = false/true
```

详细模式直接：

```ts
token.pitch_notes.map(...).join("→")
```

这意味着它还没有明确区分：

```text
canonical GAME notes
legacy NoteTracker notes
raw F0/debug
```

Round 3.1 不必做大 UI，但至少保证：

## 普通模式

显示：

```text
canonical primary musical note
```

## 详细模式

显示：

```text
canonical matched GAME notes
```

顺序：

```text
start_time ascending
```

## Debug

必须可见：

```text
continuous pitch engine = FCPE
musical note engine = GAME / Legacy fallback
reading engine = UniDic / Kana fallback
alignment engine = EnhancedLRC / ForcedAlign / MoraDP / Weighted
matcher = ...
```

---

# 31. Debug export 也应升级

当前 Debug comment 还是：

```text
FCPE / NoteTracker / Reading / Alignment / Binder
```

Round 3 后应改为：

```text
FCPE
GAME
UniDic
ForcedAlign
Matcher
DisplayAggregation
```

输出至少：

```json
{
  "continuous_pitch": {},
  "musical_note_engine": {
    "source": "GAME",
    "model": "...",
    "notes": []
  },
  "reading": {
    "provider": "UniDic",
    "spans": [],
    "moras": []
  },
  "alignment": {
    "backend": "...",
    "source": "ForcedAlign",
    "moras": []
  },
  "matching": {},
  "display_tokens": []
}
```

看到一个“れ没有音高”时，应能一次定位到是哪一级出了问题。

---

# 32. CI 建议

新增：

```text
.github/workflows/ci.yml
```

## Job 1：fast

每个 push/PR：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm install --frozen-lockfile
pnpm build
```

## Job 2：round3-integration

可用：

- manual dispatch；
- release tag；
- main push；
- 或定时。

步骤：

```text
download pinned GAME model
verify SHA256
download/build FA model
verify SHA256
prepare tiny fixtures
run GAME integration
run FA integration
run Round3 E2E
```

---

# 33. 不要重写已经做对的 UniDic provider

这次审计确认 UniDic 部分是真进展。

当前：

```rust
lindera = { version = "6.0", features = ["embed-unidic"] }
```

并且：

```rust
resolve_embedded_loader(DictionaryKind::UniDic)
```

真实初始化 dictionary。

所以不要再回退到：

```text
KanaOnly
```

或把整个 provider 推翻。

Round 3.1 应修的是：

```text
reading/mora → alignment unit
```

映射层。

---

# 34. embedded UniDic 的产品取舍

当前使用：

```text
embed-unidic
```

可以工作。

它可能明显增加：

- 编译时间；
- binary/resource size。

这属于产品工程取舍，不是本轮 blocker。

本轮优先级：

```text
正确性 > 体积优化
```

后续再决定是否改为：

```text
external Japanese accuracy pack
```

即可。

不要为了缩体积拖慢 Round 3.1。

---

# 35. 不要删除 FCPE / NoteTracker

Round 3.1 不是“GAME 替换掉一切”。

应保留：

```text
FCPE
```

因为它对连续 F0 曲线仍有明确用途。

旧：

```text
NoteTracker v2
```

也应保留为：

```text
LegacyFcpeTracker
```

用途：

- GAME 模型缺失 fallback；
- benchmark；
- debug；
- 回归对照；
- 低资源运行。

但默认 badge source 有 GAME 时必须是 GAME。

---

# 36. 不要继续调这些参数当主修复

在主链接通前，不要再把大量时间花在：

```text
40 cents
70 ms
1.8 汉字权重
spectral flux coefficient
overlap 0.12
```

这些只属于：

```text
legacy fallback tuning
```

无法替代：

```text
GAME
Forced Alignment
mora-note matcher
```

---

# 37. Round 3.1 建议实施顺序

不要并行大改，按下面顺序提交，降低回归风险。

---

## Commit 1 — correctness hotfixes

内容：

- 修 `pitch_notes` chronological sorting；
- 新增 mora duplication regression test；
- 修 `build_align_units()`，保证每个 known mora 只出现一次；
- 给 alignment unit 暴露测试辅助函数或 module-private test；
- 保留当前 UI 不大改。

验收：

```text
二人 = 3 moras → exactly 3 known mora units
愛   = 2 → 2
心   = 3 → 3
流れる -> no duplicated mora_idx
```

---

## Commit 2 — canonical musical-note model

内容：

- 将 `MusicalNoteEvent` 纳入正式 analysis result；
- 添加 note engine source metadata；
- 保留 legacy NoteEvent 的 serde 兼容；
- Binder/matcher API 开始接受 canonical notes；
- 不再把 legacy `PitchTrack.note_events` 当唯一事实源。

验收：

- 工程旧项目仍可加载；
- GAME/legacy source 能明确区分；
- Debug export 显示 source。

---

## Commit 3 — real GAME ONNX inference

内容：

- 以 OpenVPI 官方 GAME + dataset-tools 为 reference；
- 正确加载所需 ONNX sessions；
- 正确 preprocessing；
- 正确 boundary inference；
- 正确 pitch/presence decode；
- optional external boundaries；
- 删除固定 C4 placeholder。

验收：

- fixture 真实跑出多个 note；
- 与官方 reference 对齐；
- runtime source = Game；
- 无模型时明确 fallback。

---

## Commit 4 — GAME orchestration

内容：

把生产：

```text
analyze_audio()
```

真正接到：

```text
FCPE + GAME
```

而不是只让 `game.rs` 孤立存在。

验收：

Debug：

```text
continuous = FCPE
notes = GAME
```

---

## Commit 5 — real Forced Alignment backend

内容：

- 至少一个真实 backend；
- 模型真实 inference；
- 行级 LRC constraint；
- reading/mora/phoneme target；
- 输出 aligned moras；
- 失败才 fallback MoraDP。

验收：

```text
AlignmentSource::ForcedAlign
```

真正出现在真实运行结果中。

---

## Commit 6 — monotonic mora-note matcher

内容：

```text
AlignedMora[]
+
MusicalNoteEvent[]
→
bindings
```

支持：

```text
1→1
1→N melisma
N→1 held note
```

验收：

- synthetic；
- Japanese fixture；
- notes chronological。

---

## Commit 7 — UI semantics/debug

内容：

- compact canonical note；
- detailed canonical notes；
- debug source badges；
- no HashMap order leakage；
- legacy fallback 可见。

---

## Commit 8 — Round 3 acceptance + CI

内容：

- GAME real integration；
- FA real integration；
- Japanese end-to-end；
- no silent skip in acceptance；
- CI workflow；
- README 更正。

---

# 38. Round 3.1 最小验收矩阵

| 项目 | 必须状态 |
|---|---|
| FCPE 连续 F0 | ✅ |
| UniDic 汉字真实 reading | ✅ |
| mora 从 reading 展开 | ✅ |
| known mora 在 align sequence 中唯一 | ✅ |
| GAME 模型真实 ONNX inference | ✅ |
| GAME 不是固定 C4 stub | ✅ |
| 默认 canonical note source = GAME | ✅ |
| GAME 失败可见地 fallback legacy | ✅ |
| ForcedAlign 至少一个真实 backend | ✅ |
| 生产调用 ForcedAlign | ✅ |
| MoraDP 仅 fallback | ✅ |
| matcher 输入 GAME notes | ✅ |
| 1 mora → N notes | ✅ |
| N mora → 1 note | ✅ |
| detailed notes 时间排序 | ✅ |
| Debug 显示各层 source | ✅ |
| GAME integration test | ✅ |
| FA integration test | ✅ |
| Japanese E2E fixture | ✅ |
| acceptance 不 silent-skip | ✅ |
| CI correctness gate | ✅ |

只要其中粗体核心链：

```text
GAME real inference
ForcedAlign real backend
production integration
```

有一项没有完成，就不要写：

> Round 3 complete

---

# 39. 建议固定的日语 regression corpus

这几句直接保留，别每轮换：

```text
愛
二人
心
嵐
流れる
群れ
知らない二人にもどるのね
心を引き裂く嵐の中
流れる人の群れ
```

测试层级：

### Reading

```text
surface → reading → mora
```

### Structural

```text
mora count
unique mora indices
reading span membership
```

### Acoustic

```text
forced-aligned mora timing
```

### Musical

```text
GAME notes
```

### Matching

```text
mora ↔ notes
```

### Display

```text
chronological badge
```

这样一个 fixture 错了，可以马上定位到底是哪一层。

---

# 40. 中文“真转音”也要留一个 regression

用户此前给出的目标语义是类似：

```text
“我”一个字
```

可能对应：

```text
G4 → A4 → G4
```

真正 melisma。

应该做 synthetic 或自有短录音 fixture：

```text
1 lyric token
3 stable GAME notes
```

断言：

```text
token.pitch_notes.len() == 3
chronological
primary note chosen by policy
```

同时另一个：

```text
1 lyric token
A4 vibrato
```

GAME 应最好给：

```text
1 canonical note
```

FCPE 则保留连续 vibrato contour。

这正好验证 Round 3 的“双轨真值”设计。

---

# 41. Developer Agent 的执行规则

这一节请严格遵守。

## 41.1 先读当前 HEAD，不要照任务书盲改

本文件基于 2026-08-30 GitHub `main` 审计。

开始前：

```bash
git status
git log -5 --oneline
git pull --ff-only
```

确认本地与 remote 一致。

---

## 41.2 不要推翻已有正确功能

保留：

- FCPE；
- UniDic；
- reading regressions；
- NoteTracker v2 fallback；
- export；
- current UI style；
- project serialization compatibility；
- current lyric parsing；
- Enhanced LRC priority。

---

## 41.3 不再使用“接口存在 = 完成”

以下都不算完成：

```text
trait 有了
enum 有了
module 有了
TODO 写了
mock 有了
placeholder 有了
load model path 有了
测试只验证 name()
```

必须：

```text
真实数据进入
真实模型执行
真实结果流经生产路径
integration test 验证
```

---

## 41.4 不要让用户继续当传话筒

遇到以下普通工程决策：

```text
文件怎么拆
函数怎么命名
trait 放哪里
测试 fixture 怎么组织
error enum 怎么定义
serde migration 怎么写
CI job 怎么拆
```

自行做合理决定。

只有遇到真正需要产品决策、且两种方案会明显改变用户体验或许可/分发模式时再询问。

---

## 41.5 每个 Phase 完成后自己继续

不要：

```text
“Commit 1 已完成，请确认是否继续 Commit 2”
```

直接一路执行到：

```text
Round 3.1 acceptance
```

全部完成。

---

# 42. 最终交付时必须给出的证据

开发 Agent 最终报告不要只写：

```text
所有测试通过。
```

必须至少附：

```text
git commit hash
```

以及：

### GAME

```text
model files loaded:
...
fixture:
...
predicted notes:
...
reference notes:
...
metrics:
...
```

### Forced Alignment

```text
backend:
model:
fixture:
alignment source:
mora timing sample:
...
```

### Japanese structural

```text
二人:
reading = ふたり
moras = 3
align units = 3
unique mora ids = [0,1,2]
```

### E2E

```text
note source = GAME
alignment source = ForcedAlign
fallback used = false
```

### Tests

列出真正执行的命令：

```bash
cargo test ...
cargo test ... --ignored
pnpm ...
```

以及：

```text
passed / failed / ignored
```

分别多少。

---

# 43. 最终审计判据

修完后再次审核时，我会重点查下面几个地方，而不是看 commit message：

```text
src-tauri/src/note_engine/game.rs
```

是否真实 session.run()。

```text
src-tauri/src/analyzer.rs / lib.rs
```

是否真的调用 GAME。

```text
src-tauri/src/forced_align/*
```

是否有 backend + inference。

```text
src-tauri/src/lyrics.rs
```

是否：

```text
ForcedAlign > MoraDP
```

以及 mora 是否不重复。

```text
models.rs
```

是否存在清晰的 canonical musical-note data。

```text
Binder / Matcher
```

是否真的使用 GAME notes。

```text
tests
```

是否包含非 mock 的 GAME/FA/E2E。

---

# 44. 本轮审计最终判断

## 已经真实完成的

```text
✅ Lindera/UniDic dependency
✅ real UniDic loading
✅ basic Japanese reading/mora regression
✅ note_engine abstraction skeleton
✅ forced-align abstraction skeleton
✅ legacy NoteTracker fallback
✅ part of debug/binder infrastructure
```

## 未完成 / 当前阻塞 Round 3 DoD 的

```text
❌ real GAME ONNX inference
❌ GAME production integration
❌ canonical GAME note track
❌ real Forced Alignment backend
❌ Forced Alignment production integration
❌ GAME-note ↔ aligned-mora production matcher
❌ no-duplication Japanese alignment mapping
❌ chronological detailed-note guarantee
❌ real GAME acceptance test
❌ real FA acceptance test
❌ non-skipping Round3 E2E acceptance
❌ CI correctness gate
```

---

# 45. 给下一位开发 Agent 的一句话

**不要再扩功能。把 Round 3 真正接通。**

当前最重要的不是继续给项目增加更多 abstraction，而是把这四条真实数据流跑通：

```text
FCPE audio → continuous F0
GAME audio → musical notes
UniDic lyrics → mora/phoneme → Forced Alignment timing
aligned mora + GAME notes → matcher → UI
```

当 Debug 输出能对一首真实日语歌明确打印：

```text
continuous_pitch_source = FCPE
musical_note_source = GAME
reading_source = UniDic
alignment_source = ForcedAlign
```

并且这些字段背后都是真的模型推理而不是 placeholder，Round 3 才算真正结束。
