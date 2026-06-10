//! 审计日志系统 (Layer 2)
//!
//! 记录所有关键操作，支持合规审查
//! 审计事件自动同步到日志系统 (Layer 1)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use crate::{log_info, log_warn, log_error, log_debug};

/// 审计事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEvent {
    SessionStart { session_id: String, model: String },
    SessionEnd { session_id: String, duration_ms: u64 },
    ToolCall { tool_name: String, input_summary: String, success: bool, duration_ms: u64 },
    LlmRequest { model: String, input_tokens: u32, output_tokens: u32 },
    FileOperation { operation: String, path: String, bytes: usize },
    CommandExecution { command: String, exit_code: i32 },
    Error { message: String, source: String },
    ConfigChange { key: String, old_value: String, new_value: String },
    SecurityEvent { event_type: String, details: String },
    /// Hook 执行记录
    HookExecuted { hook_name: String, point: String, action: String, tool_name: String },
}

/// 审计日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub event: AuditEvent,
    pub session_id: Option<String>,
    pub user_id: Option<String>,
    pub source: String,
}

/// 审计日志管理器
pub struct AuditLogger {
    file_path: PathBuf,
    buffer: Vec<AuditEntry>,
    buffer_size: usize,
}

impl AuditLogger {
    pub fn new() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let file_path = PathBuf::from(home).join(".orion").join("audit.jsonl");

        Self {
            file_path,
            buffer: Vec::new(),
            buffer_size: 100,
        }
    }

    /// 记录审计事件 (同时输出到日志系统)
    pub fn log(&mut self, event: AuditEvent, source: impl Into<String>) {
        let source_str = source.into();
        
        // 同步到日志系统
        sync_to_log(&event, &source_str);

        // 写入文件前脱敏
        let safe_event = redact_event(&event);

        let entry = AuditEntry {
            timestamp: Utc::now(),
            event: safe_event,
            session_id: None,
            user_id: None,
            source: source_str,
        };

        self.buffer.push(entry);
        if self.buffer.len() >= self.buffer_size {
            self.flush();
        }
    }

    /// 记录带会话 ID 的审计事件
    pub fn log_with_session(&mut self, event: AuditEvent, source: impl Into<String>, session_id: impl Into<String>) {
        let source_str = source.into();
        let session = session_id.into();
        
        // 同步到日志系统
        sync_to_log(&event, &source_str);

        // 写入文件前脱敏
        let safe_event = redact_event(&event);

        let entry = AuditEntry {
            timestamp: Utc::now(),
            event: safe_event,
            session_id: Some(session),
            user_id: None,
            source: source_str,
        };

        self.buffer.push(entry);
        if self.buffer.len() >= self.buffer_size {
            self.flush();
        }
    }

    /// 刷新缓冲区到文件
    pub fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // 确保目录存在
        if let Some(parent) = self.file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log_error!("audit", "Failed to create audit log directory: {}", e);
                return;
            }
        }

        // 追加写入 JSONL 格式
        let mut content = String::new();
        for entry in &self.buffer {
            if let Ok(json) = serde_json::to_string(entry) {
                content.push_str(&json);
                content.push('\n');
            }
        }

        use std::io::Write;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(content.as_bytes()) {
                    log_error!("audit", "Failed to write audit log: {}", e);
                    return; // 保留 buffer，下次重试
                }
                self.buffer.clear();
            }
            Err(e) => {
                log_error!("audit", "Failed to open audit log file: {}", e);
                // 保留 buffer，下次重试
            }
        }
    }

    /// 查询审计日志
    pub fn query(&self, limit: usize) -> Vec<AuditEntry> {
        if !self.file_path.exists() {
            return Vec::new();
        }

        match std::fs::read_to_string(&self.file_path) {
            Ok(content) => {
                content.lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .rev()
                    .take(limit)
                    .collect()
            }
            Err(e) => {
                log_error!("audit", "Failed to read audit log: {}", e);
                Vec::new()
            }
        }
    }

    /// 按会话查询
    pub fn query_by_session(&self, session_id: &str) -> Vec<AuditEntry> {
        self.query(10_000).into_iter()
            .filter(|e| e.session_id.as_deref() == Some(session_id))
            .collect()
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

/// 写入文件前对敏感事件脱敏
fn redact_event(event: &AuditEvent) -> AuditEvent {
    match event {
        AuditEvent::ConfigChange { key, old_value, new_value } => {
            AuditEvent::ConfigChange {
                key: key.clone(),
                old_value: crate::logging::redact::redact_value(key, old_value),
                new_value: crate::logging::redact::redact_value(key, new_value),
            }
        }
        _ => event.clone(),
    }
}

impl Drop for AuditLogger {
    fn drop(&mut self) {
        self.flush();
    }
}

// ============================================================
//  全局审计日志实例
// ============================================================

/// 全局审计日志 (非阻塞, 带缓冲)
pub static AUDIT_LOGGER: std::sync::LazyLock<tokio::sync::Mutex<AuditLogger>> = std::sync::LazyLock::new(|| {
    tokio::sync::Mutex::new(AuditLogger::new())
});

// ── 审计事件 → 日志同步 ──────────────────────────────────

/// 将审计事件同步到日志系统 (Layer 1)
fn sync_to_log(event: &AuditEvent, source: &str) {
    match event {
        AuditEvent::SessionStart { session_id, model } => {
            log_info!("audit", "[{}] 会话开始: session={}, model={}", source, session_id, model);
        }
        AuditEvent::SessionEnd { session_id, duration_ms } => {
            log_info!("audit", "[{}] 会话结束: session={}, duration={}ms", source, session_id, duration_ms);
        }
        AuditEvent::ToolCall { tool_name, input_summary, success, duration_ms } => {
            if *success {
                log_info!("audit", "[{}] 工具调用: {} ({}ms) - {}", source, tool_name, duration_ms, input_summary);
            } else {
                log_warn!("audit", "[{}] 工具调用失败: {} ({}ms) - {}", source, tool_name, duration_ms, input_summary);
            }
        }
        AuditEvent::LlmRequest { model, input_tokens, output_tokens } => {
            log_debug!("audit", "[{}] LLM请求: model={}, in={}, out={}", source, model, input_tokens, output_tokens);
        }
        AuditEvent::FileOperation { operation, path, bytes } => {
            log_info!("audit", "[{}] 文件操作: {} {} ({} bytes)", source, operation, path, bytes);
        }
        AuditEvent::CommandExecution { command, exit_code } => {
            if *exit_code == 0 {
                log_info!("audit", "[{}] 命令执行: {} (exit={})", source, command, exit_code);
            } else {
                log_warn!("audit", "[{}] 命令失败: {} (exit={})", source, command, exit_code);
            }
        }
        AuditEvent::Error { message, source: err_source } => {
            log_error!("audit", "[{}] 错误: {} (from {})", source, message, err_source);
        }
        AuditEvent::ConfigChange { key, old_value, new_value } => {
            let safe_old = crate::logging::redact::redact_value(key, old_value);
            let safe_new = crate::logging::redact::redact_value(key, new_value);
            log_info!("audit", "[{}] 配置变更: {} = {} -> {}", source, key, safe_old, safe_new);
        }
        AuditEvent::SecurityEvent { event_type, details } => {
            log_warn!("audit", "[{}] 安全事件: {} - {}", source, event_type, details);
        }
        AuditEvent::HookExecuted { hook_name, point, action, tool_name } => {
            log_info!("audit", "[{}] Hook执行: {} ({}) → {} on {}", source, hook_name, point, action, tool_name);
        }
    }
}
