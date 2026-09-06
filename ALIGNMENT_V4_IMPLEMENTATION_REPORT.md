# Alignment Architecture v4 (Round 4) 实施报告

> 对应任务书: `Pitch Analyzer：日语 / 华语演唱音高－歌词对齐架构重构开发任务书`
> 实施方式: 严格按任务书 §25 的 Commit 1 → Commit 12 顺序, 每个 commit 保持可编译、测试可运行。
> 分支: `main`（`87fee3f` → `a0d1f59` 共 12 个 commit）

---

## 1. Commit 清单（与任务书 §25 一一对应）

| Commit | 内容 | 任务书条目 |
| --- | --- | --- |
| C1 `87fee3f` | Baseline & Diagnostics：`diagnostics.rs`（对齐来源分布/语言脚本观测）、pipeline 汇总日志、行为快照测试（含 P0-1 直接证据：`我爱你`→`わ が`） | §25-C1 |
| C2 `66589e5` | Language Router：`SingingLanguage`/`ProjectLanguage`、`language::router`、解析只做语法+逐行路由、中文行不进 UniDic、工程语言设置与 Tauri 命令、ProjectData.language（serde default 向后兼容） | §25-C2（P0-1） |
| C3 `f64c5a4` | GAME Language Support：`GameConfig.languages` 动态解析、`NoteTranscriptionRequest` trait 改造、`language_id()`（缺失→0+warning，无硬编码 ja=2/zh=3）、模型 config fixture 单测 | §25-C3（P0-2） |
| C4 `bd50cdd` | Remove False Precision：删除 200ms phonetic grace（P0-7）、删除 MoraDP 0.7/0.3 伪置信度（P0-5）、FA 未覆盖 token 不再整行赋时（P0-6）、`TimingPurpose::VisualOnly` + WeightedFallback 排除出正式 pitch attribution（§15.1） | §25-C4 |
| C5 `8ecba6d` | Core v4 Data Model：SourceSpan/PronunciationUnit/PhonemeUnit/AcousticSpan/PitchAttachment/F0Summary/AlignmentEvidence/NoteReliability、っ/ー/ん 语义显式化（P1-4）、旧结构 projection、serde round-trip | §25-C5（§7） |
| C6 `4556469` | Japanese Pipeline v4：mora parser 外来音扩展 + NFKC 规范化（§8.4）、`JapanesePhonemizer` trait（§8.5）、`pitch_policy.rs` 单一事实源（nucleus 范围，P1-3）、`build_japanese_layers` 搬至 `japanese/pipeline.rs`、半角假名全链路 | §25-C6 |
| C7 `5f2f481` | Mandarin Pipeline v1：`MandarinReadingProvider` 抽象、保守词典（词组消歧 > 单字 fallback > Unknown/Ambiguous）、拼音→声母/韵母、用户 override、`MandarinSyllable` 投影 | §25-C7（§9） |
| C8 `e62bffa` | Forced Alignment Backend v2：纯声学 `AlignmentRequest/Result`（segment_id/unit_id 显式，§27.3）、持久 JSONL worker（模型/音频只加载一次，P0-4）、singing-FA 诚实骨架（DependencyMissing）、Backend Manager 优先级、`THIRD_PARTY_MODELS.md`（§30）、Tier-B 协议测试（mock worker） | §25-C8（§10） |
| C9 `c8a47f5` | LyricNoteMatcher v2：真实重叠准入（15ms 帧量化容差）、many-to-many、并发竞争才算歧义（melisma 保留全部 note）、可解释评分（权重集中在 `LyricNoteMatchParams`）、诚实 status 全集、F0Summary 稳健统计（§13.2）、`attach_lines` 接入 rebind | §25-C9（§12） |
| C10 `a509fb6` | GAME Chunking v2：`GameChunkingConfig`（可由模型 config 覆盖）、静音优先/硬切兜底、[context\|core\|context] 结构、onset 归属 core（丢弃边缘低可信边界）、跨块同音 reconcile、Mode B 保持单块 | §25-C10（P1-5，§14.2） |
| C11 `3731df1` | UI v4：状态符号（—/◦/? + tooltip）、中文可选拼音行、语言选择器（变更后自动重载歌词）、侧栏 backend/回退警告、右下 backend 标签、debug 面板 v4（language/attachments/F0 摘要，§17） | §25-C11（§16） |
| C12 `a0d1f59` | Benchmark Gate：gold schema（§20.1）、合成器、评估器（§20.4 全指标）、基线回归门（Tier D）、3 个仓库 fixture（ja 边界证据/ja melisma/zh 拼音音节）、`baseline_report.json` | §25-C12（§20/§22） |

**披露**：会话开始前用户工作区中已删除的 6 个旧任务书 `.md` 被后续某次 `git add -A`（C2）一并纳入提交。该删除是用户在本次开发开始前的既有操作，非本次重构删除的代码。

## 2. 修改文件列表（摘要）

- **新增 Rust 模块**：`src-tauri/src/language/{mod,router}.rs`、`src-tauri/src/mandarin/{mod,reading,phoneme,overrides,pipeline}.rs`、`src-tauri/src/alignment/{mod,projection,note_matcher,benchmark}.rs`、`src-tauri/src/japanese/{pitch_policy,pipeline}.rs`、`src-tauri/src/forced_align/{types,legacy_mms,singing_fa,manager}.rs`、`src-tauri/src/diagnostics.rs`
- **重写/扩展**：`models.rs`（v4 类型 + 语言枚举 + TimingPurpose + LyricLine.language/pitch_attachments）、`lyrics.rs`（语言路由、P0-4~P0-7 修复、瘦身）、`note_engine/{mod,game,legacy_fcpe}.rs`、`japanese/{mora,phoneme,reading,unidic,mod}.rs`、`lib.rs`（manager/状态命令/诊断接线）
- **脚本**：`scripts/legacy_mms_worker.py`（持久 worker）、`scripts/test_mms_worker_mock.py`（Tier-B mock）
- **前端**：`src/{karaoke_display,main,models_lyrics}.ts`、`index.html`
- **测试**：新增 8 个集成测试文件 + 6 个模块内测试块（合计 24 个测试二进制、约 200 项断言）
- **数据/文档**：`testdata/alignment_gold/`（3 fixture + baseline + README）、`THIRD_PARTY_MODELS.md`

## 3. 新数据结构（v4 canonical，§7）

- 文字层：`SourceSpan { surface, char 区间, language, source_kind, pronunciation_ids }`
- 发音层：`PronunciationUnit { text, normalized, kind, pitch_bearing, phoneme_ids }`，kind 含 `JapaneseMora/Sokuon/LongVowel/MoraicNasal/MandarinSyllable`
- 音素层：`PhonemeUnit { symbol, class, pitch_bearing, alignment: Option<AcousticSpan> }`
- 声学层：`AcousticSpan { start, end, backend, quality, raw_score, calibrated_probability }`（未 calibration 时 None，禁止 clamp 冒充）
- 附着层：`PitchAttachment { unit_id, nucleus_span, note_ids, primary_note_id, f0_summary, status }`（many-to-many）
- 证据语义：`AlignmentEvidence`（source/quality/score/calibrated_probability）、`TimingPurpose{Acoustic,VisualOnly}`、`NoteReliability`（P1-6 拆分）、`AlignmentQuality{Authoritative,High,Medium,Low,Heuristic,Unknown}`

旧 `ReadingSpan/MoraUnit/ReadingDisplayGroup` 保留为兼容投影，未一次性删除。

## 4. Backend 状态（诚实清单）

| Backend | 状态 | 说明 |
| --- | --- | --- |
| FCPE 连续 F0 | ✅ 生产 | 未改算法/平滑参数（§36：调参放最后） |
| GAME 音符转写 | ✅ 生产（模型存在时） | C3 语言 id、C10 overlap chunking |
| LegacyFcpeNoteTracker | ✅ fallback | canonical 交叉验证失败时不被静默启用（保持 Round3 语义） |
| Enhanced LRC | ✅ 最高优先级 | 显示锚点权威；中文行也保留（1:1 显示组） |
| Legacy MMS-FA | ⚠️ Experimental | 持久 worker 生命周期已实现并经 mock 协议测试；真实推理需用户自备 torch/torchaudio（FA API 2.8 deprecated） |
| Singing FA (HubertFA 体系) | ❌ 骨架 | 能力声明完整，`align()` 诚实返回 `DependencyMissing`（许可审计未完成，见 THIRD_PARTY_MODELS.md） |
| MoraDP | ✅ 应急启发式 | 伪置信度已删；baseline 基准如实记录其水平（见 §6） |
| WeightedFallback | ✅ VisualOnly | 仅卡拉OK高亮，不参与正式 pitch attribution |

## 5. 测试结果

- `cargo test`：**24 个测试二进制全部通过**（0 失败；real-song / round3 acceptance 共 7 项按既有设计 ignored，需 Tier C 环境变量）。
- `npx tsc --noEmit` / `npx vite build`：零错误。
- 新增测试覆盖（Tier A/B/D）：
  - 语言路由：kana→日语、纯汉字歧义 Unknown、工程覆盖>脚本、主导语言继承、中文行零日语假名（单元+集成）
  - GAME：config languages 解析（官方映射/私有映射/缺失/非整数）、chunk 规划、边缘归属、reconcile
  - False precision 四项语义（grace 删除/伪置信度/整行赋时/VisualOnly）各 有专测
  - v4 模型 serde round-trip + 旧工程 JSON 加载
  - FA worker 协议：init-once、批处理 id 对应、无效窗口/发音/语言、非单调/越窗拒绝、init 失败
  - matcher §22 合成矩阵：exact/zero/5ms 容差/1→3 melisma/2→1 共享/并发歧义/促音/无声化/跨模型不一致/无对齐
  - benchmark：3 fixture 全链路 + sanity + 回归门

## 6. Benchmark 结果（基线，合成 fixture，测量 fallback 链路）

| Sample | onset MAE | onset median | P90 | within100 | note F1 | fallback |
| --- | --- | --- | --- | --- | --- | --- |
| ja/sample_001（边界证据） | 186.7ms | 160ms | 400ms | 0.33 | 0.57 | MoraDP 100% |
| ja/sample_002（melisma） | — | 400ms | 400ms | 0.50 | 0.80 | MoraDP 100% |
| zh/sample_001（拼音音节） | — | 90ms | 600ms | 0.67 | 0.57 | MoraDP 100% |

解读（诚实）：当前无 FA 后端，fixture 测的是 **MoraDP 时长先验 + 声学边界证据** 的水平。合成 track 在单位内部无 RMS/flux 变化，且 F0 换音按设计不作为歌词边界证据，因此指标反映的就是任务书判断的"MoraDP 不应再冒充准确逐字对齐"。这些数字作为 **baseline 固化**，后续接入真实 singing FA 后在同一评估器上对比（§21 的 ≤60ms median 目标是对真实 FA 阶段的要求）。melisma 样本 note F1=0.80 验证 many-to-many 保留正确。

## 7. 自审 checklist（任务书 §39，逐项）

1. **中文`我爱你`进入哪个函数？还调 UniDic 吗？** `parse → LanguageRouter.decide_line`；Mandarin 工程→`mandarin::pipeline::build_mandarin_layers`（DictMandarinReadingProvider），Auto 无主导→Unknown 显示层。**不调用 UniDic**（集成测试断言零假名）。
2. **`今日`生成多少 SourceSpan/PronunciationUnit？** 1 个词级 Han SourceSpan；发音层 2 个 unit（きょ/ー，UniDic 词读音）。
3. **是否人为"今=きょ/日=う"？** 否。读音按词展开、unit 挂到整个 span；无逐字伪造（快照+投影测试锁定）。
4. **`がっこう`的`っ`有 pitch 吗？** 无：`JapaneseSokuon` + `PitchBearingPolicy::None` → matcher 返回 `NonPitchBearing`，UI 显示"—"。
5. **`スーパー`的`ー`如何表示？** `JapaneseLongVowel` + `Conditional`，音素 ":"（Vowel 延续）；共享/跨 note 由声学决定，不硬编码。
6. **一个 mora 三个 GAME notes 全部保留？** 是（`note_ids=[1,2,3]`，melisma 测试）。
7. **两个 mora 共用一个 note？** 允许（`two_units_share_one_note` 测试）。
8. **没有 overlap 的 note 还能被抢？** 不能：真实重叠准入 + 15ms 帧量化容差；零重叠→`NoOverlappingNote`；group 级 200ms grace 已删（改写测试）。
9. **MoraDP 还显示 confidence=0.7？** 不：0.0（"无校准概率"）；v4 `AlignmentEvidence` 承载 quality=Heuristic。
10. **FA 失败 token 还得整行时间？** 不：保持无时间+`AlignmentMissing`。
11. **GAME ja/zh id 来自哪？** 模型 `config.json` 的 `languages` 动态读取；测试证明 alternate 映射（ja=7/zh=9）被尊重。
12. **config 无 languages？** 回退 universal id 0 + diagnostic warning。
13. **FA 模型加载几次？** 每首歌一次（worker init），音频 decode/resample 一次（mock 计数测试）。
14. **50 行歌词 50 个 Python？** 不会：单 worker 长生命周期、行请求复用、崩溃重建一次。
15. **WeightedFallback 参与正式 badge？** 不：`VisualTimingOnly`，binder 排除（专测）。
16. **benchmark 报 onset MAE / note F1？** 是（summary JSON + 回归门）。
17. **有 coverage↑ precision↓ 的测试吗？** 回归门 + false_precision 四测（C4 明确接受覆盖率下降换诚实）。
18. **raw FCPE/debug evidence？** debug 导出含逐帧 pitch/flux/RMS、raw_game_notes、canonical decisions、v4 attachments。raw/clean F0 分离（§13.1）未做（见限制）。
19. **旧 ProjectData 能加载？** 能（serde default；无 language 字段旧 JSON 有测试）。
20. **cargo test / 前端 build 全过？** 是（24/24 二进制，tsc/vite 干净）。

## 8. Definition of Done 对照（任务书 §38）

- **Architecture**：✅ 中文不经日语链路；✅ language 传到 GAME；✅ 四层独立（v4 模型+投影）；✅ pitch attachment 基于 pronunciation/acoustic unit（matcher）。
- **False precision**：✅ 200ms 借音删除；✅ 0.7 伪置信度删除；✅ FA 未覆盖不整行赋时；✅ WeightedFallback=VisualOnly。
- **Japanese**：✅ 熟字训不强拆（C1/C6 快照）；✅ っ non-pitch-bearing；✅ ー continuation；✅ pitch 绑定 nucleus（策略/nucleus 范围/matcher 均就绪；**nucleus 时间目前以 mora span 为诚实上界**，phoneme 级时间待 FA）；✅ 1 mora→N notes。
- **Mandarin**：✅ 基础 pipeline 存在；✅ 汉字不再被 UniDic 读成日语；✅ 多音字 Ambiguous/override；✅ tone 非硬约束。
- **Backend**：✅ FA 不再每行重载；✅ capabilities 可查询；✅ fallback source 可见（日志+侧栏+右下角+debug）。
- **Tests**：✅ unit/路由/matcher 合成/gold schema/回归报告；⚠️ "至少若干 JA/ZH real fixture"=仓库内合成 fixture + Tier C 环境变量路径（真人音频不入库）。
- **UI**：✅ Low-confidence/Non-pitch-bearing/AlignmentMissing 可见；✅ backend/fallback 可见。

## 9. 尚未完成项（不伪装完成）

1. **Production singing FA runtime**（HubertFA/LyricFA 接入）：骨架 + 能力声明完成，推理未接（需许可审计结论 + ONNX 模型）；当前生产路径为 EnhancedLRC > Legacy MMS（若配置）> MoraDP。
2. **Mandarin contextual G2P 模型**：当前为保守词典（~180 单字 + 47 词组），未覆盖字诚实 Unknown；未接 OpenJTalk/pypinyin 类外部分析器。
3. **Phoneme 级 nucleus 实测时间**：v4 已定义 `nucleus_span` 与 `nucleus_phoneme_range`，但 mora/音节的声学时间仍来自 DP（nucleus=span 代理），需 phoneme-FA 后回填。
4. **GAME repeated-run stability（§14.3, N=5）**：需要真实 GAME 模型；框架与 fixture 确定性已就绪，Tier C 环境下补充。
5. **GAME chunking 参数 benchmark**（18–20s/1s overlap 的实测调优）：需真实模型。
6. **FCPE raw_f0/clean_f0 分离（§13.1）与平滑调参**：按 §36 顺序排在最后，本轮未动 FCPE。
7. **MoraDP→WeightedFallback 之间的灰色 badge UI（§15.2 灰色样式）**：状态符号已实现，灰 badge 样式细节留 UI 迭代。

## 10. 已知限制

1. **MoraDP 的基准数字较差**（onset MAE 90–600ms 量级）：这正是删除伪置信度后的诚实现状；在有 Enhanced LRC 或真实 FA 的场景不受影响。
2. **ProjectData NaN 序列化**（**存量问题，非本轮引入**）：unvoiced 帧 `f32::NAN` 经 serde_json 写为 `null`，导致含未发声帧的工程文件无法直接回读。C12 已把新指标全部改为 `Option<f32>`；PitchTrack 的迁移（如 `Option<f32>` midis）留待专门的数据格式 commit。
3. **中文 override 无 UI 输入格式**：`MandarinOverride` API + LyricLine 字段就绪，UI（Commit 11 未含中文注音输入）待后续。
4. **MoraDP voiced 窗口裁剪（P1-2）**：按任务书"本轮不精调"保留现状（±80ms margin），其影响已由基准量化。
5. **半角假名分词粒度**：与全角一致的逐字符 token（含浊音记号），显示层行为一致；未做合并优化。
6. **旧任务书 md 删除混入 C2 提交**（见 §1 披露）。

## 11. 复现

```bash
cd src-tauri && cargo test          # 24 个测试二进制全绿
npx tsc --noEmit && npx vite build  # 前端零错误
cargo test --test benchmark_gate_test -- --nocapture   # 基准 + 回归门
```
