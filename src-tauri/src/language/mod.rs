// Language Router (Round4 Commit 2, P0-1 修复)
//
// 歌词解析只做语法解析; 语言处理由本模块路由:
//
//   parse → LanguageRouter → JapanesePipeline | MandarinPipeline | GenericFallback
//
// 目标架构 (任务书 §6):
//   user project override > line explicit language metadata(未来) > script heuristics
//
// 脚本启发式 (任务书 §6.2):
//   - 出现假名 → 高概率 Japanese
//   - Han + 无假名 → AmbiguousHan, 不直接认定 Mandarin
//   - AmbiguousHan 继承工程主导语言; 无主导语言时保持 Unknown (不悄悄选)
//   - Latin-only → Unknown/Generic

pub mod router;

pub use router::{LanguageDecisionReason, LanguageRouter, LineLanguageDecision, ScriptFeatures};
