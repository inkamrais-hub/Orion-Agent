//! 多终端抽象层
//!
//! 支持多种终端类型，模型可以按需选择。
//! 处理交互式输入场景，避免终端卡死。

use async_trait::async_trait;
use serde_json::Value;

use super::{Tool, ToolContext, ToolResult};

/// 终端类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShellKind {
    /// PowerShell (Windows 默认)
    PowerShell,
    /// CMD (Windows 命令提示符)
    Cmd,
    /// Bash (Linux/macOS/WSL)
    Bash,
    /// Sh (POSIX shell)
    Sh,
    /// WSL (Windows Subsystem for Linux)
    Wsl,
    /// SSH (远程终端)
    Ssh,
}

impl ShellKind {
    /// 从字符串解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "powershell" | "pwsh" | "ps" => Some(Self::PowerShell),
            "cmd" | "command" => Some(Self::Cmd),
            "bash" => Some(Self::Bash),
            "sh" | "shell" => Some(Self::Sh),
            "wsl" => Some(Self::Wsl),
            "ssh" => Some(Self::Ssh),
            _ => None,
        }
    }

    /// 获取 shell 命令和参数
    pub fn to_command(&self) -> (&'static str, &'static str) {
        match self {
            Self::PowerShell => ("powershell", "-Command"),
            Self::Cmd => ("cmd", "/C"),
            Self::Bash => ("bash", "-c"),
            Self::Sh => ("sh", "-c"),
            Self::Wsl => ("wsl", "-e"),
            Self::Ssh => ("ssh", ""), // SSH 需要特殊处理
        }
    }

    /// 默认终端 (根据平台)
    pub fn default_for_platform() -> Self {
        if cfg!(windows) {
            Self::PowerShell
        } else {
            Self::Bash
        }
    }

    /// 名称
    pub fn name(&self) -> &'static str {
        match self {
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::Wsl => "wsl",
            Self::Ssh => "ssh",
        }
    }
}

/// 交互式输入检测器
pub struct InteractiveDetector;

impl InteractiveDetector {
    /// 检测输出是否需要交互式输入
    pub fn needs_input(output: &str) -> Option<InputRequest> {
        let lower = output.to_lowercase();

        // 密码输入
        if lower.contains("password:")
            || lower.contains("password for")
            || lower.contains("enter password")
            || lower.contains("passphrase")
        {
            return Some(InputRequest::Password);
        }

        // 确认提示 (y/n)
        if lower.contains("[y/n]")
            || lower.contains("[yes/no]")
            || lower.contains("(y/n)?")
            || lower.contains("continue?")
            || lower.contains("are you sure")
        {
            return Some(InputRequest::Confirm);
        }

        // URL 输入
        if lower.contains("enter url")
            || lower.contains("repository url")
            || lower.contains("remote url")
        {
            return Some(InputRequest::Url);
        }

        // API Key 输入
        if lower.contains("api key")
            || lower.contains("token:")
            || lower.contains("access token")
        {
            return Some(InputRequest::ApiKey);
        }

        // 选择菜单
        if lower.contains("select an option")
            || lower.contains("choose")
            || lower.contains("pick one")
        {
            return Some(InputRequest::Selection);
        }

        // 等待输入 (通用)
        if lower.ends_with(": ")
            || lower.ends_with("? ")
            || lower.contains("input:")
            || lower.contains("enter ")
        {
            return Some(InputRequest::Generic);
        }

        None
    }
}

/// 输入请求类型
#[derive(Debug, Clone)]
pub enum InputRequest {
    /// 密码输入
    Password,
    /// 确认 (y/n)
    Confirm,
    /// URL 输入
    Url,
    /// API Key 输入
    ApiKey,
    /// 选择菜单
    Selection,
    /// 通用输入
    Generic,
}

/// 多终端命令执行器
pub struct MultiShellExecutor {
    default_shell: ShellKind,
}

impl Default for MultiShellExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiShellExecutor {
    pub fn new() -> Self {
        Self {
            default_shell: ShellKind::default_for_platform(),
        }
    }

    /// 执行命令
    pub async fn execute(
        &self,
        command: &str,
        shell: Option<ShellKind>,
        input: Option<&str>,
        timeout_secs: u64,
    ) -> crate::Result<ShellOutput> {
        let shell = shell.unwrap_or(self.default_shell);

        // SSH 特殊处理
        if shell == ShellKind::Ssh {
            return Err(crate::Error::Tool(
                "SSH requires connection parameters. Use: ssh user@host 'command'".into()
            ));
        }

        let (shell_cmd, flag) = shell.to_command();

        let mut cmd = tokio::process::Command::new(shell_cmd);
        cmd.arg(flag);

        // 对于 WSL，直接传递命令
        if shell == ShellKind::Wsl {
            cmd.arg(command);
        } else {
            cmd.arg(command);
        }

        // 如果有预设输入，通过 stdin 传入
        if let Some(_input_text) = input {
            cmd.stdin(std::process::Stdio::piped());
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // 设置环境变量
        cmd.env("TERM", "dumb"); // 禁用终端控制序列
        cmd.env("NO_COLOR", "1"); // 禁用颜色

        let start = std::time::Instant::now();

        // 如果有预设输入，需要处理 stdin
        let result = if let Some(input_text) = input {
            let mut child = cmd.spawn()
                .map_err(|e| crate::Error::Tool(format!("Command failed: {}", e)))?;

            // 写入 stdin
            if let Some(mut stdin) = child.stdin.take() {
                let input_bytes = input_text.as_bytes().to_vec();
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(&input_bytes).await;
                    // 关闭 stdin 触发 EOF
                    drop(stdin);
                });
            }

            // 等待完成
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                child.wait_with_output(),
            ).await
        } else {
            tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                cmd.output(),
            ).await
        };

        let elapsed = start.elapsed();

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);

                // 检测是否需要交互式输入
                let needs_input = InteractiveDetector::needs_input(&stdout)
                    .or_else(|| InteractiveDetector::needs_input(&stderr));

                Ok(ShellOutput {
                    stdout,
                    stderr,
                    exit_code,
                    elapsed,
                    shell: shell.name().to_string(),
                    needs_input,
                    timed_out: false,
                })
            }
            Ok(Err(e)) => Err(crate::Error::Tool(format!("Command failed: {}", e))),
            Err(_) => Ok(ShellOutput {
                stdout: String::new(),
                stderr: format!("Command timed out after {}s", timeout_secs),
                exit_code: -1,
                elapsed,
                shell: shell.name().to_string(),
                needs_input: None,
                timed_out: true,
            }),
        }
    }
}

/// Shell 执行结果
#[derive(Debug)]
pub struct ShellOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub elapsed: std::time::Duration,
    pub shell: String,
    pub needs_input: Option<InputRequest>,
    pub timed_out: bool,
}

impl ShellOutput {
    /// 格式化输出
    pub fn format(&self) -> String {
        let mut output = String::new();

        if !self.stdout.is_empty() {
            output.push_str(&self.stdout);
        }

        if !self.stderr.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&format!("[stderr]\n{}", self.stderr));
        }

        if self.exit_code != 0 {
            output.push_str(&format!("\n[exit code: {}]", self.exit_code));
        }

        if self.timed_out {
            output.push_str(&format!("\n[TIMEOUT: command exceeded {}s limit]", self.elapsed.as_secs()));
        }

        if let Some(ref input_request) = self.needs_input {
            output.push_str(&format!(
                "\n[INTERACTIVE: {:?} input detected. Provide input via 'input' parameter.]",
                input_request
            ));
        }

        output.push_str(&format!(
            "\n[shell: {} | time: {:.1}s]",
            self.shell,
            self.elapsed.as_secs_f64()
        ));

        output
    }
}

/// 多终端工具
pub struct MultiShellTool;

#[async_trait]
impl Tool for MultiShellTool {
    fn name(&self) -> &str { "terminal" }

    fn description(&self) -> &str {
        "Execute commands in various terminal environments. \
         Supports: powershell, cmd, bash, sh, wsl. \
         For interactive commands (password, y/n, URL input), use the 'input' parameter \
         to pre-provide input and avoid terminal hanging."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Command to execute"
                },
                "shell": {
                    "type": "string",
                    "description": "Terminal type: powershell (default on Windows), cmd, bash, sh, wsl",
                    "enum": ["powershell", "cmd", "bash", "sh", "wsl"]
                },
                "input": {
                    "type": "string",
                    "description": "Pre-provide input for interactive commands (password, y/n, URL, etc.)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120, max: 600)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let command = input["command"].as_str()
            .ok_or_else(|| crate::Error::Tool("missing 'command' field".into()))?;

        // 安全检查
        if let Err(reason) = crate::core::workspace::is_command_safe(command).await {
            return Ok(ToolResult {
                content: format!("安全拦截: 命令 '{}' 被工作区安全策略拒绝: {}", command, reason),
                is_error: true,
                metadata: None,
            });
        }

        let shell = input["shell"].as_str()
            .and_then(ShellKind::from_str);

        let pre_input = input["input"].as_str();

        let timeout = input["timeout"].as_u64()
            .map(|t| t.clamp(1, 600))
            .unwrap_or(120);

        let executor = MultiShellExecutor::new();
        let output = executor.execute(command, shell, pre_input, timeout).await?;

        Ok(ToolResult {
            content: output.format(),
            is_error: output.exit_code != 0 || output.timed_out,
            metadata: Some(serde_json::json!({
                "exit_code": output.exit_code,
                "shell": output.shell,
                "elapsed_ms": output.elapsed.as_millis(),
                "timed_out": output.timed_out,
                "needs_input": output.needs_input.is_some(),
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_kind_from_str() {
        assert_eq!(ShellKind::from_str("powershell"), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::from_str("pwsh"), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::from_str("cmd"), Some(ShellKind::Cmd));
        assert_eq!(ShellKind::from_str("bash"), Some(ShellKind::Bash));
        assert_eq!(ShellKind::from_str("wsl"), Some(ShellKind::Wsl));
        assert_eq!(ShellKind::from_str("ssh"), Some(ShellKind::Ssh));
        assert_eq!(ShellKind::from_str("unknown"), None);
    }

    #[test]
    fn test_interactive_detector_password() {
        assert!(InteractiveDetector::needs_input("Enter password:").is_some());
        assert!(InteractiveDetector::needs_input("Password for user:").is_some());
    }

    #[test]
    fn test_interactive_detector_confirm() {
        assert!(InteractiveDetector::needs_input("Continue? [y/n]").is_some());
        assert!(InteractiveDetector::needs_input("Are you sure? (y/n)?").is_some());
    }

    #[test]
    fn test_interactive_detector_url() {
        assert!(InteractiveDetector::needs_input("Enter URL:").is_some());
        assert!(InteractiveDetector::needs_input("Repository URL:").is_some());
    }

    #[test]
    fn test_interactive_detector_api_key() {
        assert!(InteractiveDetector::needs_input("Enter API key:").is_some());
        assert!(InteractiveDetector::needs_input("Access token:").is_some());
    }

    #[test]
    fn test_interactive_detector_no_input() {
        assert!(InteractiveDetector::needs_input("Hello world").is_none());
        assert!(InteractiveDetector::needs_input("File not found").is_none());
    }
}
