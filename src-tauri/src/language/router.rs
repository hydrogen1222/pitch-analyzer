// 语言路由决策实现 (Round4 Commit 2)
//
// 纯函数决策, 不做任何 I/O; 日语链路只在决策为 Japanese 时才被调用。
// Mandarin 行在本 commit 返回 Unknown 处理路径 (不走 UniDic);
// 真正的 MandarinPipeline 在 Round4 Commit 7 接入。

use crate::models::{ProjectLanguage, SingingLanguage};

/// 行内脚本特征 (诊断与路由共用)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScriptFeatures {
    pub has_kana: bool,
    pub has_han: bool,
    pub has_ascii_alnum: bool,
    pub has_visible: bool,
}

pub fn scan_script(text: &str) -> ScriptFeatures {
    let mut f = ScriptFeatures::default();
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        f.has_visible = true;
        if crate::japanese::mora::is_kana_char(c) || c == '\u{30FC}' {
            f.has_kana = true;
        } else if is_han_char(c) {
            f.has_han = true;
        } else if c.is_ascii_alphanumeric() {
            f.has_ascii_alnum = true;
        }
    }
    f
}

fn is_han_char(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineScript {
    Empty,
    Kana,
    HanOnly,
    Other,
}

fn classify(script: ScriptFeatures) -> LineScript {
    if !script.has_visible {
        LineScript::Empty
    } else if script.has_kana {
        LineScript::Kana
    } else if script.has_han {
        LineScript::HanOnly
    } else {
        LineScript::Other
    }
}

/// 为什么得出这个语言 (诊断/测试可见, 不进 UI 主路径)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageDecisionReason {
    /// 用户工程语言覆盖 (最高优先级)
    ProjectOverride,
    /// 脚本证据: 行内出现假名
    ScriptKana,
    /// 纯汉字行继承工程主导语言
    InheritedProjectDominant,
    /// 纯汉字且无法判定 (无主导语言) —— 保持 Unknown, 不悄悄选语言
    HanOnlyAmbiguous,
    /// 拉丁/符号等无 CJK 证据
    LatinOrOther,
    /// 空行
    Empty,
}

/// 行级语言决策结果。`language` 永远是具体语言, 不会是 Auto。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineLanguageDecision {
    pub language: SingingLanguage,
    pub reason: LanguageDecisionReason,
}

pub struct LanguageRouter;

impl LanguageRouter {
    /// 单行决策。`project_dominant` 由 `detect_project_dominant` 对整首
    /// 歌词计算一次后传入, 避免逐行重复扫描。
    pub fn decide_line(
        &self,
        primary_text: &str,
        project: ProjectLanguage,
        project_dominant: Option<SingingLanguage>,
    ) -> LineLanguageDecision {
        let script = classify(scan_script(primary_text));
        if script == LineScript::Empty {
            return LineLanguageDecision {
                language: SingingLanguage::Unknown,
                reason: LanguageDecisionReason::Empty,
            };
        }
        // 1. 用户工程覆盖 (P0-1: 不允许再悄悄进入日语词典)
        match project {
            ProjectLanguage::Japanese => {
                return LineLanguageDecision {
                    language: SingingLanguage::Japanese,
                    reason: LanguageDecisionReason::ProjectOverride,
                };
            }
            ProjectLanguage::Mandarin => {
                return LineLanguageDecision {
                    language: SingingLanguage::Mandarin,
                    reason: LanguageDecisionReason::ProjectOverride,
                };
            }
            ProjectLanguage::Auto | ProjectLanguage::Mixed => {}
        }
        // 2. 脚本启发式
        match script {
            LineScript::Kana => LineLanguageDecision {
                language: SingingLanguage::Japanese,
                reason: LanguageDecisionReason::ScriptKana,
            },
            LineScript::HanOnly => {
                // Mixed 工程: 用户已声明两种语言并存, 纯汉字行仍歧义。
                // Auto 工程: 仅当工程主导语言存在时继承。
                if project == ProjectLanguage::Auto {
                    if let Some(dominant) = project_dominant {
                        return LineLanguageDecision {
                            language: dominant,
                            reason: LanguageDecisionReason::InheritedProjectDominant,
                        };
                    }
                }
                LineLanguageDecision {
                    language: SingingLanguage::Unknown,
                    reason: LanguageDecisionReason::HanOnlyAmbiguous,
                }
            }
            _ => LineLanguageDecision {
                language: SingingLanguage::Unknown,
                reason: LanguageDecisionReason::LatinOrOther,
            },
        }
    }

    /// 项目主导语言。只能从脚本证据检测出日语:
    /// 假名行数量必须严格多于纯汉字行 (任务书 §6.2 "整首多数歌词含 kana")。
    /// 纯汉字项目无法从脚本推断 Mandarin —— 返回 None, 由 UI 提示用户确认。
    pub fn detect_project_dominant(&self, primary_texts: &[&str]) -> Option<SingingLanguage> {
        let mut kana_lines = 0usize;
        let mut han_only_lines = 0usize;
        for text in primary_texts {
            match classify(scan_script(text)) {
                LineScript::Kana => kana_lines += 1,
                LineScript::HanOnly => han_only_lines += 1,
                _ => {}
            }
        }
        if kana_lines > 0 && kana_lines > han_only_lines {
            Some(SingingLanguage::Japanese)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_features_scan() {
        let f = scan_script("今日(きょう)");
        assert!(f.has_kana && f.has_han && f.has_visible);
        let f = scan_script("我爱你");
        assert!(!f.has_kana && f.has_han);
        let f = scan_script("Hello 123");
        assert!(!f.has_kana && !f.has_han && f.has_ascii_alnum);
        let f = scan_script("   ");
        assert!(!f.has_visible);
    }

    #[test]
    fn kana_line_routes_japanese() {
        let d = LanguageRouter.decide_line("きみと歩く", ProjectLanguage::Auto, None);
        assert_eq!(d.language, SingingLanguage::Japanese);
        assert_eq!(d.reason, LanguageDecisionReason::ScriptKana);
    }

    #[test]
    fn han_only_without_dominant_stays_unknown() {
        // 任务书 §6.2: 不要悄悄选一种语言
        let d = LanguageRouter.decide_line("我爱你", ProjectLanguage::Auto, None);
        assert_eq!(d.language, SingingLanguage::Unknown);
        assert_eq!(d.reason, LanguageDecisionReason::HanOnlyAmbiguous);
    }

    #[test]
    fn han_only_inherits_japanese_dominant() {
        let d = LanguageRouter.decide_line("未来", ProjectLanguage::Auto, Some(SingingLanguage::Japanese));
        assert_eq!(d.language, SingingLanguage::Japanese);
        assert_eq!(d.reason, LanguageDecisionReason::InheritedProjectDominant);
    }

    #[test]
    fn project_override_beats_script() {
        // 用户显式 Mandarin: 即使行内出现假名也不进入日语链路
        let d = LanguageRouter.decide_line("ラララ", ProjectLanguage::Mandarin, None);
        assert_eq!(d.language, SingingLanguage::Mandarin);
        assert_eq!(d.reason, LanguageDecisionReason::ProjectOverride);
        let d = LanguageRouter.decide_line("我爱你", ProjectLanguage::Japanese, None);
        assert_eq!(d.language, SingingLanguage::Japanese);
    }

    #[test]
    fn mixed_project_keeps_han_ambiguous() {
        let d = LanguageRouter.decide_line("未来", ProjectLanguage::Mixed, Some(SingingLanguage::Japanese));
        assert_eq!(d.language, SingingLanguage::Unknown);
        assert_eq!(d.reason, LanguageDecisionReason::HanOnlyAmbiguous);
        let d = LanguageRouter.decide_line("きみ", ProjectLanguage::Mixed, None);
        assert_eq!(d.language, SingingLanguage::Japanese);
    }

    #[test]
    fn latin_routes_unknown() {
        let d = LanguageRouter.decide_line("Hello World", ProjectLanguage::Auto, None);
        assert_eq!(d.language, SingingLanguage::Unknown);
        assert_eq!(d.reason, LanguageDecisionReason::LatinOrOther);
    }

    #[test]
    fn empty_routes_unknown() {
        let d = LanguageRouter.decide_line("　", ProjectLanguage::Auto, None);
        assert_eq!(d.reason, LanguageDecisionReason::Empty);
    }

    #[test]
    fn dominant_detection_majority_rule() {
        let router = LanguageRouter;
        assert_eq!(
            router.detect_project_dominant(&["きみと", "歩く", "未来"]),
            Some(SingingLanguage::Japanese)
        );
        // 假名行不占多数: 不判定主导语言
        assert_eq!(router.detect_project_dominant(&["我", "爱", "ら"]), None);
        // 纯汉字项目: 无法从脚本推断 Mandarin
        assert_eq!(router.detect_project_dominant(&["我", "爱", "你"]), None);
    }
}
