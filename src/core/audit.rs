//! 审查 (Audit) 日志系统
//!
//! 记录 Agent 的每一个操作: 模型输出、工具调用、护栏决策、压缩事件
//! 不可篡改: 追加写入 (append-only)
//! 格式: JSON Lines (.jsonl), 每行一个事件
//!
//! 使用:
//!   let audit = AuditLogger::new("session_123.jsonl")?;
//!   audit.record(AuditEvent::ToolCall { ... }).await?;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use std::path::Path;

// ============================================================
//  审查事件类型
// ============================================================

/// 审查事件 — 所有 Agent 操作的统一记录格式
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AuditEvent {
    /// 用户输入
    UserInput {
        session_id: String,
        content: String,
        timestamp: DateTime<Utc>,
    },

    /// 模型输出
    ModelOutput {
        session_id: String,
        turn: u64,
        content: String,
        input_tokens: u64,
        output_tokens: u64,
        model: String,
        timestamp: DateTime<Utc>,
    },

    /// 工具调用
    ToolCall {
        session_id: String,
        turn: u64,
        tool_name: String,
        input: serde_json::Value,
        output: String,
        is_error: bool,
        duration_ms: u64,
        cache_hit: bool,
        timestamp: DateTime<Utc>,
    },

    /// 护栏决策 (拦截/放行/跳过)
    GuardrailDecision {
        session_id: String,
        turn: u64,
        guardrail_name: String,
        tool_name: String,
        decision: String,        // "allow" | "deny" | "skip"
        reason: Option<String>,
        timestamp: DateTime<Utc>,
    },

    /// 上下文压缩事件
    ContextCompaction {
        session_id: String,
        turn: u64,
        strategy: String,
        messages_before: usize,
        messages_after: usize,
        tokens_freed: u64,
        success: bool,
        timestamp: DateTime<Utc>,
    },

    /// Agent 间通信 (A2A)
    AgentToAgent {
        from: String,
        to: String,
        content_len: usize,
        timestamp: DateTime<Utc>,
    },

    /// 错误
    Error {
        session_id: String,
        turn: u64,
        error: String,
        timestamp: DateTime<Utc>,
    },

    /// Session 开始
    SessionStart {
        session_id: String,
        model: String,
        timestamp: DateTime<Utc>,
    },

    /// Session 结束
    SessionEnd {
        session_id: String,
        turn_count: u64,
        total_tokens: u64,
        duration_secs: u64,
        timestamp: DateTime<Utc>,
    },
}

impl AuditEvent {
    #[allow(dead_code)]
    fn timestamp(&self) -> DateTime<Utc> {
        match self {
            AuditEvent::UserInput { timestamp, .. } => *timestamp,
            AuditEvent::ModelOutput { timestamp, .. } => *timestamp,
            AuditEvent::ToolCall { timestamp, .. } => *timestamp,
            AuditEvent::GuardrailDecision { timestamp, .. } => *timestamp,
            AuditEvent::ContextCompaction { timestamp, .. } => *timestamp,
            AuditEvent::AgentToAgent { timestamp, .. } => *timestamp,
            AuditEvent::Error { timestamp, .. } => *timestamp,
            AuditEvent::SessionStart { timestamp, .. } => *timestamp,
            AuditEvent::SessionEnd { timestamp, .. } => *timestamp,
        }
    }
}

// ============================================================
//  审查日志写入器
// ============================================================

/// 审查日志 — 追加写入 JSON Lines 文件
pub struct AuditLogger {
    file: tokio::io::BufWriter<tokio::fs::File>,
    path: String,
    written: u64,
}

impl AuditLogger {
    /// 创建审查日志 (追加模式)
    pub async fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
            .await?;

        let path_str = path.as_ref().to_string_lossy().to_string();
        Ok(Self {
            file: tokio::io::BufWriter::new(file),
            path: path_str,
            written: 0,
        })
    }

    /// 记录一条事件
    pub async fn record(&mut self, event: AuditEvent) -> std::io::Result<()> {
        let line = serde_json::to_string(&event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        self.file.write_all(line.as_bytes()).await?;
        self.file.write_all(b"\n").await?;
        self.file.flush().await?;
        self.written += 1;
        Ok(())
    }

    /// 已写入事件数
    pub fn count(&self) -> u64 {
        self.written
    }

    /// 日志文件路径
    pub fn path(&self) -> &str {
        &self.path
    }
}

// ============================================================
//  无操作审查器 (关闭审查功能时使用)
// ============================================================

/// 无操作审查器 — 所有 record 都静默丢弃
pub struct NullAuditLogger;

impl Default for NullAuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl NullAuditLogger {
    pub fn new() -> Self {
        Self
    }

    pub async fn record(&mut self, _event: AuditEvent) -> std::io::Result<()> {
        Ok(()) // 什么都不做
    }

    pub fn count(&self) -> u64 { 0 }
    pub fn path(&self) -> &str { "/dev/null" }
}
