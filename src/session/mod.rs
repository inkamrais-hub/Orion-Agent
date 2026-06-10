//! Session 管理 — 统一存储层
//!
//! 所有持久化操作通过 `UnifiedStore` 完成（单一 SQLite 数据库）。
//! 旧的 SessionManager (JSONL) / SessionStore / AgentStore 已废弃，
//! 保留仅供向后兼容，新代码请统一使用 `UnifiedStore`。

pub mod manager;
pub mod memory;
pub mod store;
pub mod unified;
pub mod files;
pub mod rollout;
pub mod sandbox;

// ── 统一存储重导出 ──────────────────────────────────────────
pub use unified::UnifiedStore;

// ── 旧类型（仅 manager 模块内部使用，新代码请使用 store::* 或 unified::*）──

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Session 元数据 (旧版，仅供 SessionManager 兼容使用)
#[deprecated(since = "0.2.0", note = "Use session::store::SessionMeta or UnifiedStore instead")]
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

#[deprecated(since = "0.2.0", note = "Use session::store::SessionStatus instead")]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
}

#[allow(deprecated)]
impl SessionStatus {
    /// 转换为 store::SessionStatus
    pub fn to_store_status(&self) -> crate::session::store::SessionStatus {
        match self {
            SessionStatus::Active => crate::session::store::SessionStatus::Active,
            SessionStatus::Completed => crate::session::store::SessionStatus::Completed,
            SessionStatus::Failed => crate::session::store::SessionStatus::Failed,
        }
    }
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
