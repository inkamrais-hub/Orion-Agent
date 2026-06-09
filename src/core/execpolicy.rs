//! execpolicy — 可配置的安全策略引擎
//!
//! Codex 设计: 策略写在配置文件里，不写死在代码里
//!
//! 配置文件: .orion/execpolicy.yaml
//! ```yaml
//! rules:
//!   - program: "cargo"
//!     prefix: ["build", "test", "run"]
//!     decision: allow
//!   - program: "rm"
//!     prefix: ["-rf", "/"]
//!     decision: forbid
//!   - program: "*"
//!     prefix: ["src/main.rs"]
//!     decision: forbid
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 决策类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Forbid,
}

/// 单条规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// 程序名 (如 "cargo", "rm", "*" 表示所有)
    pub program: String,
    /// 命令前缀列表
    pub prefix: Vec<String>,
    /// 决策
    pub decision: Decision,
    /// 可选描述
    pub description: Option<String>,
}

/// 策略配置文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecPolicy {
    pub rules: Vec<Rule>,
}

impl ExecPolicy {
    /// 从文件加载策略
    pub fn load(path: &PathBuf) -> crate::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let policy: ExecPolicy = serde_yaml::from_str(&content)?;
        Ok(policy)
    }

    /// 从默认路径加载 (.orion/execpolicy.yaml)
    pub fn load_default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = cwd.join(".orion").join("execpolicy.yaml");
        
        if path.exists() {
            match Self::load(&path) {
                Ok(policy) => return policy,
                Err(e) => {
                    log_warn!("execpolicy", "加载策略文件失败: {}", e);
                }
            }
        }

        // 默认策略
        Self::default_policy()
    }

    /// 默认安全策略
    fn default_policy() -> Self {
        Self {
            rules: vec![
                // 允许常见命令
                Rule {
                    program: "cargo".into(),
                    prefix: vec!["build".into(), "test".into(), "run".into(), "check".into()],
                    decision: Decision::Allow,
                    description: Some("Cargo 常用命令".into()),
                },
                Rule {
                    program: "git".into(),
                    prefix: vec!["status".into(), "log".into(), "diff".into(), "add".into(), "commit".into()],
                    decision: Decision::Allow,
                    description: Some("Git 常用命令".into()),
                },
                Rule {
                    program: "dir".into(),
                    prefix: vec![],
                    decision: Decision::Allow,
                    description: Some("Windows dir 命令".into()),
                },
                Rule {
                    program: "ls".into(),
                    prefix: vec![],
                    decision: Decision::Allow,
                    description: Some("Linux ls 命令".into()),
                },
                Rule {
                    program: "cat".into(),
                    prefix: vec![],
                    decision: Decision::Allow,
                    description: Some("Linux cat 命令".into()),
                },
                Rule {
                    program: "type".into(),
                    prefix: vec![],
                    decision: Decision::Allow,
                    description: Some("Windows type 命令".into()),
                },
                // 禁止危险命令
                Rule {
                    program: "rm".into(),
                    prefix: vec!["-rf".into(), "/".into()],
                    decision: Decision::Forbid,
                    description: Some("禁止递归删除根目录".into()),
                },
                Rule {
                    program: "format".into(),
                    prefix: vec![],
                    decision: Decision::Forbid,
                    description: Some("禁止格式化磁盘".into()),
                },
                Rule {
                    program: "del".into(),
                    prefix: vec!["/s".into(), "/q".into()],
                    decision: Decision::Forbid,
                    description: "禁止静默递归删除".to_string().into(),
                },
            ],
        }
    }

    /// 检查命令是否允许
    pub fn check(&self, command: &str) -> Decision {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Decision::Allow;
        }

        let program = parts[0];
        let args = &parts[1..];

        // 按顺序检查规则，第一个匹配的生效
        for rule in &self.rules {
            if self.matches_rule(rule, program, args) {
                return rule.decision.clone();
            }
        }

        // 默认允许
        Decision::Allow
    }

    fn matches_rule(&self, rule: &Rule, program: &str, args: &[&str]) -> bool {
        // 程序名匹配
        if rule.program != "*" && rule.program != program {
            return false;
        }

        // 前缀匹配 (空 prefix = 匹配所有)
        if rule.prefix.is_empty() {
            return true;
        }

        // 检查参数是否包含任何前缀
        let args_str = args.join(" ");
        rule.prefix.iter().any(|p| args_str.contains(p.as_str()))
    }

    /// 获取允许的命令前缀 (给 LLM 提示)
    pub fn allowed_prefixes(&self) -> Vec<String> {
        self.rules.iter()
            .filter(|r| r.decision == Decision::Allow)
            .map(|r| {
                if r.prefix.is_empty() {
                    format!("{} (任意参数)", r.program)
                } else {
                    format!("{} {}", r.program, r.prefix.join("|"))
                }
            })
            .collect()
    }

    /// 获取禁止的命令前缀
    pub fn forbidden_prefixes(&self) -> Vec<String> {
        self.rules.iter()
            .filter(|r| r.decision == Decision::Forbid)
            .map(|r| {
                if r.prefix.is_empty() {
                    format!("{} (任意参数)", r.program)
                } else {
                    format!("{} {}", r.program, r.prefix.join("|"))
                }
            })
            .collect()
    }
}

// ── 依赖 ──────────────────────────────────────────────────

use crate::log_warn;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = ExecPolicy::default_policy();
        assert!(policy.rules.len() > 0);
    }

    #[test]
    fn test_cargo_build_allowed() {
        let policy = ExecPolicy::default_policy();
        assert_eq!(policy.check("cargo build"), Decision::Allow);
        assert_eq!(policy.check("cargo test"), Decision::Allow);
    }

    #[test]
    fn test_rm_rf_forbidden() {
        let policy = ExecPolicy::default_policy();
        assert_eq!(policy.check("rm -rf /"), Decision::Forbid);
    }

    #[test]
    fn test_unknown_command_allowed() {
        let policy = ExecPolicy::default_policy();
        assert_eq!(policy.check("echo hello"), Decision::Allow);
    }
}
