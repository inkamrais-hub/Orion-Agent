//! Session 管理 — SQLite 持久化
//!
//! 功能:
//! - Session 创建/查询/恢复
//! - 消息历史持久化 (每轮对话)
//! - 工具调用记录
//! - 审计报告
//! - 快照接口预留

pub mod manager;
pub mod memory;
pub mod store;
pub mod files;
pub mod rollout;
pub mod sandbox;

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Session 元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub model: String,
    pub turn_count: u64,
    pub total_tokens: u64,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
}

/// 对话消息条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    pub timestamp: DateTime<Utc>,
}
