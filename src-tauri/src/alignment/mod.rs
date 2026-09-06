// Alignment module (Round4 任务书 §5/§12)
//
// 以后真正的 orchestration layer:
//   - projection: 旧三层文本结构 → v4 canonical 数据模型 (Commit 5)
//   - note_matcher: pronunciation/acoustic unit ↔ GAME note 匹配 (Commit 9)
//   - pipeline / diagnostics: 后续 commit 接入

pub mod benchmark;
pub mod note_matcher;
pub mod projection;
