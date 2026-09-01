# Pitch Analyzer Round 3.2：注音 LRC × 高可信歌词对齐 × 音符去冗余最终方案

> 仓库：`https://github.com/hydrogen1222/pitch-analyzer`  
> 审计基线：GitHub `main`，Round 3.1 最新提交 `6829488`  
> 本轮原则：**不要再继续靠调几个阈值修补。把“歌词读音真值”“声学时间真值”“音乐音符真值”“显示策略”彻底分层。**

---

# 0. 最终结论

用户提出的：

```text
冷(つめ)たく微笑(ほほえ)んだ
悲(かな)しい位(くらい)に見(み)つめてた
心(こころ)を引(ひ)き裂(さ)く嵐(あらし)の中(なか)
知(し)らない二人(ふたり)にもどるのね
```

这个思路**完全可行，而且建议正式支持为“高精度日语 LRC”格式**。

它应该被解释为：

```text
显示：
冷たく微笑んだ

读音真值：
冷   -> つめ
微笑 -> ほほえ
```

而不是把括号里的假名显示出来、参与分词或参与歌词字符计数。

Reading 优先级应明确为：

```text
User Ruby / Furigana
    >
Project Reading Override
    >
UniDic
    >
KanaOnly
    >
Heuristic
```

但是必须同时明确：

> **注音主要解决“歌词到底怎么读”和 forced alignment；它不能直接修复 GAME/音符识别错误。**

当前截图里的两种失败要分开处理：

```text
歌词错位
→ Ruby/UniDic + Forced Alignment

普通模式错误单音、详细模式过度碎片化
→ GAME reference parity + FCPE evidence fusion + note consolidation
```

---

# 1. 当前 Round 3.1 的真实状态

已经有：

```text
FCPE continuous F0          ✅
GAME real ONNX path         ✅
UniDic                      ✅
MMS Forced Align backend    ✅
MoraDP fallback             ✅
canonical musical notes     ✅
CI / acceptance scaffold    ✅
```

但是当前结果仍很差，剩余主矛盾是：

1. `漢字(かな)` 当前并没有正式 parser；
2. 多字符 reading span 仍不适合直接回填到“逐汉字 token”；
3. GAME Rust 实现还必须与官方 reference 做严格 parity；
4. GAME metadata 仍有半占位数据；
5. 详细模式仍是 `pitch_notes.map(...).join("→")`，本质是 raw event dump；
6. 普通模式仍缺少 GAME 与 FCPE 的交叉证据校验。

---

# 2. P0：正式支持 `漢字(かな)` Ruby-LRC

## 2.1 语法

支持：

```text
漢字(かな)
漢字列(かなかな)
```

例如：

```text
愛(あい)
心(こころ)
位(くらい)
二人(ふたり)
指先(ゆびさき)
背伸(せの)び
運命(さだめ)
```

不要要求用户写：

```text
二(ふ)人(たり)
```

因为这种逐汉字映射很多时候本身就是伪造的。

---

## 2.2 Ruby 识别规则

只有同时满足以下条件时把括号解释成注音：

- `(` 前紧邻一个包含汉字的日文 surface span；
- 括号内容非空；
- 内容只含平假名、片假名、长音 `ー`，必要时允许 `・`；
- 有闭合 `)`。

例如：

```text
愛(あい)      -> Ruby
二人(ふたり)  -> Ruby
```

而：

```text
(コーラス)
（笑）
(yeah)
```

默认按普通歌词文本处理，不解释成 Ruby。

建议同时支持 `\(`、`\)` 转义。

---

# 3. Ruby parsing 必须发生在 tokenization 之前

新增纯函数：

```rust
pub struct RubyAnnotation {
    pub surface: String,
    pub reading: String,
    pub display_start: usize,
    pub display_end: usize,
    pub raw_start: usize,
    pub raw_end: usize,
}

pub struct ParsedRubyText {
    pub raw_text: String,
    pub display_text: String,
    pub annotations: Vec<RubyAnnotation>,
}

fn parse_ruby_text(raw: &str) -> Result<ParsedRubyText, RubyParseError>;
```

示例：

```text
输入：
心(こころ)を引(ひ)き裂(さ)く

display_text：
心を引き裂く

annotations：
心 -> こころ
引 -> ひ
裂 -> さ
```

Ruby 内容不能进入 `display_text`。

---

# 4. 统一 char offset 坐标系

以后生产结构中的：

```text
LyricToken.char_start/end
ReadingSpan.char_start/end
MoraUnit.char_start/end
Enhanced-LRC anchor position
ReadingOverride
Debug token selection
```

全部使用：

```text
去掉 Ruby 标记之后的 display_text
```

作为唯一 canonical char coordinate。

禁止混用：

```text
raw annotated text char offset
display char offset
UTF-8 byte offset
```

---

# 5. parse_lrc 正确流程

```text
Raw LRC
 ↓
parse [mm:ss.xxx]
 ↓
parse enhanced-LRC timing tags
 ↓
parse_ruby_text()
 ↓
display_text + RubyAnnotation[]
 ↓
tokenize_core(display_text)
 ↓
UniDic
 ↓
apply UserRuby override
 ↓
ReadingSpan
 ↓
Mora
 ↓
Phoneme
```

Ruby 必须真接进已有 `ReadingOverride` 逻辑，而不是只保存一个字段。

---

# 6. Reading priority

明确实现：

```text
UserRuby          confidence = 1.0
ProjectOverride
UniDic
Kana surface
Unknown heuristic
```

例如：

```text
運命(さだめ)
```

即使 UniDic 给出：

```text
うんめい
```

最终也必须使用：

```text
さだめ
```

---

# 7. Ruby 与 Enhanced LRC 必须互不冲突

Reading truth 和 Timing truth 是两件不同的事。

Reading：

```text
UserRuby
>
UniDic
>
Kana
>
Heuristic
```

Timing：

```text
Enhanced LRC explicit anchors
>
ForcedAlign
>
MoraDP
>
Weighted fallback
```

未来输入：

```text
[00:18.000]<00:18.220>冷(つめ)<00:19.120>たく...
```

必须保持 `<...>` anchor 在**去 Ruby 后的 display coordinate** 中仍正确。

---

# 8. 双语 LRC 保持现有行为

用户当前文件是：

```text
[time]日语原文
[time]中文翻译
```

相同时间戳合并。

Ruby 只处理日语 primary line。

中文 translation 不送进 Japanese reading pipeline，不剥括号，不改内容。

---

# 9. 最重要的数据结构修正：Display Glyph ≠ Reading Unit

例如：

```text
二人(ふたり)
```

真实关系：

```text
2 display glyphs
3 mora
```

不能继续默认：

```text
一个汉字 = 一个读音单位 = 一个时间区间 = 一个音符
```

建议新增：

```rust
pub struct ReadingDisplayGroup {
    pub id: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub reading_span_id: usize,
    pub mora_start: usize,
    pub mora_end: usize,
    pub surface: String,
    pub reading: String,
    pub start_time: Option<f32>,
    pub end_time: Option<f32>,
}
```

例如：

```text
surface = 二人
reading = ふたり
moras = [ふ, た, り]
```

---

# 10. 修复 Forced Alignment → display token 的 coarse-span 重复问题

当前逻辑会对每个 display token 找所有 char-span 相交的 mora。

对于：

```text
二人 -> ふたり
```

如果每个 mora 都继承整个 `二人` 的 coarse span，那么：

```text
二
→ ふ / た / り

人
→ ふ / た / り
```

两字很容易获得完全相同的 start/end，并重复绑定音符。

正确架构：

```text
ReadingSpan
 ↓
Mora[]
 ↓
Forced Alignment
 ↓
ReadingDisplayGroup timing
 ↓
最后才投影到显示 glyph
```

不要让 display glyph 成为 acoustic alignment primitive。

---

# 11. 多汉字 span 的 UI 策略

推荐：

```text
    G4→A4
   ───────
    二 人
```

即 badge 首先属于整个 reading group。

如果现有 UI 暂时必须“一字一格”，可以在**声学对齐完成之后**做纯视觉时间分配，但这个 heuristic：

> 只能影响 UI highlight，不能重新进入 Forced Alignment 或 note matching。

---

# 12. 用户当前注音 LRC 应作为开发 oracle

当前这首歌已经覆盖了很好的测试类型：

```text
冷(つめ)
微笑(ほほえ)
悲(かな)
位(くらい)
心(こころ)
嵐(あらし)
流(なが)
人(ひと)
群(む)
二人(ふたり)
愛(あい)
```

尤其：

```text
1 glyph -> N mora:
心 -> こころ
愛 -> あい
位 -> くらい

N glyph -> M mora:
二人 -> ふたり
指先 -> ゆびさき
以上 -> いじょう
```

对当前这首开发测试歌，**建议完整保留这份注音 LRC**，把“读音不确定”这个变量先彻底排除。

但正式产品不应该要求普通用户必须逐个汉字注音：

```text
普通 LRC
→ UniDic 自动读音

高精度 LRC
→ 漢字(かな) 覆盖自动读音
```

---

# 13. P0：注音不会自动修好音高，必须重做音符显示语义

当前前端详细模式本质：

```ts
token.pitch_notes
  .map(noteName)
  .join("→")
```

所以：

```text
D4→D4
D4→A#4→A4→G4→...
```

只要后端绑定了事件，就全部倒到 UI。

这不是“详细音高”。

应该重新定义为三个内部层：

```text
1. Compact
2. Musical Detail
3. Debug Raw
```

---

# 14. Compact 模式

原则：

> **宁缺毋滥。**

只显示：

```text
high-confidence primary musical note
```

证据不足就空着。

不要为了“每个字上方必须有音符”强制填一个 C4/A#4。

---

# 15. Musical Detail 模式

只显示：

```text
musically significant note sequence
```

真正 melisma：

```text
我
G4→A4→G4
```

应该保留。

普通 vibrato：

```text
A4 附近周期摆动
```

只显示：

```text
A4
```

缓慢 glide：

```text
A4 -> B4
```

不应因为经过 A#4 就显示：

```text
A4→A#4→B4
```

---

# 16. Debug Raw 模式

只在调试面板显示：

```text
全部 GAME events
全部 FCPE frames
raw matcher scores
raw overlap
FA timing
```

不要把 Raw Debug 当“详细音高”。

---

# 17. GAME 输出必须先做官方 reference parity

Round 3.1 的 Rust GAME 已经真正跑 ONNX，但还存在可疑点：

```text
duration 单位采用自适应猜测
confidence = 0.9 写死
boundary_confidence = None
is_slur = false 写死
```

因此在继续调显示阈值前，必须先回答：

> **同一个 WAV，Rust GAME 与 OpenVPI 官方/reference implementation 是否产生近似相同的 notes？**

新增开发期 oracle：

```text
official GAME / dataset-tools
 ↓
reference JSON
```

Rust 对相同 fixture 比较：

```text
note count
pitch
onset
offset
```

建议初始 acceptance：

```text
pitch agreement >= 90%
median onset error <= 50 ms
median offset error <= 80 ms
note-count difference <= 10%
```

具体阈值可以依据官方输出调整。

在 parity 通过前，禁止直接归因：

```text
“GAME 本来就这样”
```

---

# 18. GAME metadata 必须诚实

不要再让：

```rust
confidence: 0.9
is_slur: false
```

这种固定值参与生产评分。

如果模型能提供：

```text
presence probability
boundary score
slur
```

就传真实值。

如果确实没有：

```text
None
```

也比伪造 0.9 更好。

---

# 19. 在 GAME 与 UI 之间增加 `CanonicalNotePostProcessor`

建议结构：

```rust
pub struct CanonicalSungNote {
    pub event_ids: Vec<u32>,
    pub start: f32,
    pub end: f32,
    pub game_midi: f32,
    pub fcpe_median_midi: Option<f32>,
    pub display_midi: f32,
    pub voiced_coverage: f32,
    pub fcpe_support: f32,
    pub confidence: f32,
    pub class: SungNoteClass,
}

pub enum SungNoteClass {
    Stable,
    Ornament,
    Transition,
    Uncertain,
}
```

---

# 20. GAME + FCPE 做交叉证据，而不是互相替代

已有两条独立观测：

```text
GAME
→ discrete musical notes

FCPE
→ continuous F0
```

对于每个 GAME event：

1. 找时间窗内 FCPE voiced frames；
2. 丢弃低 FCPE confidence frames；
3. 算 weighted median MIDI；
4. 算 voiced coverage；
5. 算 MIDI MAD/IQR；
6. 算 GAME pitch 与 FCPE median 的 cents 差。

例如：

```text
GAME = C4
FCPE median = F4
```

差 5 semitones。

这种事件不应该高置信显示。

---

# 21. 新增 NoteEvidence

```rust
pub struct NoteEvidence {
    pub fcpe_frame_count: usize,
    pub voiced_coverage: f32,
    pub median_midi: Option<f32>,
    pub midi_mad_cents: Option<f32>,
    pub pitch_delta_cents: Option<f32>,
    pub support_score: f32,
}
```

Compact/Detail 都使用经过 evidence 的 canonical note，不直接使用 raw GAME event。

---

# 22. 相邻同音必须合并

截图里：

```text
D4→D4
```

完全没有音乐信息价值。

同 `midi_rounded` 且间隔很短的相邻事件：

```text
D4 100ms
D4 80ms
D4 130ms
```

合并：

```text
D4 310ms
```

---

# 23. 不要只写死一个 min-duration

真正快速转音也可能很短。

额外音符是否保留至少同时看：

```text
absolute duration
relative duration
FCPE voiced coverage
FCPE pitch support
pitch separation
```

建议 significance 概念：

```text
significance =
  duration_score
× voiced_coverage_score
× pitch_support
× relative_occupancy_score
```

参数用 fixture 标定，不要把任一阈值当神值。

---

# 24. 真正 melisma 的判据

同一个 aligned mora / ReadingDisplayGroup 内：

```text
至少两个独立稳定 musical-note regions
```

并且每个都有：

```text
足够时长
独立 FCPE plateau/support
```

才显示：

```text
G4→A4→G4
```

---

# 25. 详细模式 UI 限制

≤ 3 个有效音：

```text
A4→B4→A4
```

完整显示。

> 3 个：

```text
A4→…→G4 ×5
```

点击 Debug 查看完整序列。

这只是视觉压缩，不删除真实数据。

---

# 26. Compact 主音评分也要加入 FCPE 证据

不要只：

```text
duration × confidence
```

建议：

```text
primary_score =
  true_overlap_duration
× fcpe_support
× voiced_coverage
× model_confidence_if_real
```

如果跨模型一致性低：

```text
留空
```

比显示明显错误音符更好。

---

# 27. 新增 UnpitchedReason

建议增加：

```rust
LowCrossModelAgreement,
GameUnsupportedByFcpe,
AmbiguousMusicalNote,
AlignmentLowConfidence,
```

Debug 能解释：

```text
为什么这个字没显示音高
```

而不是一律 `None`。

---

# 28. Forced Alignment：Ruby 会显著帮助，但 MMS 仍需唱歌域验证

当前 FA backend 是 `torchaudio.MMS_FA`。

Ruby 会让输入变成确定的：

```text
User Reading
 ↓
Mora
 ↓
Phoneme
 ↓
MMS transcript
```

因此读音歧义会减少。

但是 MMS 属于 speech-domain，不能因为“真实模型已经运行”就假设 singing alignment 一定够好。

建议选 5~10 句人工试听：

```text
冷たく微笑んだ
悲しい位に見つめてた
流れる人の群れ
知らない二人にもどるのね
心を引き裂く嵐の中
```

检查 mora/token onset。

如果 MMS 明显仍差：

```text
用 pyshiro 作开发期外部 oracle benchmark
```

不要直接复制 GPLv3 代码进核心；只在开发机比较同一 WAV/reading 的边界。

---

# 29. Debug Overlay 必须能解释一切

点：

```text
冷
```

应看到：

```text
surface: 冷
reading: つめ
reading_source: UserRuby
moras: [つ, め]

alignment_source: ForcedAlign
alignment_confidence: ...

GAME candidates:
...

FCPE median:
...

canonical decision:
keep / merge / reject

display:
C4 / blank / ...
```

对于：

```text
二人(ふたり)
```

应看到：

```text
display group: 二人
reading: ふたり
moras: [ふ, た, り]
```

禁止显示：

```text
二 -> 整个ふたり
人 -> 整个ふたり
```

作为两个独立 acoustic truth。

---

# 30. 单元测试

Ruby parser：

```text
愛(あい)
心(こころ)
位(くらい)
二人(ふたり)
指先(ゆびさき)
背伸(せの)び
知(し)らない
```

断言：

```text
display_text
annotations
display char span
reading
```

Literal parentheses：

```text
(コーラス)
（笑）
(yeah)
```

不得误判。

Malformed：

```text
愛(
愛()
愛(あい
愛(ai)
```

不得 panic。

Reading priority：

```text
運命(さだめ)
```

最终必须 `さだめ`。

Mora：

```text
愛 -> [あ, い]
心 -> [こ, こ, ろ]
二人 -> [ふ, た, り]
```

---

# 31. GAME / Fusion tests

官方 GAME parity fixture：

```text
same WAV
official reference
Rust result
```

必须比较。

Cross validation：

```text
GAME A4 + FCPE ~A4
→ keep

GAME C5 + FCPE ~A4
→ reject/uncertain
```

Consolidation：

```text
D4 + D4
→ D4

A4 vibrato fragmentation
→ A4

A4 -> B4 stable transition
→ A4→B4

G4→A4→G4 true melisma
→ 保留
```

---

# 32. UI tests

详细模式不得出现：

```text
D4→D4
```

不得让一个 badge 无限变长撑坏歌词行。

普通模式低置信结果：

```text
blank
```

优先于错误音符。

---

# 33. 本轮实施顺序

## Commit 1 — Ruby parser

```text
漢字(かな)
→ display_text + RubyAnnotation
```

## Commit 2 — Ruby reading integration

```text
UserRuby > UniDic
```

## Commit 3 — ReadingDisplayGroup

修复多字符 span 的 display duplication。

## Commit 4 — Ruby × Enhanced-LRC offset mapping

确保逐字时间 anchor 不漂移。

## Commit 5 — GAME official parity harness

先验证 Rust inference/decoder 是否真的与官方一致。

## Commit 6 — GAME metadata correctness

移除固定 `confidence=0.9`、假 `is_slur=false`。

## Commit 7 — FCPE evidence fusion

生成 `CanonicalSungNote`。

## Commit 8 — Note consolidation

同音合并、低支持过滤、vibrato/glide 抑制、真 melisma 保留。

## Commit 9 — UI semantics

```text
Compact
Musical Detail
Debug Raw
```

## Commit 10 — 本地真实歌曲验收

使用当前注音 LRC 完整试听与 Debug。

---

# 34. 不要做的事情

本轮禁止：

- 再加新的 pitch 神经网络；
- 大改 UI 主题；
- 增加 ASR；
- 增加在线歌词搜索；
- 增加自动翻译；
- 把 `漢字≈1.8` 继续作为主算法；
- 强求每个字必须有音符；
- 直接把所有 raw GAME notes 当“转音”；
- 继续用单个 `70ms/100ms` 参数包治百病；
- 强迫用户把多汉字词伪造为逐字 reading；
- 用“接口已预留”代替真实生产链路。

---

# 35. Definition of Done

只有全部满足才允许汇报完成：

```text
[ ] 漢字(かな) 正式解析
[ ] Ruby reading 不显示、不进入 display token
[ ] UserRuby 优先于 UniDic
[ ] 所有 char offsets 统一到 display_text
[ ] Ruby + Enhanced LRC anchor 正确
[ ] 二人(ふたり) 不重复整组 mora 到两个 glyph
[ ] ForcedAlign 消费 Ruby-derived mora/phoneme
[ ] Rust GAME 通过官方 reference parity
[ ] 不再使用假的 0.9 confidence 做生产评分
[ ] GAME note 具有 FCPE evidence
[ ] 相邻同音合并
[ ] vibrato 不再变成长 note-chain
[ ] 真 melisma 能保留
[ ] Compact 宁缺毋滥
[ ] Musical Detail 不再等于 Raw Dump
[ ] Debug 可以解释每一个额外 note 为什么保留
[ ] 当前这首注音 LRC 完整人工验收
```

---

# 36. 最终开发 Agent 交付报告必须包含

不要只写：

```text
全部完成，测试通过
```

必须给出：

```text
commit SHA
```

以及以下真实输出。

## Ruby

```text
input:
冷(つめ)たく微笑(ほほえ)んだ

display:
冷たく微笑んだ

reading:
冷 -> つめ [UserRuby]
微笑 -> ほほえ [UserRuby]
```

## Multi-char

```text
二人(ふたり)
moras = ふ / た / り
display duplication = false
```

## Forced Alignment

```text
alignment_source = ForcedAlign
fallback = false
```

## GAME parity

```text
reference note count:
Rust note count:
pitch agreement:
onset MAE:
offset MAE:
```

## Fusion sample

```text
GAME:
FCPE median:
support:
decision: keep / merge / reject
```

## UI

```text
compact:
musical detail:
debug raw:
```

---

# 37. 给开发 Agent 的最后一句话

本轮不要追求：

> “让每个汉字上方都出现一个音符。”

目标应该变成：

> **只有当语言学对齐与声学证据都足够可靠时，程序才声称某个歌词单位对应某个音乐音符。**

用户/LLM 辅助提供的：

```text
漢字(かな)
```

是非常有价值的语言真值，但它只解决 reading。

最终准确度来自：

```text
Explicit Reading
+
Forced Alignment
+
GAME official-correct inference
+
FCPE independent evidence
+
Conservative display policy
```

其中任何一层证据不足：

```text
宁可空着
```

也不要再输出一排看似精确、实际上错误的 `C4 / A#4 / D4→D4→...`。
