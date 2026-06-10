pub mod registry;
pub mod code_intelligence;
pub mod ask_user;
pub mod agent_tool;
pub mod mcp;
pub mod a2a_message;
pub mod edit;
pub mod glob_tool;
pub mod grep_tool;
pub mod spec;
pub mod web_search;
pub mod category;
pub mod multi_shell;
pub mod mcp_init;
pub mod meta_tools;
pub mod skeleton_tool;
pub mod docker_executor;

use async_trait::async_trait;
use serde_json::Value;
use crate::audit::{AuditEvent as GlobalAuditEvent, AUDIT_LOGGER};

// ============================================================
//  积木: Tool (工具系统)
//  职责: 定义工具接口, 每个工具是独立积木
// ============================================================

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
    pub metadata: Option<Value>,
}

/// Tool trait — 实现此 trait 可注册为工具
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// JSON Schema 格式的输入描述
    fn input_schema(&self) -> Value;

    /// 执行工具
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult>;
}

/// 工具执行上下文
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub working_dir: String,
    pub turn_number: u64,
    pub agent_id: String,
    pub registry: Option<std::sync::Arc<crate::agent::registry::AgentRegistry>>,
}

// ============================================================
//  内置: 基础工具积木
// ============================================================

/// 文件读取工具
pub struct ReadTool;

/// 文件读取最大字节数 (防止 token 浪费)
const READ_MAX_BYTES: u64 = 256 * 1024; // 256KB (~64K tokens)

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }
    fn description(&self) -> &str {
        "Read file contents. Max 256KB per read. For large files, use offset/limit to read specific sections. \
         Supports utf-8 (default) and binary (hex dump) encoding. \
         IMPORTANT: Always read a file before editing or writing to it. \
         Use offset/limit for large files to avoid token waste."
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute file path" },
                "offset": { "type": "integer", "description": "Line number to start from (1-based, optional)" },
                "limit": { "type": "integer", "description": "Max lines to read (optional, default: entire file or until 256KB)" },
                "encoding": { "type": "string", "description": "File encoding: 'utf-8' (default) or 'binary' for hex dump (optional)" }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let path = input["path"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'path' field".into())
        })?;
        let encoding = input["encoding"].as_str().unwrap_or("utf-8");

        // ── Binary 模式: 返回 hex dump ──
        if encoding == "binary" {
            let bytes = tokio::fs::read(path).await
                .map_err(|e| crate::Error::Tool(format!("Cannot read '{}': {}", path, e)))?;
            let max_bytes = (READ_MAX_BYTES as usize).min(bytes.len());
            let hex_lines: Vec<String> = bytes[..max_bytes].chunks(16)
                .enumerate()
                .map(|(i, chunk)| {
                    let hex: Vec<String> = chunk.iter().map(|b| format!("{:02x}", b)).collect();
                    let ascii: String = chunk.iter()
                        .map(|&b| if (0x20..=0x7e).contains(&b) { b as char } else { '.' })
                        .collect();
                    format!("{:08x}  {:<47}  {}", i * 16, hex.join(" "), ascii)
                })
                .collect();
            let mut content = hex_lines.join("\n");
            if bytes.len() > max_bytes {
                content.push_str(&format!("\n\n[Hex dump truncated: showing {}/{} bytes]", max_bytes, bytes.len()));
            }
            return Ok(ToolResult {
                content,
                is_error: false,
                metadata: Some(serde_json::json!({"path": path, "size": bytes.len(), "encoding": "binary"})),
            });
        }

        // 1. 查文件缓存 (mtime 匹配, 同步)
        if input.get("offset").is_none() && input.get("limit").is_none() {
            if let Some(cached) = crate::core::cache::file_cache_get(path) {
                return Ok(ToolResult {
                    content: cached,
                    is_error: false,
                    metadata: Some(serde_json::json!({"cached": true, "path": path})),
                });
            }
        }

        // 2. 检查文件大小
        let meta = tokio::fs::metadata(path).await
            .map_err(|e| crate::Error::Tool(format!("Cannot access '{}': {}", path, e)))?;
        let file_size = meta.len();

        // 3. 读取 (支持 offset/limit 行范围)
        let offset = input["offset"].as_u64().map(|v| v.max(1) as usize);
        let limit = input["limit"].as_u64().map(|v| v.max(1) as usize);

        let content = if file_size > READ_MAX_BYTES && offset.is_none() && limit.is_none() {
            // 大文件: 只读前部分 + 提示
            let raw = tokio::fs::read_to_string(path).await?;
            let truncated = if raw.len() > READ_MAX_BYTES as usize {
                let mut end = READ_MAX_BYTES as usize;
                while end > 0 && !raw.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...[truncated, showing {}/{} bytes]", &raw[..end], end, raw.len())
            } else {
                raw.clone()
            };
            let total_lines = raw.lines().count();
            format!("{}\n\n[File truncated: {} bytes, {} total lines. Use offset/limit to read specific sections.]",
                truncated, file_size, total_lines)
        } else {
            let raw = tokio::fs::read_to_string(path).await?;
            match (offset, limit) {
                (Some(off), Some(lim)) => {
                    raw.lines().skip(off - 1).take(lim).collect::<Vec<_>>().join("\n")
                }
                (Some(off), None) => {
                    raw.lines().skip(off - 1).collect::<Vec<_>>().join("\n")
                }
                (None, Some(lim)) => {
                    raw.lines().take(lim).collect::<Vec<_>>().join("\n")
                }
                (None, None) => raw,
            }
        };

        // 4. 写缓存 (仅全量读取时)
        if offset.is_none() && limit.is_none() && file_size <= READ_MAX_BYTES {
            crate::core::cache::file_cache_set(path, content.clone());
        }

        Ok(ToolResult {
            content,
            is_error: false,
            metadata: Some(serde_json::json!({"path": path, "size": file_size})),
        })
    }
}

/// 注册默认工具集 (run 和 chat 共用)
///
/// 包含: Read, Write, Bash, Edit, SymbolSearch, FindCallers, ProjectMap,
///       AskUser, Glob, Grep
pub fn register_default_tools(tools: &mut registry::ToolRegistry) {
    tools.register(ReadTool);
    tools.register(WriteTool);
    tools.register(BashTool);
    tools.register(edit::EditTool);
    tools.register(code_intelligence::SymbolSearchTool);
    tools.register(code_intelligence::FindCallersTool);
    tools.register(code_intelligence::ProjectMapTool);
    tools.register(ask_user::AskUserTool);
    tools.register(glob_tool::GlobTool);
    tools.register(grep_tool::GrepTool);
    tools.register(skeleton_tool::SkeletonTool);
}

/// 注册元工具（延迟装载模式专用）
///
/// 注册 `load_tool` 和 `list_categories` 到注册表，并将它们自身激活。
/// 调用前需确保 registry 已启用 `enable_lazy_mode()`。
pub fn register_meta_tools(
    tools: &mut registry::ToolRegistry,
    weak_ref: std::sync::Weak<registry::ToolRegistry>,
) {
    tools.register(meta_tools::LoadToolTool::new(weak_ref.clone()));
    tools.register(meta_tools::ListCategoriesTool::new(weak_ref));
    // 元工具自身必须始终可见
    tools.activate("load_tool");
    tools.activate("list_categories");
}

/// 文件写入工具
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }
    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories if needed. \
         WARNING: This overwrites the entire file. For partial edits, use the 'edit' tool instead. \
         ALWAYS read the file first to understand its structure before writing."
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let path = input["path"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'path' field".into())
        })?;
        let content = input["content"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'content' field".into())
        })?;

        // 工作区安全检查
        if let Err(e) = crate::core::workspace::can_write_file(std::path::Path::new(path)).await {
            return Ok(ToolResult {
                content: e,
                is_error: true,
                metadata: None,
            });
        }

        // Create parent directories if they don't exist
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await
                    .map_err(|e| crate::Error::Tool(format!("Cannot create directory '{}': {}", parent.display(), e)))?;
            }
        }

        tokio::fs::write(path, content).await?;
        // 更新文件缓存 (写入新内容，而非仅失效)
        crate::core::cache::file_cache_set(path, content.to_string());
        let size = content.len();
        let exists = std::path::Path::new(path).parent().map(|p| p.exists()).unwrap_or(false);

        // 审计: 文件写入
        {
            let mut logger = AUDIT_LOGGER.lock().await;
            logger.log(GlobalAuditEvent::FileOperation {
                operation: "write".to_string(),
                path: path.to_string(),
                bytes: size,
            }, "tool");
        }

        Ok(ToolResult {
            content: format!("Written {} bytes to {}", size, path),
            is_error: false,
            metadata: Some(serde_json::json!({"size": size, "path": path, "parent_exists": exists})),
        })
    }
}

/// Bash 执行工具
pub struct BashTool;

/// Bash 输出最大字节数 (防止内存爆炸)
const BASH_MAX_OUTPUT_BYTES: usize = 1024 * 1024; // 1MB
/// Bash 命令超时秒数
const BASH_TIMEOUT_SECS: u64 = 120;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str {
        "Execute a shell command. Default timeout: 120s. Output truncated at 1MB. \
         Use for: running tests, building, git, file operations, package management. \
         Chain commands with && for sequential execution. \
         Risk levels: Safe(ls,cat,echo) | Low(git,cargo,npm) | Medium(rm,kill) | High(sudo) | Critical(rm -rf / — blocked). \
         On Windows, commands run in PowerShell. Use 'terminal' tool for CMD/WSL/SSH."
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout": { "type": "integer", "description": "Command timeout in seconds (default: 120, max: 600)" }
            },
            "required": ["command"]
        })
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let cmd = input["command"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'command' field".into())
        })?;

        // 解析 timeout (默认 120s, 最大 600s)
        let timeout_secs = input["timeout"].as_u64()
            .map(|t| t.clamp(1, 600))
            .unwrap_or(BASH_TIMEOUT_SECS);

        // 工作区安全检查
        if let Err(e) = crate::core::workspace::is_command_safe(cmd).await {
            return Ok(ToolResult {
                content: e,
                is_error: true,
                metadata: None,
            });
        }

        // ── 风险分级检查 ──────────────────────────────────────
        let risk = crate::core::r#loop::classify_bash_risk(cmd);
        if risk == crate::core::r#loop::BashRisk::Critical {
            return Ok(ToolResult {
                content: format!(
                    "Command blocked: critical risk detected ({:?}). Command: {}",
                    risk, cmd
                ),
                is_error: true,
                metadata: Some(serde_json::json!({
                    "risk_level": "critical",
                    "blocked": true,
                })),
            });
        }
        // High 及以下继续执行

        // ── Docker 沙箱模式 ──────────────────────────────────
        let config = crate::config::OrionConfig::load_cached();
        if config.docker.enabled {
            use docker_executor::{DockerExecutor, DockerExecutorConfig};

            // Docker 不可用时优雅降级到本地模式
            if !DockerExecutor::is_available().await {
                tracing::warn!("Docker enabled but not available, falling back to local execution");
            } else {
                let executor = DockerExecutor::new(DockerExecutorConfig {
                    image: config.docker.image.clone(),
                    workdir: config.docker.workdir.clone(),
                    auto_pull: config.docker.auto_pull,
                    network: config.docker.network.clone(),
                    memory_limit: config.docker.memory_limit.clone(),
                    cpu_limit: config.docker.cpu_limit.clone(),
                });

                return match executor.execute(cmd, timeout_secs).await {
                    Ok(output) => {
                        let mut result = String::new();
                        if !output.stdout.is_empty() {
                            result.push_str(&output.stdout);
                        }
                        if !output.stderr.is_empty() {
                            if !result.is_empty() { result.push_str("\n--- stderr ---\n"); }
                            result.push_str(&output.stderr);
                        }

                        // 截断过长输出
                        if result.len() > BASH_MAX_OUTPUT_BYTES {
                            let mut end = BASH_MAX_OUTPUT_BYTES;
                            while end > 0 && !result.is_char_boundary(end) {
                                end -= 1;
                            }
                            let truncated = &result[..end];
                            result = format!("{}\n\n[Output truncated, showing {}/{} bytes. Use more specific commands to reduce output.]", truncated, end, result.len());
                        }

                        if risk == crate::core::r#loop::BashRisk::High {
                            result = format!(
                                "[WARNING] High-risk command detected. Executing in Docker sandbox.\n{}",
                                result
                            );
                        }

                        let risk_str = format!("{:?}", risk);
                        Ok(ToolResult {
                            content: result,
                            is_error: output.exit_code != 0,
                            metadata: Some(serde_json::json!({
                                "exit_code": output.exit_code,
                                "mode": "docker",
                                "risk_level": risk_str,
                            })),
                        })
                    }
                    Err(e) => Ok(ToolResult {
                        content: format!("Docker execution error: {}", e),
                        is_error: true,
                        metadata: Some(serde_json::json!({"mode": "docker"})),
                    }),
                };
            }
        }

        // ── 本地执行模式 ─────────────────────────────────────
        // Windows 上用 PowerShell 处理 Unicode 路径
        // PowerShell 原生支持 Unicode，比 cmd 更可靠
        let cmd_owned: String;
        let (shell, flag, actual_cmd) = if cfg!(windows) {
            // 用 PowerShell 执行，原生支持 Unicode 路径
            cmd_owned = cmd.to_string();
            ("powershell", "-Command", cmd_owned.as_str())
        } else {
            ("sh", "-c", cmd)
        };

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new(shell)
                .arg(flag)
                .arg(actual_cmd)
                // 强制 UTF-8 输出
                .env("PYTHONIOENCODING", "utf-8")
                .env("LANG", "en_US.UTF-8")
                .output(),
        ).await;

        let output = match output {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Ok(ToolResult {
                content: format!("Command failed to start: {}", e),
                is_error: true,
                metadata: None,
            }),
            Err(_) => return Ok(ToolResult {
                content: format!("Command timed out after {}s", timeout_secs),
                is_error: true,
                metadata: None,
            }),
        };

        let mut content = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).to_string()
        } else {
            format!(
                "Exit code: {}\n{}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr)
            )
        };

        // 截断过长输出
        if content.len() > BASH_MAX_OUTPUT_BYTES {
            let mut end = BASH_MAX_OUTPUT_BYTES;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = &content[..end];
            content = format!("{}\n\n[Output truncated, showing {}/{} bytes. Use more specific commands to reduce output.]", truncated, end, content.len());
        }

        let exit_code = output.status.code().unwrap_or(-1);
        let cwd = std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();

        // High 风险: 在结果中附加警告
        if risk == crate::core::r#loop::BashRisk::High {
            content = format!(
                "[WARNING] High-risk command detected. Executing in autonomous mode.\n{}",
                content
            );
        }

        let risk_str = format!("{:?}", risk);
        Ok(ToolResult {
            content,
            is_error: !output.status.success(),
            metadata: Some(serde_json::json!({
                "exit_code": exit_code,
                "cwd": cwd,
                "risk_level": risk_str,
            })),
        })
    }
}
