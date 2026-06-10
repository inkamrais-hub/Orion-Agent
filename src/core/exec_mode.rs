//! 执行模式 — 控制 Agent 的自主程度
//!
//! - Assist: 每步都需要用户确认 (最安全)
//! - Auto: 多步自动执行，危险操作前确认 (平衡)
//! - Plan: 只生成计划文本，不执行任何工具 (只读)

use serde::{Deserialize, Serialize};

/// 执行模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExecMode {
    /// 辅助模式: 每个工具调用都需要用户明确确认
    Assist,
    /// 自动模式: 安全工具自动执行，危险工具需要确认
    Auto,
    /// 规划模式: 不执行任何工具，只生成文本计划
    Plan,
}

impl Default for ExecMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl ExecMode {
    /// 是否允许自动执行工具
    pub fn allows_auto_execute(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// 是否需要用户确认每个工具调用
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Assist)
    }

    /// 是否只读模式 (不执行工具)
    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::Plan)
    }

    /// 是否允许执行 bash 命令
    pub fn allows_bash(&self) -> bool {
        !matches!(self, Self::Plan)
    }

    /// 是否允许写操作 (write, edit)
    pub fn allows_write(&self) -> bool {
        !matches!(self, Self::Plan)
    }

    /// 获取模式描述
    pub fn description(&self) -> &str {
        match self {
            Self::Assist => "每步确认，最安全",
            Self::Auto => "自动执行，危险操作前确认",
            Self::Plan => "只生成计划，不执行",
        }
    }

    /// 从字符串解析 (不区分大小写)
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "assist" | "interactive" | "manual" => Self::Assist,
            "auto" | "automatic" | "autonomous" => Self::Auto,
            "plan" | "readonly" | "dry-run" | "dryrun" => Self::Plan,
            _ => Self::Auto,
        }
    }
}

// ============================================================
//  Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_auto() {
        assert_eq!(ExecMode::default(), ExecMode::Auto);
    }

    #[test]
    fn test_allows_auto_execute() {
        assert!(ExecMode::Auto.allows_auto_execute());
        assert!(!ExecMode::Assist.allows_auto_execute());
        assert!(!ExecMode::Plan.allows_auto_execute());
    }

    #[test]
    fn test_requires_confirmation() {
        assert!(ExecMode::Assist.requires_confirmation());
        assert!(!ExecMode::Auto.requires_confirmation());
        assert!(!ExecMode::Plan.requires_confirmation());
    }

    #[test]
    fn test_is_read_only() {
        assert!(ExecMode::Plan.is_read_only());
        assert!(!ExecMode::Auto.is_read_only());
        assert!(!ExecMode::Assist.is_read_only());
    }

    #[test]
    fn test_allows_bash() {
        assert!(ExecMode::Auto.allows_bash());
        assert!(ExecMode::Assist.allows_bash());
        assert!(!ExecMode::Plan.allows_bash());
    }

    #[test]
    fn test_allows_write() {
        assert!(ExecMode::Auto.allows_write());
        assert!(ExecMode::Assist.allows_write());
        assert!(!ExecMode::Plan.allows_write());
    }

    #[test]
    fn test_description_non_empty() {
        assert!(!ExecMode::Assist.description().is_empty());
        assert!(!ExecMode::Auto.description().is_empty());
        assert!(!ExecMode::Plan.description().is_empty());
    }

    #[test]
    fn test_from_str_loose_exact() {
        assert_eq!(ExecMode::from_str_loose("assist"), ExecMode::Assist);
        assert_eq!(ExecMode::from_str_loose("auto"), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose("plan"), ExecMode::Plan);
    }

    #[test]
    fn test_from_str_loose_aliases() {
        assert_eq!(ExecMode::from_str_loose("interactive"), ExecMode::Assist);
        assert_eq!(ExecMode::from_str_loose("manual"), ExecMode::Assist);
        assert_eq!(ExecMode::from_str_loose("automatic"), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose("autonomous"), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose("readonly"), ExecMode::Plan);
        assert_eq!(ExecMode::from_str_loose("dry-run"), ExecMode::Plan);
        assert_eq!(ExecMode::from_str_loose("dryrun"), ExecMode::Plan);
    }

    #[test]
    fn test_from_str_loose_case_insensitive() {
        assert_eq!(ExecMode::from_str_loose("ASSIST"), ExecMode::Assist);
        assert_eq!(ExecMode::from_str_loose("Auto"), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose("PLAN"), ExecMode::Plan);
    }

    #[test]
    fn test_from_str_loose_unknown_defaults_to_auto() {
        assert_eq!(ExecMode::from_str_loose("unknown"), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose(""), ExecMode::Auto);
        assert_eq!(ExecMode::from_str_loose("banana"), ExecMode::Auto);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mode = ExecMode::Assist;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, r#""assist""#);
        let parsed: ExecMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ExecMode::Assist);

        let mode2 = ExecMode::Plan;
        let json2 = serde_json::to_string(&mode2).unwrap();
        assert_eq!(json2, r#""plan""#);
        let parsed2: ExecMode = serde_json::from_str(&json2).unwrap();
        assert_eq!(parsed2, ExecMode::Plan);
    }

    #[test]
    fn test_clone_and_eq() {
        let mode = ExecMode::Auto;
        let cloned = mode.clone();
        assert_eq!(mode, cloned);
    }

    #[test]
    fn test_debug_format() {
        let s = format!("{:?}", ExecMode::Auto);
        assert_eq!(s, "Auto");
    }
}
