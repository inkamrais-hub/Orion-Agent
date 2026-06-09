//! Session SQLite 持久化存储
//!
//! 功能:
//! - Session 创建/查询/更新/删除
//! - 消息历史持久化 (每轮对话)
//! - 审计关联 (session_id ↔ event_id)
//! - 快照接口预留

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use chrono::{DateTime, Utc};

// ── Session 类型 ──────────────────────────────────────────

/// Session 状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Deleted,
}

/// Session 元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub agent_name: String,
    pub model: String,
    pub working_dir: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub turn_count: u32,
    pub tool_call_count: u32,
    pub total_tokens: u64,
}

impl SessionMeta {
    pub fn status_str(&self) -> &'static str {
        match self.status {
            SessionStatus::Active => "活跃",
            SessionStatus::Paused => "暂停",
            SessionStatus::Completed => "完成",
            SessionStatus::Failed => "失败",
            SessionStatus::Deleted => "已删除",
        }
    }
}

/// 消息轮次
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub turn_id: String,
    pub session_id: String,
    pub turn_index: u32,
    pub role: String,  // "user" / "assistant" / "system"
    pub content: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub created_at: DateTime<Utc>,
    pub tokens_used: u32,
}

/// 工具调用记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub call_id: String,
    pub tool_name: String,
    pub input_summary: String,
    pub output_summary: String,
    pub success: bool,
    pub duration_ms: u64,
}

/// 快照记录 (预留接口)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub snapshot_type: String,  // "pre_session" / "pre_tool" / "manual"
    pub created_at: DateTime<Utc>,
    pub file_hashes: std::collections::HashMap<String, String>,
    pub metadata: String,  // JSON 扩展字段
}

// ── Session Store ─────────────────────────────────────────

/// SQLite Session 存储
pub struct SessionStore {
    conn: Mutex<Connection>,
}

impl SessionStore {
    /// 创建新的 Session Store
    pub fn new(db_path: &PathBuf) -> crate::Result<Self> {
        // 确保目录存在
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_tables()?;
        Ok(store)
    }

    /// 在内存中创建 (用于测试)
    pub fn in_memory() -> crate::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_tables()?;
        Ok(store)
    }

    /// 初始化数据库表
    fn init_tables(&self) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                agent_name TEXT NOT NULL,
                model TEXT NOT NULL,
                working_dir TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                turn_count INTEGER NOT NULL DEFAULT 0,
                tool_call_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS turns (
                turn_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                created_at TEXT NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );

            CREATE TABLE IF NOT EXISTS tool_calls (
                call_id TEXT PRIMARY KEY,
                turn_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                input_summary TEXT NOT NULL,
                output_summary TEXT NOT NULL,
                success INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                FOREIGN KEY (turn_id) REFERENCES turns(turn_id),
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );

            CREATE TABLE IF NOT EXISTS snapshots (
                snapshot_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                snapshot_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                file_hashes TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );

            CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_turn ON tool_calls(turn_id);
            CREATE INDEX IF NOT EXISTS idx_snapshots_session ON snapshots(session_id);
        ")?;

        Ok(())
    }

    // ── Session 操作 ────────────────────────────────────

    /// 创建 Session
    pub fn create_session(&self, meta: &SessionMeta) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, agent_name, model, working_dir, status, created_at, updated_at, turn_count, tool_call_count, total_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                meta.session_id,
                meta.agent_name,
                meta.model,
                meta.working_dir,
                serde_json::to_string(&meta.status).unwrap_or_default(),
                meta.created_at.to_rfc3339(),
                meta.updated_at.to_rfc3339(),
                meta.turn_count,
                meta.tool_call_count,
                meta.total_tokens,
            ],
        )?;
        Ok(())
    }

    /// 获取 Session
    pub fn get_session(&self, session_id: &str) -> crate::Result<Option<SessionMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status, created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions WHERE session_id = ?1"
        )?;

        let mut rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionMeta {
                session_id: row.get(0)?,
                agent_name: row.get(1)?,
                model: row.get(2)?,
                working_dir: row.get(3)?,
                status: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(SessionStatus::Active),
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap_or_default().with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .unwrap_or_default().with_timezone(&Utc),
                turn_count: row.get(7)?,
                tool_call_count: row.get(8)?,
                total_tokens: row.get(9)?,
            })
        })?;

        match rows.next() {
            Some(Ok(meta)) => Ok(Some(meta)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// 列出最近的 Session
    pub fn list_sessions(&self, limit: u32) -> crate::Result<Vec<SessionMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status, created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions ORDER BY updated_at DESC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit], |row| {
            Ok(SessionMeta {
                session_id: row.get(0)?,
                agent_name: row.get(1)?,
                model: row.get(2)?,
                working_dir: row.get(3)?,
                status: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(SessionStatus::Active),
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                    .unwrap_or_default().with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .unwrap_or_default().with_timezone(&Utc),
                turn_count: row.get(7)?,
                tool_call_count: row.get(8)?,
                total_tokens: row.get(9)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// 更新 Session 状态
    pub fn update_session_status(&self, session_id: &str, status: SessionStatus) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE session_id = ?3",
            params![serde_json::to_string(&status).unwrap_or_default(), Utc::now().to_rfc3339(), session_id],
        )?;
        Ok(())
    }

    /// 更新 Session 统计
    pub fn update_session_stats(&self, session_id: &str, turn_count: u32, tool_call_count: u32, total_tokens: u64) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET turn_count = ?1, tool_call_count = ?2, total_tokens = ?3, updated_at = ?4 WHERE session_id = ?5",
            params![turn_count, tool_call_count, total_tokens, Utc::now().to_rfc3339(), session_id],
        )?;
        Ok(())
    }

    // ── Turn 操作 ───────────────────────────────────────

    /// 保存 Turn
    pub fn save_turn(&self, turn: &Turn) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO turns (turn_id, session_id, turn_index, role, content, thinking, created_at, tokens_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                turn.turn_id,
                turn.session_id,
                turn.turn_index,
                turn.role,
                turn.content,
                turn.thinking,
                turn.created_at.to_rfc3339(),
                turn.tokens_used,
            ],
        )?;
        Ok(())
    }

    /// 获取 Session 的所有 Turn
    pub fn get_turns(&self, session_id: &str) -> crate::Result<Vec<Turn>> {
        let turn_ids: Vec<(String, u32, String, String, Option<String>, String, u32)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT turn_id, turn_index, role, content, thinking, created_at, tokens_used
                 FROM turns WHERE session_id = ?1 ORDER BY turn_index"
            )?;

            let rows = stmt.query_map(params![session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,  // turn_id
                    row.get::<_, u32>(1)?,     // turn_index
                    row.get::<_, String>(2)?,  // role
                    row.get::<_, String>(3)?,  // content
                    row.get::<_, Option<String>>(4)?, // thinking
                    row.get::<_, String>(5)?,  // created_at
                    row.get::<_, u32>(6)?,     // tokens_used
                ))
            })?;

            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            result
        };

        // 在锁外填充 tool_calls
        let mut turns = Vec::new();
        for (turn_id, turn_index, role, content, thinking, created_at, tokens_used) in turn_ids {
            turns.push(Turn {
                turn_id: turn_id.clone(),
                session_id: session_id.to_string(),
                turn_index,
                role,
                content,
                thinking,
                tool_calls: self.get_tool_calls_for_turn(&turn_id)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default().with_timezone(&Utc),
                tokens_used,
            });
        }
        Ok(turns)
    }

    // ── ToolCall 操作 ───────────────────────────────────

    /// 保存工具调用
    pub fn save_tool_call(&self, call: &ToolCallRecord, turn_id: &str, session_id: &str) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tool_calls (call_id, turn_id, session_id, tool_name, input_summary, output_summary, success, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                call.call_id,
                turn_id,
                session_id,
                call.tool_name,
                call.input_summary,
                call.output_summary,
                call.success as i32,
                call.duration_ms,
            ],
        )?;
        Ok(())
    }

    /// 获取 Turn 的工具调用
    fn get_tool_calls_for_turn(&self, turn_id: &str) -> crate::Result<Vec<ToolCallRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT call_id, tool_name, input_summary, output_summary, success, duration_ms
             FROM tool_calls WHERE turn_id = ?1"
        )?;

        let rows = stmt.query_map(params![turn_id], |row| {
            Ok(ToolCallRecord {
                call_id: row.get(0)?,
                tool_name: row.get(1)?,
                input_summary: row.get(2)?,
                output_summary: row.get(3)?,
                success: row.get::<_, i32>(4)? != 0,
                duration_ms: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// 获取 Session 的所有工具调用
    pub fn get_tool_calls(&self, session_id: &str) -> crate::Result<Vec<ToolCallRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT call_id, tool_name, input_summary, output_summary, success, duration_ms
             FROM tool_calls WHERE session_id = ?1 ORDER BY call_id"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(ToolCallRecord {
                call_id: row.get(0)?,
                tool_name: row.get(1)?,
                input_summary: row.get(2)?,
                output_summary: row.get(3)?,
                success: row.get::<_, i32>(4)? != 0,
                duration_ms: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Snapshot 操作 (预留接口) ────────────────────────

    /// 保存快照 (预留)
    pub fn save_snapshot(&self, snapshot: &Snapshot) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO snapshots (snapshot_id, session_id, snapshot_type, created_at, file_hashes, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.snapshot_id,
                snapshot.session_id,
                snapshot.snapshot_type,
                snapshot.created_at.to_rfc3339(),
                serde_json::to_string(&snapshot.file_hashes).unwrap_or_default(),
                snapshot.metadata,
            ],
        )?;
        Ok(())
    }

    /// 获取 Session 的快照 (预留)
    pub fn get_snapshots(&self, session_id: &str) -> crate::Result<Vec<Snapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT snapshot_id, session_id, snapshot_type, created_at, file_hashes, metadata
             FROM snapshots WHERE session_id = ?1 ORDER BY created_at"
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(Snapshot {
                snapshot_id: row.get(0)?,
                session_id: row.get(1)?,
                snapshot_type: row.get(2)?,
                created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                    .unwrap_or_default().with_timezone(&Utc),
                file_hashes: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                metadata: row.get(5)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── 审计报告 ────────────────────────────────────────

    /// 生成 Session 审计报告
    pub fn audit_report(&self, session_id: &str) -> crate::Result<SessionAuditReport> {
        let session = self.get_session(session_id)?
            .ok_or_else(|| crate::Error::Tool(format!("Session not found: {}", session_id)))?;
        let turns = self.get_turns(session_id)?;
        let tool_calls = self.get_tool_calls(session_id)?;

        let successful_calls = tool_calls.iter().filter(|c| c.success).count();
        let failed_calls = tool_calls.iter().filter(|c| !c.success).count();
        let total_duration_ms: u64 = tool_calls.iter().map(|c| c.duration_ms).sum();

        Ok(SessionAuditReport {
            session,
            turns,
            tool_calls,
            successful_calls,
            failed_calls,
            total_duration_ms,
        })
    }
}

/// Session 审计报告
#[derive(Debug, Serialize)]
pub struct SessionAuditReport {
    pub session: SessionMeta,
    pub turns: Vec<Turn>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub successful_calls: usize,
    pub failed_calls: usize,
    pub total_duration_ms: u64,
}

// ── Session ID 生成 ──────────────────────────────────────

/// 生成 Session ID: session_{timestamp}_{8位随机}
pub fn generate_session_id() -> String {
    let now = Utc::now();
    let timestamp = now.format("%Y%m%d_%H%M%S");
    let random: String = uuid::Uuid::new_v4().to_string()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!("session_{}_{}", timestamp, random.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_id_format() {
        let id = generate_session_id();
        assert!(id.starts_with("session_"));
        // session_20240101_120000_abc123
        let parts: Vec<&str> = id.split('_').collect();
        assert!(parts.len() >= 4); // session, date, time, random
    }

    #[test]
    fn test_crud() {
        let store = SessionStore::in_memory().unwrap();
        let id = generate_session_id();
        
        let meta = SessionMeta {
            session_id: id.clone(),
            agent_name: "main".into(),
            model: "deepseek-chat".into(),
            working_dir: "F:\\测试".into(),
            status: SessionStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        };

        store.create_session(&meta).unwrap();
        let loaded = store.get_session(&id).unwrap().unwrap();
        assert_eq!(loaded.session_id, id);
        assert_eq!(loaded.agent_name, "main");
    }

    #[test]
    fn test_turns() {
        let store = SessionStore::in_memory().unwrap();
        let id = generate_session_id();
        
        let meta = SessionMeta {
            session_id: id.clone(),
            agent_name: "main".into(),
            model: "deepseek-chat".into(),
            working_dir: "F:\\测试".into(),
            status: SessionStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        };
        store.create_session(&meta).unwrap();

        let turn = Turn {
            turn_id: uuid::Uuid::new_v4().to_string(),
            session_id: id.clone(),
            turn_index: 0,
            role: "user".into(),
            content: "hello".into(),
            thinking: None,
            tool_calls: Vec::new(),
            created_at: Utc::now(),
            tokens_used: 10,
        };
        store.save_turn(&turn).unwrap();

        let turns = store.get_turns(&id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "hello");
    }
}
