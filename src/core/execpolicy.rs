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
use std::path::{Path, PathBuf};

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
    /// Default decision for commands that match no rule.
    ///
    /// SECURITY WARNING: Setting this to `Allow` means any unrecognized command
    /// will be permitted. For stricter security, set to `Forbid` and explicitly
    /// allow only known-safe commands. The current default (`Allow`) preserves
    /// backward compatibility but may be too permissive for production use.
    #[serde(default = "default_allow")]
    pub default_policy: Decision,
    /// 沙箱模式: 禁止所有网络操作和 VCS 写操作
    ///
    /// 开启后，以下命令将被无条件拦截 (优先于用户规则):
    /// - git push / git remote add / git fetch
    /// - curl / wget / Invoke-WebRequest
    /// - ssh / scp / rsync
    /// - 任何涉及网络 I/O 的命令
    #[serde(default)]
    pub sandbox: bool,
}

fn default_allow() -> Decision {
    Decision::Allow
}

/// 沙箱模式下无条件禁止的程序列表
const SANDBOX_FORBIDDEN_PROGRAMS: &[&str] = &[
    // VCS 写操作
    "git-push", "git-remote", "git-fetch", "git-clone",
    // 网络工具
    "curl", "wget", "invoke-webrequest", "ssh", "scp", "rsync",
    "ftp", "sftp", "nc", "ncat", "netcat", "telnet",
    // 包管理器网络操作
    "pip", "pip3", "npm", "yarn", "pnpm", "cargo-install",
    // 远程控制
    "powershell.exe",  // 防止通过 PowerShell 绕过 (Invoke-WebRequest 等)
];

/// 沙箱模式下禁止的 git 子命令
const SANDBOX_FORBIDDEN_GIT_SUBCOMMANDS: &[&str] = &[
    "push", "remote", "fetch", "clone", "pull", "submodule",
];

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
            // SECURITY WARNING: The default is Allow for backward compatibility.
            // Consider changing to Forbid in production environments for a
            // deny-by-default posture, explicitly allowing only safe commands.
            default_policy: Decision::Allow,
            sandbox: false,
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
                    prefix: vec!["status".into(), "log".into(), "diff".into(), "branch".into()],
                    decision: Decision::Allow,
                    description: Some("Git 只读命令".into()),
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

    /// 创建沙箱模式策略 (网络隔离 + VCS 写保护)
    pub fn sandbox_policy() -> Self {
        let mut policy = Self::default_policy();
        policy.sandbox = true;
        policy
    }

    /// 启用/禁用沙箱模式
    pub fn set_sandbox(&mut self, enabled: bool) {
        self.sandbox = enabled;
    }

    /// 检查命令是否允许
    pub fn check(&self, command: &str) -> Decision {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return self.default_policy.clone();
        }

        // ── 沙箱模式: 优先检查网络/VCS 黑名单 ──────────────
        if self.sandbox {
            if let Some(reason) = self.check_sandbox(&parts) {
                tracing::warn!(command = command, reason = %reason, "Sandbox blocked command");
                return Decision::Forbid;
            }
        }

        let program = parts[0];
        let args = &parts[1..];

        // 按顺序检查规则，第一个匹配的生效
        for rule in &self.rules {
            if self.matches_rule(rule, program, args) {
                return rule.decision.clone();
            }
        }

        // 没有匹配的规则，使用默认策略
        self.default_policy.clone()
    }

    /// 沙箱检查: 返回 Some(reason) 表示应拦截
    fn check_sandbox(&self, parts: &[&str]) -> Option<String> {
        let program = parts[0];
        let program_lower = program.to_lowercase();
        let program_name = Path::new(&program_lower).file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&program_lower);

        // 检查禁止的程序
        for forbidden in SANDBOX_FORBIDDEN_PROGRAMS {
            if program_name == *forbidden || program_lower == *forbidden {
                return Some(format!("sandbox: '{}' is blocked (network operation)", program));
            }
        }

        // 特殊处理 git: 只允许只读子命令
        if (program_name == "git" || program_lower == "git")
            && parts.len() > 1 {
                let subcmd = parts[1].to_lowercase();
                for forbidden_sub in SANDBOX_FORBIDDEN_GIT_SUBCOMMANDS {
                    if subcmd == *forbidden_sub {
                        return Some(format!("sandbox: 'git {}' is blocked (VCS write operation)", subcmd));
                    }
                }
            }

        // 检查 PowerShell 网络 cmdlet (通过 powershell -Command "...")
        if program_name == "powershell" || program_name == "powershell.exe"
            || program_name == "pwsh" || program_name == "pwsh.exe"
        {
            let full_cmd = parts.join(" ").to_lowercase();
            let net_cmdlets = [
                "invoke-webrequest", "invoke-restmethod", "new-pssession",
                "enter-pssession", "start-bitstransfer", "send-mailmessage",
                "system.net.webclient", "system.net.http.httpclient",
            ];
            for cmdlet in &net_cmdlets {
                if full_cmd.contains(cmdlet) {
                    return Some(format!("sandbox: PowerShell network cmdlet '{}' blocked", cmdlet));
                }
            }
        }

        // 检查 bash/sh -c 中的嵌套网络命令
        if program_name == "bash" || program_name == "sh" || program_name == "cmd"
            || program_name == "cmd.exe"
        {
            let full_cmd = parts.join(" ").to_lowercase();
            for net_prog in &["curl", "wget", "git push", "git clone", "git fetch", "git pull"] {
                if full_cmd.contains(net_prog) {
                    return Some(format!("sandbox: nested '{}' in shell blocked", net_prog));
                }
            }
        }

        None
    }

    fn matches_rule(&self, rule: &Rule, program: &str, args: &[&str]) -> bool {
        // 程序名匹配 (also compare file name component to handle full paths like /usr/bin/rm)
        let program_name = Path::new(program).file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(program);
        if rule.program != "*" && rule.program != program && rule.program != program_name {
            return false;
        }

        // 前缀匹配 (空 prefix = 匹配所有)
        if rule.prefix.is_empty() {
            return true;
        }

        // Check each argument individually against each prefix pattern.
        // This prevents bypasses like `rm -r -f /` evading a rule for `["-rf", "/"]`
        // which would happen if args were joined into a single string first.
        rule.prefix.iter().any(|p| args.iter().any(|a| *a == p.as_str() || a.starts_with(p.as_str())))
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

    #[test]
    fn test_sandbox_blocks_git_push() {
        let policy = ExecPolicy::sandbox_policy();
        assert_eq!(policy.check("git push"), Decision::Forbid);
        assert_eq!(policy.check("git push origin main"), Decision::Forbid);
        assert_eq!(policy.check("git clone https://example.com/repo"), Decision::Forbid);
        assert_eq!(policy.check("git fetch --all"), Decision::Forbid);
    }

    #[test]
    fn test_sandbox_allows_git_readonly() {
        let policy = ExecPolicy::sandbox_policy();
        assert_eq!(policy.check("git status"), Decision::Allow);
        assert_eq!(policy.check("git log --oneline"), Decision::Allow);
        assert_eq!(policy.check("git diff"), Decision::Allow);
        assert_eq!(policy.check("git branch"), Decision::Allow);
    }

    #[test]
    fn test_sandbox_blocks_network_tools() {
        let policy = ExecPolicy::sandbox_policy();
        assert_eq!(policy.check("curl https://example.com"), Decision::Forbid);
        assert_eq!(policy.check("wget https://example.com/file"), Decision::Forbid);
        assert_eq!(policy.check("ssh user@host"), Decision::Forbid);
        assert_eq!(policy.check("scp file user@host:/path"), Decision::Forbid);
    }

    #[test]
    fn test_sandbox_blocks_nested_shell_commands() {
        let policy = ExecPolicy::sandbox_policy();
        assert_eq!(policy.check("bash -c \"curl https://example.com\""), Decision::Forbid);
        assert_eq!(policy.check("sh -c \"git push origin main\""), Decision::Forbid);
    }

    #[test]
    fn test_sandbox_allows_build_tools() {
        let policy = ExecPolicy::sandbox_policy();
        assert_eq!(policy.check("cargo build"), Decision::Allow);
        assert_eq!(policy.check("cargo test"), Decision::Allow);
        assert_eq!(policy.check("cargo check"), Decision::Allow);
        assert_eq!(policy.check("ls -la"), Decision::Allow);
        assert_eq!(policy.check("cat file.rs"), Decision::Allow);
    }

    #[test]
    fn test_non_sandbox_allows_git_push() {
        let policy = ExecPolicy::default_policy();
        // 非沙箱模式下 git push 走 default_policy = Allow
        assert_eq!(policy.check("git push"), Decision::Allow);
    }
}
