//! Permission Broker — 统一安全决策点
//!
//! 将散落在多处的安全检查 (ExecPolicy, GuardrailChain, orionignore)
//! 合并为单一决策接口, 消除安全模型碎片化。

use crate::core::execpolicy::{Decision as PolicyDecision, ExecPolicy};
use crate::core::guardrail::{GuardResult, GuardrailChain, TurnContext};
use crate::core::orionignore;
use std::path::Path;

/// 统一安全决策结果
#[derive(Debug, Clone)]
pub enum SecurityDecision {
    /// 允许执行
    Allow,
    /// 拒绝执行 (含原因)
    Deny { reason: String },
    /// 跳过 (不执行, 但不报错)
    Skip,
}

impl SecurityDecision {
    /// 是否为允许
    pub fn is_allowed(&self) -> bool {
        matches!(self, SecurityDecision::Allow)
    }

    /// 是否为拒绝
    pub fn is_denied(&self) -> bool {
        matches!(self, SecurityDecision::Deny { .. })
    }
}

/// 文件操作工具名列表 (用于识别需要 orionignore 检查的工具)
const FILE_TOOLS: &[&str] = &["read", "write", "edit", "glob", "grep"];

/// Permission Broker 配置
///
/// 将 ExecPolicy (命令安全策略)、GuardrailChain (护栏链) 和 orionignore
/// (文件忽略规则) 整合为统一决策点。
pub struct PermissionBroker {
    exec_policy: Option<ExecPolicy>,
    guardrails: Option<GuardrailChain>,
    /// 是否启用 .orionignore 检查
    respect_orionignore: bool,
    /// 是否保护敏感文件 (.env, .pem, .key 等)
    protect_sensitive_files: bool,
}

impl PermissionBroker {
    /// 创建默认配置的 PermissionBroker
    ///
    /// 默认启用 orionignore 和敏感文件保护，不含 ExecPolicy 和 GuardrailChain。
    pub fn new() -> Self {
        Self {
            exec_policy: None,
            guardrails: None,
            respect_orionignore: true,
            protect_sensitive_files: true,
        }
    }

    /// 设置 ExecPolicy (命令安全策略)
    pub fn with_exec_policy(mut self, policy: ExecPolicy) -> Self {
        self.exec_policy = Some(policy);
        self
    }

    /// 设置 GuardrailChain (护栏链)
    pub fn with_guardrails(mut self, chain: GuardrailChain) -> Self {
        self.guardrails = Some(chain);
        self
    }

    /// 设置是否尊重 .orionignore 规则
    pub fn with_orionignore(mut self, enabled: bool) -> Self {
        self.respect_orionignore = enabled;
        self
    }

    /// 设置是否保护敏感文件
    pub fn with_sensitive_file_protection(mut self, enabled: bool) -> Self {
        self.protect_sensitive_files = enabled;
        self
    }

    /// 工具执行前的统一安全检查
    ///
    /// 按以下顺序依次执行所有启用的检查:
    /// 1. GuardrailChain (护栏链) — 权限/预算/自定义护栏
    /// 2. ExecPolicy (命令安全策略) — 仅针对 `bash` 工具
    /// 3. orionignore (文件忽略规则) — 仅针对文件操作工具
    ///
    /// 任一检查拒绝或跳过，则立即返回对应决策。
    pub async fn check_tool_execution(
        &self,
        turn_ctx: &TurnContext,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> SecurityDecision {
        // 1. GuardrailChain 检查
        if let Some(ref chain) = self.guardrails {
            match chain.check_tool(turn_ctx, tool_name, input).await {
                GuardResult::Allow => {}
                GuardResult::Deny(reason) => {
                    return SecurityDecision::Deny {
                        reason: format!("guardrail denied: {}", reason),
                    };
                }
                GuardResult::Skip => return SecurityDecision::Skip,
            }
        }

        // 2. ExecPolicy 检查 (仅 bash 工具)
        if tool_name == "bash" {
            if let Some(ref policy) = self.exec_policy {
                if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                    match policy.check(cmd) {
                        PolicyDecision::Allow => {}
                        PolicyDecision::Forbid => {
                            return SecurityDecision::Deny {
                                reason: format!("command blocked by exec policy: {}", cmd),
                            };
                        }
                    }
                }
            }
        }

        // 3. orionignore 检查 (仅文件操作工具)
        if self.respect_orionignore && is_file_tool(tool_name) {
            if let Some(path_str) = extract_file_path(input) {
                let path = Path::new(&path_str);
                if let Some(reason) = orionignore::should_ignore_with_reason(path) {
                    return SecurityDecision::Deny {
                        reason: format!("file ignored: {}", reason),
                    };
                }
                // 敏感文件额外保护
                if self.protect_sensitive_files
                    && orionignore::is_sensitive_file_path(&path_str)
                {
                    return SecurityDecision::Deny {
                        reason: format!("access to sensitive file denied: {}", path_str),
                    };
                }
            }
        }

        SecurityDecision::Allow
    }

    /// 文件访问的统一安全检查
    ///
    /// 检查顺序:
    /// 1. orionignore 规则 (若已启用)
    /// 2. 敏感文件保护 (若已启用)
    pub fn check_file_access(&self, path: &str) -> SecurityDecision {
        // 1. orionignore 检查
        if self.respect_orionignore {
            let p = Path::new(path);
            if let Some(reason) = orionignore::should_ignore_with_reason(p) {
                return SecurityDecision::Deny {
                    reason: format!("file ignored: {}", reason),
                };
            }
        }

        // 2. 敏感文件保护
        if self.protect_sensitive_files && orionignore::is_sensitive_file_path(path) {
            return SecurityDecision::Deny {
                reason: format!("access to sensitive file denied: {}", path),
            };
        }

        SecurityDecision::Allow
    }

    /// 检查路径是否为敏感文件
    ///
    /// 识别 `.env`、`.pem`、`.key`、`credentials.*`、`id_rsa` 等含有凭据或
    /// 密钥的文件。
    pub fn is_sensitive_file(path: &str) -> bool {
        orionignore::is_sensitive_file_path(path)
    }
}

impl Default for PermissionBroker {
    fn default() -> Self {
        Self::new()
    }
}

// ── 内部辅助函数 ──────────────────────────────────────────────

/// 判断工具名是否为文件操作工具
fn is_file_tool(tool_name: &str) -> bool {
    FILE_TOOLS.contains(&tool_name)
}

/// 从工具输入 JSON 中提取文件路径
///
/// 支持 `path`、`file_path` 和 `pattern` (glob) 字段。
fn extract_file_path(input: &serde_json::Value) -> Option<String> {
    // 尝试常见路径字段
    input
        .get("path")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("pattern"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ── 测试 ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_defaults() {
        let broker = PermissionBroker::new();
        assert!(broker.exec_policy.is_none());
        assert!(broker.guardrails.is_none());
        assert!(broker.respect_orionignore);
        assert!(broker.protect_sensitive_files);
    }

    #[test]
    fn test_default_trait() {
        let broker = PermissionBroker::default();
        assert!(broker.respect_orionignore);
        assert!(broker.protect_sensitive_files);
    }

    #[test]
    fn test_is_sensitive_file() {
        assert!(PermissionBroker::is_sensitive_file(".env"));
        assert!(PermissionBroker::is_sensitive_file("config/.env.production"));
        assert!(PermissionBroker::is_sensitive_file("certs/server.pem"));
        assert!(PermissionBroker::is_sensitive_file("server.key"));
        assert!(PermissionBroker::is_sensitive_file("config/credentials.json"));
        assert!(!PermissionBroker::is_sensitive_file("src/main.rs"));
        assert!(!PermissionBroker::is_sensitive_file("Cargo.toml"));
    }

    #[test]
    fn test_check_file_access_allows_normal() {
        let broker = PermissionBroker::new();
        let decision = broker.check_file_access("src/main.rs");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_check_file_access_denies_env() {
        let broker = PermissionBroker::new();
        let decision = broker.check_file_access(".env");
        assert!(decision.is_denied());
    }

    #[test]
    fn test_check_file_access_denies_credentials() {
        let broker = PermissionBroker::new();
        let decision = broker.check_file_access("config/credentials.json");
        assert!(decision.is_denied());
    }

    #[test]
    fn test_check_file_access_orionignore_disabled() {
        let broker = PermissionBroker::new().with_orionignore(false);
        // 构建产物在 orionignore 中，禁用后应允许 (但敏感文件仍受保护)
        let decision = broker.check_file_access("target/debug/build");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_check_file_access_sensitive_disabled() {
        let broker = PermissionBroker::new().with_sensitive_file_protection(false);
        // 敏感文件保护禁用后, .env 仍被 orionignore 拦截
        let decision = broker.check_file_access(".env");
        assert!(decision.is_denied());
    }

    #[test]
    fn test_check_file_access_both_disabled() {
        let broker = PermissionBroker::new()
            .with_orionignore(false)
            .with_sensitive_file_protection(false);
        let decision = broker.check_file_access(".env");
        assert!(decision.is_allowed());
    }

    #[test]
    fn test_is_file_tool() {
        assert!(is_file_tool("read"));
        assert!(is_file_tool("write"));
        assert!(is_file_tool("glob"));
        assert!(!is_file_tool("bash"));
        assert!(!is_file_tool("web_search"));
    }

    #[test]
    fn test_extract_file_path() {
        let input = serde_json::json!({"path": "src/main.rs"});
        assert_eq!(extract_file_path(&input), Some("src/main.rs".to_string()));

        let input = serde_json::json!({"file_path": "src/lib.rs"});
        assert_eq!(extract_file_path(&input), Some("src/lib.rs".to_string()));

        let input = serde_json::json!({"pattern": "*.rs"});
        assert_eq!(extract_file_path(&input), Some("*.rs".to_string()));

        let input = serde_json::json!({"command": "ls"});
        assert_eq!(extract_file_path(&input), None);
    }

    #[test]
    fn test_security_decision_helpers() {
        let allow = SecurityDecision::Allow;
        assert!(allow.is_allowed());
        assert!(!allow.is_denied());

        let deny = SecurityDecision::Deny {
            reason: "test".into(),
        };
        assert!(!deny.is_allowed());
        assert!(deny.is_denied());

        let skip = SecurityDecision::Skip;
        assert!(!skip.is_allowed());
        assert!(!skip.is_denied());
    }
}
