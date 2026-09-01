# Pitch Analyzer Round 3.2 源码审计报告 & Round 3.3 收尾修复建议

> 仓库：`https://github.com/hydrogen1222/pitch-analyzer`  
> 审计提交：`9a82f1994ff097c8f3c3306d9d7e983291bf50c1`  
> Commit message：`feat: implement Round3.2 ruby lyrics and evidence notes`  
> 变更规模：19 files changed, +1544 / -221  
>
> 结论：**这轮是真正做了很多东西，不是“接口完成学”。Ruby-LRC、ReadingDisplayGroup、GAME/FCPE evidence fusion、三档显示、真实 GAME parity harness 都已经落地。**
>
> 但是我不建议现在宣布“算法完成”。本轮还存在几个会直接影响真实歌曲效果的逻辑问题，其中至少 3 个应作为 P0/P1 修复后再做目标歌曲试听。

---

# 0. 审计总评

## 已确认真实完成

### Ruby/Furigana LRC

已经真实支持：

```text
冷(つめ)
微笑(ほほえ)
二人(ふたり)
運命(さだめ)
```

并且：

```text
raw annotated text
↓
display text
↓
RubyAnnotation[]
```

的坐标体系已经建立。

`parse_ruby_text()` 确实会把：

```text
冷(つめ)たく微笑(ほほえ)んだ
```

解析成：

```text
display_text = 冷たく微笑んだ
```

并保存：

```text
冷   -> つめ
微笑 -> ほほえ
```

的 display span。

---

### Ruby > UniDic

Ruby annotation 已真正转为 `ReadingOverride`，并在 mora 展开前覆盖 UniDic。

对于：

```text
運命(さだめ)
```

会用：

```text
さだめ
```

而不是 UniDic 默认读音。

而且 multi-glyph override 会把相交的多个 UniDic span splice 成一个 authoritative ReadingSpan。

这是正确方向。

---

### `二人(ふたり)` 不再重复 mora

新增：

```rust
ReadingDisplayGroup
```

而且测试明确验证：

```text
二人 = 2 glyph
ふたり = 3 mora
```

两个 glyph 共享一个 display group，但 acoustic unit 数仍然等于 mora 数，不再产生：

```text
3 mora → 6 align units
```

这一轮是真修掉了。

---

### Ruby × Enhanced LRC 坐标

代码先去 `<time>`，再解析 Ruby，并通过 `ruby_display_offset()` 把 anchor 投影到 display coordinate。

测试也覆盖：

```text
<00:00.000>冷(つめ)<00:00.500>たく
```

这是非常重要且做对了的一块。

---

### GAME metadata 不再伪造 0.9

Round3.1 里：

```rust
confidence: 0.9
is_slur: Some(false)
```

这轮已经改成：

```rust
confidence: 0.0
model_confidence: None
is_slur: None
```

并明确说明 released GAME ONNX 只提供 hard presence，不能伪造 calibrated confidence。

这是正确的。

---

### GAME duration 解码比上一轮更严谨

之前代码会自动猜：

```text
秒？
帧？
还是按 chunk_duration 强行 scale？
```

这一轮明确采用官方 GAME `bd2dur` 输出为秒的语义，并且：

```text
sum(valid_duration)
```

与 chunk duration 差异过大时直接报错，不再暗中 rescale。

这是明显进步。

---

### FCPE × GAME evidence fusion

新增：

```rust
CanonicalNotePostProcessor
CanonicalSungNote
NoteEvidence
SungNoteClass
```

并且生产链真的走：

```text
GAME raw notes
↓
CanonicalNotePostProcessor
↓
accepted_events()
↓
PitchTrack.musical_notes
```

`raw_game_notes` 和 `canonical_sung_notes` 也保留下来用于 Debug。

所以现在确实已经从：

```text
GAME raw → UI
```

变成：

```text
GAME + FCPE evidence → canonical notes → UI
```

---

### UI 三档语义真的接进去了

已有：

```text
Compact
Musical Detail
Debug Raw
```

并且 Musical Detail 会去掉连续重复音名：

```text
D4→D4
→
D4
```

超过三个音时也会压缩为：

```text
A4→…→G4 ×5
```

不会再让 badge 无限增长。

---

# 1. P0：真转音 / melisma 的回归测试其实是“假绿”

这是本轮最明确的测试问题。

测试：

```rust
let melisma = processor.process(
    &[
        game_event(0, 0.0, 0.40, 69.0),
        game_event(1, 0.40, 0.80, 71.0),
    ],
    &fcpe_track(69.0),
);

assert_eq!(melisma.len(), 2);
```

它想证明：

```text
A4 → B4
```

是真正稳定转音，应保留两个音。

但注意：

```text
整个 FCPE track = MIDI 69 (A4)
第二个 GAME event = MIDI 71 (B4)
```

因此第二个 event 的：

```text
pitch_delta = 200 cents
```

而默认：

```rust
maximum_pitch_delta_cents = 180
```

所以第二个音实际上会被：

```text
class = Uncertain
```

`process()` 本来就会保留 uncertain candidate 供 Debug，因此：

```text
melisma.len() == 2
```

完全不能证明第二个音进入生产 musical notes。

换句话说：

> **测试声称“true melisma retained”，实际上只证明“两个 raw/canonical decision 对象还存在”。**

它没有检查：

```rust
processor.accepted_events(&melisma)
```

也没有检查：

```rust
melisma[1].class != Uncertain
```

---

## 修复

synthetic FCPE 必须与 melisma 同步：

```text
0.00–0.40: MIDI 69
0.40–0.80: MIDI 71
```

然后断言：

```rust
assert_eq!(accepted.len(), 2);
assert_ne!(melisma[0].class, SungNoteClass::Uncertain);
assert_ne!(melisma[1].class, SungNoteClass::Uncertain);
```

更完整应加入：

```text
G4 → A4 → G4
```

三段稳定平台。

---

# 2. P0/P1：`classify_transition()` 的语义是错的

当前：

```rust
if class == Stable
    && previous note exists
    && pitch changed >= 1 semitone
{
    Transition
}
```

这意味着普通旋律：

```text
C4 → D4 → E4 → G4
```

会变成：

```text
C4 Stable
D4 Transition
E4 Transition
G4 Transition
```

但这里的：

```text
Transition
```

并不是“滑音中的过渡段”。

它只是：

> “这个音和前一个音不一样。”

这是两个完全不同的概念。

---

## 为什么危险

我们原来定义的：

```text
Stable
Ornament
Transition
Uncertain
```

是想表达**音符内部/声学形态**：

```text
Stable      = 稳定目标音
Ornament    = 短促装饰音
Transition  = glide / portamento 中间过渡
Uncertain   = 证据不足
```

而当前实现把：

```text
“正常换音”
```

也叫 Transition。

这样以后任何：

```text
Musical Detail filter
Debug interpretation
melisma classifier
```

都会被污染。

---

## 修复建议

不要根据：

```text
previous note MIDI != current note MIDI
```

分类 Transition。

Transition 必须基于 FCPE contour 内部形态，例如：

```text
持续单调变化
缺少稳定平台
MAD / slope pattern
短暂占据中间 pitch
```

如果暂时不想实现真正 glide classifier：

> **宁可所有 accepted note 继续标 Stable / Ornament，也不要错误标 Transition。**

---

# 3. P0/P1：Uncertain fragment 会“毒死”一个本来正确的稳定音

当前 consolidation：

```rust
if same_note || vibrato_fragment {
    merge_into(current, next);
}
```

而：

```rust
left.class =
    if left.class == Uncertain || right.class == Uncertain {
        Uncertain
    } else {
        Stable
    };
```

这意味着：

```text
稳定 A4 300 ms
+
旁边一个低证据 A4 30 ms
```

只要两个事件满足 merge 条件：

```text
same note
小 gap
```

最终整个合并 note 会变成：

```text
Uncertain
```

然后：

```rust
accepted_events()
```

会把整段删除。

---

## 这和“宁缺毋滥”不是一回事

正确策略应该是：

> **低质量小碎片不能反向否定已经有强证据的主稳定平台。**

当前逻辑却是：

```text
Stable + Uncertain = Uncertain
```

属于典型的 uncertainty poisoning。

这会制造：

```text
明明有一个很稳定的音
结果 badge 整个消失
```

---

## 建议修法 A（推荐）

先按时间邻接聚合 region，然后对**合并后的完整时间窗重新 measure FCPE evidence**，最后重新 classify。

即：

```text
raw candidate fragments
↓
candidate grouping
↓
merged interval
↓
重新计算 FCPE median / MAD / coverage / delta
↓
最终 classification
```

不要平均旧 evidence 再用 OR 传播 Uncertain。

---

## 建议修法 B（最低改动）

至少：

```text
Stable + tiny Uncertain same-pitch fragment
→ Stable
```

只要 stable 部分：

```text
时长占比
support
coverage
```

明显占优势。

---

# 4. P1：当前 GAME/FCPE 允许“差接近两个半音”仍通过，过于宽松

默认：

```rust
maximum_pitch_delta_cents = 180
```

而 pitch support：

```rust
1 - |delta| / 240
```

因此：

```text
GAME 与 FCPE 相差 100 cents
```

并不会被拒绝。

只要：

```text
coverage 高
MAD 小
```

仍可能获得相当不错的 support。

而最终：

```rust
display_midi = FCPE median
```

也就是说：

> GAME 候选可能是一个音，而 UI 最后直接使用另一个由 FCPE 决定的音。

这并非一定错误，但需要明确：此时 GAME 更多是在提供**时间分段**，FCPE 在提供最终 pitch identity。

如果这是设计目标，就应该显式写成：

```text
GAME = note boundary engine
FCPE = pitch identity verifier/corrector
```

而不是继续把输出称为“GAME canonical note”。

---

## 建议

先不要拍脑袋把 180 改成 50。

必须使用真实人声 fixture 标定：

```text
GAME pitch vs FCPE median
```

统计：

```text
正确 note 的 delta 分布
错误 note 的 delta 分布
```

然后选 threshold。

我倾向初始研究范围：

```text
80–120 cents
```

但最终必须靠 corpus，而不是直接采纳这个数字。

---

# 5. P1：`vibrato_fragment <= 0.70 semitone` 依旧只是启发式

当前 consolidation：

```rust
abs(current.display_midi - next.display_midi) <= 0.70
```

就可能认作 vibrato fragmentation。

这比旧的：

```text
整数 MIDI 一变化就换音
```

好很多。

但它并没有真正判断：

```text
周期性
中心 pitch
slope
稳定平台
```

因此它仍可能：

- 吞掉真实快速邻近装饰音；
- 保留较大幅 vibrato 的碎片；
- 误把 glide 小段当 stable notes。

本轮可以保留这个 heuristic，但不要把：

```text
“vibrato 识别完成”
```

写进 DoD。

---

# 6. P1：真正的 GAME official parity test 存在，但默认不会跑

这是好事与限制同时存在。

你现在有真实：

```rust
game_official_reference_parity()
```

它会：

```text
同一 WAV
Rust GAME
vs
official/reference JSON
```

并比较：

```text
pitch agreement
onset MAE
offset MAE
note count difference
```

这个设计是对的。

但它：

```rust
#[ignore]
```

并依赖：

```text
GAME_PARITY_AUDIO
GAME_PARITY_REFERENCE
```

外部 fixture。

所以普通：

```bash
cargo test
```

全绿，**仍不代表 GAME parity 已通过。**

---

## 必须实际执行一次

在下一次宣称音高算法可用之前，开发机上必须真的运行：

```bash
ROUND3_ACCEPTANCE=1 \
GAME_PARITY_AUDIO=... \
GAME_PARITY_REFERENCE=... \
cargo test --manifest-path src-tauri/Cargo.toml \
  --test round3_acceptance game_official_reference_parity -- --ignored --nocapture
```

Windows 改成相应 PowerShell 环境变量写法。

最终报告贴：

```text
pitch_agreement
onset_mae
offset_mae
note_count_difference
```

没有这四个数字，不要继续猜 GAME Rust decoder 对不对。

---

# 7. P1：Round3.2 仍没有真正针对用户目标歌曲的可重复 acceptance

当前新的 `round3_2_acceptance.rs` 很不错，但主要还是：

```text
Ruby synthetic
enhanced LRC synthetic
FCPE synthetic
GAME comparator synthetic
```

它没有把当前真正失败的歌曲片段变成：

```text
人工标注 oracle
```

当然完整商业歌曲音频不应该提交仓库。

但是本地应至少做一个：

```text
private/manual fixture
```

记录以下 5 句：

```text
冷(つめ)たく微笑(ほほえ)んだ
悲(かな)しい位(くらい)に見(み)つめてた
流(なが)れる人(ひと)の群(む)れ
知(し)らない二人(ふたり)にもどるのね
心(こころ)を引(ひ)き裂(さ)く嵐(あらし)の中(なか)
```

至少人工标：

```text
大致 mora onset
主音
是否存在真实 melisma
```

否则算法很容易在 synthetic tests 全绿，但截图仍旧很差。

---

# 8. 非常重要的运行时提醒：Forced Alignment 默认仍然是关闭的

README 当前明确说明：

```text
PITCH_ANALYZER_FA_ENABLE=1
```

才启用 MMS-FA。

否则：

```text
MoraDP fallback
```

仍然正常运行。

因此如果你直接：

```text
pnpm tauri dev
```

而没有设置这个变量，**即使你把整首歌都写成 `漢字(かな)`，也并不代表你正在测试真正的 Forced Alignment。**

Ruby 仍会改善：

```text
mora 数
reading
duration prior
```

但核心 timing 仍可能是：

```text
MoraDP
```

---

## 测试时必须确认 Debug

目标：

```text
reading_source = UserRuby
alignment_source = ForcedAlign
musical_note_source = Game
```

如果看到：

```text
alignment_source = MoraDp
```

那么你测试的不是完整 Round3.2 高精度链。

---

# 9. 同样确认 GAME 是否真的启用

完整 GAME bundle 必须存在：

```text
config.json
encoder.onnx
segmenter.onnx
bd2dur.onnx
dur2bd.onnx
estimator.onnx
```

否则程序会：

```text
LegacyFcpeTracker
```

fallback。

Debug 必须看到：

```text
Note engine: Game / ...
```

否则不能用当前结果评价 GAME + FCPE fusion。

---

# 10. P2：Ruby parser 的当前语法范围是有意“窄”的

当前 parser 只在：

```text
紧邻 `(` 前是 Kanji
```

时解析，并向前取连续 Kanji。

所以很好支持：

```text
微笑(ほほえ)
二人(ふたり)
指先(ゆびさき)
```

但不支持把“混合 kana+kanji surface 整体”作为一个 ruby span，例如：

```text
取り戻す(とりもどす)
```

因为 `(` 前是 `す`，不是汉字。

这不是当前用户文件的 blocker，因为你的写法是：

```text
戻(もど)
```

这种局部注音。

所以 Round3.3 不需要扩大语法，除非以后确实遇到这种 LRC。

---

# 11. P2：全角括号目前不是 Ruby syntax

当前只解析：

```text
漢字(かな)
```

不会解析：

```text
漢字（かな）
```

这也可以接受，因为 README 已明确约定 ASCII parentheses。

后续若做用户友好增强，再支持全角括号即可。

---

# 12. 下一轮建议：不要再大重构，做 Round 3.3 算法校准

当前架构已经基本正确。

不要再：

```text
换模型
加 ASR
再造第四层 abstraction
```

下一轮只修：

1. melisma test；
2. Transition class；
3. uncertainty poisoning；
4. GAME parity 实际跑通；
5. real-song private oracle；
6. 标定 FCPE/GAME threshold。

---

# 13. Round 3.3 推荐提交顺序

## Commit 1 — fix melisma acceptance test

构造真实 piecewise FCPE：

```text
G4 0.0–0.4
A4 0.4–0.8
G4 0.8–1.2
```

GAME 同步：

```text
G4 → A4 → G4
```

断言：

```text
accepted_events.len() == 3
```

再加 vibrato fixture：

```text
A4 center ± cents
```

断言：

```text
accepted == one A4
```

---

## Commit 2 — remove false Transition semantics

临时方案：

```text
Stable / Ornament / Uncertain
```

先真正准确。

只有实现 FCPE slope/glide classifier 后再启用：

```text
Transition
```

---

## Commit 3 — remeasure after merge

从：

```text
fragment evidence average
+
Uncertain OR propagation
```

改成：

```text
group fragments
↓
merged interval
↓
重新测 FCPE evidence
↓
重新 classify
```

---

## Commit 4 — run official GAME parity for real

用官方/reference JSON 真跑一次。

把结果保存为开发报告：

```text
pitch agreement:
onset MAE:
offset MAE:
note-count delta:
```

如果不过，先修 GAME decoder，不做后续 threshold tuning。

---

## Commit 5 — target-song calibration

本地人工标 5 句。

至少统计：

```text
Compact 主音正确率
明显错误 badge 数
漏 badge 数
真实 melisma precision/recall
```

不要追求论文级指标，先让结果肉眼可靠。

---

# 14. 建议验收指标

对于当前用途，我建议比“每个字都有音符”更实际：

## Compact

```text
错误音高率 < 5–10%
```

宁愿漏一些。

---

## Musical Detail

重点是 precision：

```text
显示出来的转音大部分真的是转音
```

建议：

```text
melisma precision > 85–90%
```

recall 可以先低一些。

即：

> 少报比乱报更好。

---

## Lyric timing

Ruby + FA：

```text
肉耳基本不会感觉 active glyph 提前/滞后
```

可以对 20–30 个关键 mora 人工打点，看：

```text
median boundary error
```

---

# 15. 当前这轮是否值得马上试运行？

**值得。**

但必须确保实际启动的是完整链：

```text
UserRuby
+
ForcedAlign
+
GAME
+
FCPE evidence
```

而不是：

```text
Ruby
+
MoraDP fallback
+
LegacyFcpeTracker
```

否则会得到一个“代码明明都改了，怎么效果还是差”的假结论。

---

# 16. 我建议你下一次运行前先确认的 Debug 四行

随便点击一个 Ruby 汉字：

```text
Reading source: UserRuby
Alignment: ForcedAlign
Note engine: Game
Canonical decisions: > 0
```

如果这四项有任何一个不对，先处理运行环境，不要开始评价算法效果。

---

# 17. 本轮最终评级

我会给 `9a82f19`：

```text
架构实现：A-
Ruby/日语结构：A
生产接线：A-
测试设计：B+
音符分类语义：C+/B-
真实歌曲验证：尚未完成
```

相比 Round3 初版，这是**质的进步**。

现在已经不需要再推翻架构。

真正剩下的是：

> **让现有架构在真实歌声上被正确标定，并修掉几个 post-processing 的逻辑坑。**

---

# 18. 最重要的三个修复点

只看三件事的话，就是：

### ① 修真转音测试

当前 melisma test 是假绿。

### ② 修 Stable + Uncertain 合并污染

不要让短坏碎片删除长好音符。

### ③ 不要把“正常换音”标成 Transition

Transition 应代表 glide/过渡，而不是“和前一个音不同”。

把这三项修掉，再实际跑一次 GAME official parity，然后再回到那首《ドラマ》试听，才是下一步最有价值的工作。
