# Alignment Gold Benchmark (Round4 任务书 §20)

## Schema (§20.1)

```json
{
  "language": "ja",
  "text": "今日も",
  "line_start": 1.0,
  "line_end": 4.2,
  "units": [
    { "text": "きょ", "start": 1.12, "end": 1.48,
      "nucleus_start": 1.20, "nucleus_end": 1.48, "notes": [64] }
  ]
}
```

- `units` 与 pipeline 的 mora/syllable 序列按序配对。
- `notes` 为该单位的真实 MIDI 集合; 空 = unpitched (促音/无声化/静音)。
- melisma: 一个 unit 多个 note (按时间顺序均分, 用于合成 track)。

## 目录约定

- `ja/` `zh/`: 仓库内合成 fixture (无需外部音频, 由 evaluator 从 gold
  合成确定性 PitchTrack, 测量 MoraDP/legacy note 链路的真实表现)。
- 真实歌曲音频因版权不入库: 通过环境变量加载私有 fixture:
  - `PITCH_ANALYZER_TEST_AUDIO_ROOT` 音频根目录
  - `PITCH_ANALYZER_TEST_GAME_MODEL` GAME ONNX 目录
  - `PITCH_ANALYZER_TEST_FA_MODEL` singing FA 模型目录
  (加载逻辑见 `src-tauri/tests/round3_acceptance.rs` 同款模式)

## 回归门 (Tier D)

`baseline_report.json` 由测试首跑写入并提交; 之后每次运行与基线比较:
onset MAE 退化 > 25ms 或 note F1 下降 > 0.05 → 失败。
