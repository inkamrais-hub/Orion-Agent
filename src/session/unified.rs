//! 统一存储层 — 单一 SQLite 数据库 + spawn_blocking 异步封装
//!
//! 合并了原 SessionStore (SQLite) + AgentStore (SQLite) + SessionManager (JSONL)
//! 的所有功能，统一到一个数据库文件中。
//!
//! 所有 SQLite 操作通过 `tokio::task::spawn_blocking` 执行，
//! 不阻塞 tokio 异步运行时。

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use chrono::{DateTime, Utc};

// ── 类型重导出 ──────────────────────────────────────────────

pub use crate::session::store::{
    SessionMeta, SessionStatus, Turn, ToolCallRecord, Snapshot,
    generate_session_id,
};
pub use crate::agent::store::{AgentConfigModel, SessionSnapshot, RollbackAction};

// ── 本模块新增类型 ──────────────────────────────────────────

/// 对话消息条目 (替代 SessionManager 中的 JSONL 存储)
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

// ── 内部辅助函数 ────────────────────────────────────────────

fn status_to_str(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Paused => "paused",
        SessionStatus::Completed => "completed",
        SessionStatus::Failed => "failed",
        SessionStatus::Deleted => "deleted",
    }
}

fn status_from_str(s: &str) -> SessionStatus {
    match s {
        "active" => SessionStatus::Active,
        "paused" => SessionStatus::Paused,
        "completed" => SessionStatus::Completed,
        "failed" => SessionStatus::Failed,
        "deleted" => SessionStatus::Deleted,
        _ => SessionStatus::Active,
    }
}

/// 默认数据库路径
fn default_db_path() -> PathBuf {
    crate::config::data_dir_path().join("orion.db")
}

// ── 统一存储 ────────────────────────────────────────────────

/// 统一存储
///
/// 所有 SQLite 操作通过 `tokio::task::spawn_blocking` 在阻塞线程池上执行，
/// 不会阻塞 tokio 异步运行时。`Connection` 用 `Arc<Mutex<..>>` 包裹以便
/// 移入 spawn_blocking 闭包。
pub struct UnifiedStore {
    conn: Arc<std::sync::Mutex<Connection>>,
    db_path: PathBuf,
}

impl UnifiedStore {
    // ── 构造 ──────────────────────────────────────────────

    /// 在默认路径打开 (或创建) 统一数据库
    pub async fn open() -> crate::Result<Self> {
        let db_path = default_db_path();
        Self::open_at(db_path).await
    }

    /// 在指定路径打开 (或创建) 统一数据库
    pub async fn open_at(db_path: PathBuf) -> crate::Result<Self> {
        let path = db_path.clone();
        let conn = tokio::task::spawn_blocking(move || -> crate::Result<Connection> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let conn = Connection::open(&path)?;
            Self::init_tables_sync(&conn)?;
            Ok(conn)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))??;

        Ok(Self {
            conn: Arc::new(std::sync::Mutex::new(conn)),
            db_path,
        })
    }

    /// 在内存中创建 (用于测试)
    pub fn in_memory() -> crate::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_tables_sync(&conn)?;
        Ok(Self {
            conn: Arc::new(std::sync::Mutex::new(conn)),
            db_path: PathBuf::from(":memory:"),
        })
    }

    /// 同步初始化所有数据库表
    fn init_tables_sync(conn: &Connection) -> crate::Result<()> {
        conn.execute_batch(
            "
            -- ── 来自 SessionStore ──

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

            -- ── 来自 AgentStore ──

            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                model TEXT NOT NULL,
                system_prompt TEXT NOT NULL DEFAULT '',
                tools_json TEXT NOT NULL DEFAULT '[]',
                mcp_servers_json TEXT NOT NULL DEFAULT '[]',
                max_turns INTEGER NOT NULL DEFAULT 20,
                max_tool_calls INTEGER NOT NULL DEFAULT 30,
                token_budget INTEGER NOT NULL DEFAULT 128000,
                thinking INTEGER NOT NULL DEFAULT 0,
                reasoning_effort TEXT NOT NULL DEFAULT 'medium',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS session_snapshots (
                snapshot_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                turn_index INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                target_path TEXT NOT NULL,
                content_before TEXT,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_ss_session ON session_snapshots(session_id);
            CREATE INDEX IF NOT EXISTS idx_ss_session_turn ON session_snapshots(session_id, turn_index);

            -- ── 替代 SessionManager JSONL ──

            CREATE TABLE IF NOT EXISTS transcripts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                parent_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls TEXT,
                timestamp TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_transcripts_session ON transcripts(session_id);
            ",
        )?;
        Ok(())
    }

    /// 获取数据库文件路径
    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    // ════════════════════════════════════════════════════════
    //  Session CRUD
    // ════════════════════════════════════════════════════════

    /// 创建 Session
    pub async fn create_session(&self, meta: &SessionMeta) -> crate::Result<()> {
        let conn = self.conn.clone();
        let meta = meta.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| {
                tracing::warn!("SQLite mutex poisoned, recovering");
                p.into_inner()
            });
            conn.execute(
                "INSERT INTO sessions
                    (session_id, agent_name, model, working_dir, status,
                     created_at, updated_at, turn_count, tool_call_count, total_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    meta.session_id,
                    meta.agent_name,
                    meta.model,
                    meta.working_dir,
                    status_to_str(&meta.status),
                    meta.created_at.to_rfc3339(),
                    meta.updated_at.to_rfc3339(),
                    meta.turn_count,
                    meta.tool_call_count,
                    meta.total_tokens,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取 Session
    pub async fn get_session(&self, session_id: &str) -> crate::Result<Option<SessionMeta>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT session_id, agent_name, model, working_dir, status,
                        created_at, updated_at, turn_count, tool_call_count, total_tokens
                 FROM sessions WHERE session_id = ?1",
            )?;
            let mut rows = stmt.query_map(params![sid], |row| {
                Ok(SessionMeta {
                    session_id: row.get(0)?,
                    agent_name: row.get(1)?,
                    model: row.get(2)?,
                    working_dir: row.get(3)?,
                    status: status_from_str(&row.get::<_, String>(4)?),
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
                    updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
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
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 列出最近的 Session
    pub async fn list_sessions(&self, limit: u32) -> crate::Result<Vec<SessionMeta>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT session_id, agent_name, model, working_dir, status,
                        created_at, updated_at, turn_count, tool_call_count, total_tokens
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(SessionMeta {
                    session_id: row.get(0)?,
                    agent_name: row.get(1)?,
                    model: row.get(2)?,
                    working_dir: row.get(3)?,
                    status: status_from_str(&row.get::<_, String>(4)?),
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
                    updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
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
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 更新 Session 状态
    pub async fn update_session_status(
        &self,
        session_id: &str,
        status: SessionStatus,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE session_id = ?3",
                params![status_to_str(&status), Utc::now().to_rfc3339(), sid],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 更新 Session 统计
    pub async fn update_session_stats(
        &self,
        session_id: &str,
        turn_count: u32,
        tool_call_count: u32,
        total_tokens: u64,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "UPDATE sessions SET turn_count = ?1, tool_call_count = ?2,
                        total_tokens = ?3, updated_at = ?4
                 WHERE session_id = ?5",
                params![turn_count, tool_call_count, total_tokens, Utc::now().to_rfc3339(), sid],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 删除 Session (级联删除 turns, tool_calls, transcripts)
    pub async fn delete_session(&self, session_id: &str) -> crate::Result<bool> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            // 级联删除关联数据
            conn.execute("DELETE FROM turns WHERE session_id = ?1", params![sid])?;
            conn.execute("DELETE FROM tool_calls WHERE session_id = ?1", params![sid])?;
            conn.execute("DELETE FROM transcripts WHERE session_id = ?1", params![sid])?;
            conn.execute("DELETE FROM snapshots WHERE session_id = ?1", params![sid])?;
            conn.execute("DELETE FROM session_snapshots WHERE session_id = ?1", params![sid])?;
            let rows = conn.execute("DELETE FROM sessions WHERE session_id = ?1", params![sid])?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Turn CRUD
    // ════════════════════════════════════════════════════════

    /// 保存 Turn
    pub async fn save_turn(&self, turn: &Turn) -> crate::Result<()> {
        let conn = self.conn.clone();
        let turn = turn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO turns
                    (turn_id, session_id, turn_index, role, content, thinking, created_at, tokens_used)
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
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取 Session 的所有 Turn (优化: 一次查询 turns + 一次查询 tool_calls, 避免 N+1)
    pub async fn get_turns(&self, session_id: &str) -> crate::Result<Vec<Turn>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());

            // 1. 获取所有 turns
            let mut stmt = conn.prepare(
                "SELECT turn_id, turn_index, role, content, thinking, created_at, tokens_used
                 FROM turns WHERE session_id = ?1 ORDER BY turn_index",
            )?;
            let turn_rows = stmt.query_map(params![sid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, u32>(6)?,
                ))
            })?;
            let mut turn_data = Vec::new();
            for row in turn_rows {
                turn_data.push(row?);
            }

            // 2. 一次性获取所有 tool_calls，按 turn_id 分组
            let mut tc_stmt = conn.prepare(
                "SELECT call_id, turn_id, tool_name, input_summary, output_summary, success, duration_ms
                 FROM tool_calls WHERE session_id = ?1 ORDER BY call_id",
            )?;
            let tc_rows = tc_stmt.query_map(params![sid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i32>(5)?,
                    row.get::<_, u64>(6)?,
                ))
            })?;

            let mut tc_map: std::collections::HashMap<String, Vec<ToolCallRecord>> =
                std::collections::HashMap::new();
            for row in tc_rows {
                let (call_id, turn_id, tool_name, input_summary, output_summary, success, duration_ms) =
                    row?;
                tc_map.entry(turn_id).or_default().push(ToolCallRecord {
                    call_id,
                    tool_name,
                    input_summary,
                    output_summary,
                    success: success != 0,
                    duration_ms,
                });
            }

            // 3. 组装 Turn
            let turns = turn_data
                .into_iter()
                .map(
                    |(turn_id, turn_index, role, content, thinking, created_at, tokens_used)| Turn {
                        turn_id: turn_id.clone(),
                        session_id: sid.clone(),
                        turn_index,
                        role,
                        content,
                        thinking,
                        tool_calls: tc_map.remove(&turn_id).unwrap_or_default(),
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .unwrap_or_default()
                            .with_timezone(&Utc),
                        tokens_used,
                    },
                )
                .collect();

            Ok(turns)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  ToolCall CRUD
    // ════════════════════════════════════════════════════════

    /// 保存工具调用
    pub async fn save_tool_call(
        &self,
        call: &ToolCallRecord,
        turn_id: &str,
        session_id: &str,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        let call = call.clone();
        let tid = turn_id.to_string();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO tool_calls
                    (call_id, turn_id, session_id, tool_name, input_summary, output_summary, success, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    call.call_id,
                    tid,
                    sid,
                    call.tool_name,
                    call.input_summary,
                    call.output_summary,
                    call.success as i32,
                    call.duration_ms,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取 Session 的所有工具调用
    pub async fn get_tool_calls(&self, session_id: &str) -> crate::Result<Vec<ToolCallRecord>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT call_id, tool_name, input_summary, output_summary, success, duration_ms
                 FROM tool_calls WHERE session_id = ?1 ORDER BY call_id",
            )?;
            let rows = stmt.query_map(params![sid], |row| {
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
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Session Store 级快照 (预留接口)
    // ════════════════════════════════════════════════════════

    /// 保存快照 (预留)
    pub async fn save_snapshot(&self, snapshot: &Snapshot) -> crate::Result<()> {
        let conn = self.conn.clone();
        let snapshot = snapshot.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO snapshots
                    (snapshot_id, session_id, snapshot_type, created_at, file_hashes, metadata)
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
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取 Session 的快照 (预留)
    pub async fn get_snapshots(&self, session_id: &str) -> crate::Result<Vec<Snapshot>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT snapshot_id, session_id, snapshot_type, created_at, file_hashes, metadata
                 FROM snapshots WHERE session_id = ?1 ORDER BY created_at",
            )?;
            let rows = stmt.query_map(params![sid], |row| {
                Ok(Snapshot {
                    snapshot_id: row.get(0)?,
                    session_id: row.get(1)?,
                    snapshot_type: row.get(2)?,
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(3)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
                    file_hashes: serde_json::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or_default(),
                    metadata: row.get(5)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Transcript (替代 SessionManager JSONL)
    // ════════════════════════════════════════════════════════

    /// 追加对话消息
    pub async fn append_transcript(
        &self,
        session_id: &str,
        entry: &TranscriptEntry,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        let entry = entry.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let tool_calls_json = entry
                .tool_calls
                .as_ref()
                .map(|tc| serde_json::to_string(tc).unwrap_or_default());
            conn.execute(
                "INSERT INTO transcripts
                    (id, session_id, parent_id, role, content, tool_calls, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.id,
                    sid,
                    entry.parent_id,
                    entry.role,
                    entry.content,
                    tool_calls_json,
                    entry.timestamp.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取 Session 的完整对话记录
    pub async fn get_transcript(&self, session_id: &str) -> crate::Result<Vec<TranscriptEntry>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT id, parent_id, role, content, tool_calls, timestamp
                 FROM transcripts WHERE session_id = ?1 ORDER BY timestamp",
            )?;
            let rows = stmt.query_map(params![sid], |row| {
                let tool_calls_str: Option<String> = row.get(4)?;
                let tool_calls = tool_calls_str
                    .and_then(|s| serde_json::from_str(&s).ok());
                Ok(TranscriptEntry {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    tool_calls,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(5)?)
                        .unwrap_or_default()
                        .with_timezone(&Utc),
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Agent Config CRUD
    // ════════════════════════════════════════════════════════

    /// 新增 Agent 配置
    pub async fn create_agent(&self, config: &AgentConfigModel) -> crate::Result<()> {
        let conn = self.conn.clone();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO agents
                    (id, name, model, system_prompt, tools_json, mcp_servers_json,
                     max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    config.id,
                    config.name,
                    config.model,
                    config.system_prompt,
                    config.tools_json,
                    config.mcp_servers_json,
                    config.max_turns,
                    config.max_tool_calls,
                    config.token_budget,
                    config.thinking as i32,
                    config.reasoning_effort,
                    config.created_at,
                    config.updated_at,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 按 ID 获取 Agent 配置
    pub async fn get_agent(&self, id: &str) -> crate::Result<Option<AgentConfigModel>> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                        max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                        created_at, updated_at
                 FROM agents WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![id], Self::row_to_agent)?;
            match rows.next() {
                Some(Ok(m)) => Ok(Some(m)),
                Some(Err(e)) => Err(e.into()),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 列出全部 Agent 配置
    pub async fn list_agents(&self) -> crate::Result<Vec<AgentConfigModel>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                        max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                        created_at, updated_at
                 FROM agents ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], Self::row_to_agent)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 更新指定 ID 的 Agent 配置
    pub async fn update_agent(
        &self,
        id: &str,
        config: &AgentConfigModel,
    ) -> crate::Result<bool> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let now = Utc::now().to_rfc3339();
            let rows = conn.execute(
                "UPDATE agents SET name=?1, model=?2, system_prompt=?3, tools_json=?4,
                        mcp_servers_json=?5, max_turns=?6, max_tool_calls=?7, token_budget=?8,
                        thinking=?9, reasoning_effort=?10, updated_at=?11
                 WHERE id=?12",
                params![
                    config.name,
                    config.model,
                    config.system_prompt,
                    config.tools_json,
                    config.mcp_servers_json,
                    config.max_turns,
                    config.max_tool_calls,
                    config.token_budget,
                    config.thinking as i32,
                    config.reasoning_effort,
                    now,
                    id,
                ],
            )?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 删除指定 ID 的 Agent 配置
    pub async fn delete_agent(&self, id: &str) -> crate::Result<bool> {
        let conn = self.conn.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let rows = conn.execute("DELETE FROM agents WHERE id = ?1", params![id])?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Session Snapshot (AgentStore 回滚快照)
    // ════════════════════════════════════════════════════════

    /// 保存回滚快照 (工具执行前的文件备份)
    pub async fn save_session_snapshot(&self, snapshot: &SessionSnapshot) -> crate::Result<()> {
        let conn = self.conn.clone();
        let snapshot = snapshot.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO session_snapshots
                    (snapshot_id, session_id, agent_id, turn_index, tool_name,
                     target_path, content_before, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    snapshot.snapshot_id,
                    snapshot.session_id,
                    snapshot.agent_id,
                    snapshot.turn_index,
                    snapshot.tool_name,
                    snapshot.target_path,
                    snapshot.content_before,
                    snapshot.created_at,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 获取指定 session 的所有回滚快照
    pub async fn get_session_snapshots(
        &self,
        session_id: &str,
    ) -> crate::Result<Vec<SessionSnapshot>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT snapshot_id, session_id, agent_id, turn_index, tool_name,
                        target_path, content_before, created_at
                 FROM session_snapshots
                 WHERE session_id = ?1
                 ORDER BY turn_index DESC, created_at DESC",
            )?;
            let rows = stmt.query_map(params![sid], Self::row_to_snapshot)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 回滚到指定轮次: 返回需要恢复的文件列表
    ///
    /// 筛选 `turn_index >= target_turn` 的快照，按 turn_index DESC, created_at DESC 排序，
    /// 对每个 target_path 只保留最新的快照 (后改的先恢复)。
    pub async fn rollback_to_turn(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        let ti = turn_index as i64;
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT target_path, content_before
                 FROM session_snapshots
                 WHERE session_id = ?1 AND turn_index >= ?2
                 ORDER BY turn_index DESC, created_at DESC",
            )?;
            let rows = stmt.query_map(params![sid, ti], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?;

            let mut seen = std::collections::HashSet::new();
            let mut actions = Vec::new();
            for row in rows {
                let (target_path, content_before) = row?;
                if seen.insert(target_path.clone()) {
                    actions.push(RollbackAction {
                        target_path,
                        content_before,
                    });
                }
            }
            Ok(actions)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    /// 清理指定轮次之后的回滚快照
    pub async fn cleanup_session_snapshots_after(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        let sid = session_id.to_string();
        let ti = turn_index as i64;
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "DELETE FROM session_snapshots
                 WHERE session_id = ?1 AND turn_index >= ?2",
                params![sid, ti],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking join error: {}", e)))?
    }

    // ════════════════════════════════════════════════════════
    //  Audit Report
    // ════════════════════════════════════════════════════════

    /// 生成 Session 审计报告
    pub async fn audit_report(&self, session_id: &str) -> crate::Result<SessionAuditReport> {
        // 依次复用已有异步方法
        let session = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| crate::Error::Tool(format!("Session not found: {}", session_id)))?;
        let turns = self.get_turns(session_id).await?;
        let tool_calls = self.get_tool_calls(session_id).await?;

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

    // ════════════════════════════════════════════════════════
    //  内部行映射辅助
    // ════════════════════════════════════════════════════════

    fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentConfigModel> {
        Ok(AgentConfigModel {
            id: row.get(0)?,
            name: row.get(1)?,
            model: row.get(2)?,
            system_prompt: row.get(3)?,
            tools_json: row.get(4)?,
            mcp_servers_json: row.get(5)?,
            max_turns: row.get(6)?,
            max_tool_calls: row.get(7)?,
            token_budget: row.get(8)?,
            thinking: row.get::<_, i32>(9)? != 0,
            reasoning_effort: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    }

    fn row_to_snapshot(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSnapshot> {
        Ok(SessionSnapshot {
            snapshot_id: row.get(0)?,
            session_id: row.get(1)?,
            agent_id: row.get(2)?,
            turn_index: row.get(3)?,
            tool_name: row.get(4)?,
            target_path: row.get(5)?,
            content_before: row.get(6)?,
            created_at: row.get(7)?,
        })
    }
}

// ════════════════════════════════════════════════════════════
//  Tests
// ════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_meta(session_id: &str) -> SessionMeta {
        SessionMeta {
            session_id: session_id.to_string(),
            agent_name: "main".into(),
            model: "deepseek-chat".into(),
            working_dir: "/tmp/test".into(),
            status: SessionStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        }
    }

    fn make_agent(id: &str) -> AgentConfigModel {
        let now = Utc::now().to_rfc3339();
        AgentConfigModel {
            id: id.to_string(),
            name: "test-agent".into(),
            model: "deepseek-chat".into(),
            system_prompt: "You are a test agent".into(),
            tools_json: "[]".into(),
            mcp_servers_json: "[]".into(),
            max_turns: 20,
            max_tool_calls: 30,
            token_budget: 128000,
            thinking: false,
            reasoning_effort: "medium".into(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    // ── Session CRUD ──

    #[tokio::test]
    async fn test_create_and_get_session() {
        let store = UnifiedStore::in_memory().unwrap();
        let meta = make_meta("sess_001");
        store.create_session(&meta).await.unwrap();

        let loaded = store.get_session("sess_001").await.unwrap().unwrap();
        assert_eq!(loaded.session_id, "sess_001");
        assert_eq!(loaded.agent_name, "main");
        assert_eq!(loaded.model, "deepseek-chat");
    }

    #[tokio::test]
    async fn test_list_sessions() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_a"))
            .await
            .unwrap();
        store
            .create_session(&make_meta("sess_b"))
            .await
            .unwrap();

        let list = store.list_sessions(10).await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_update_session_status() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_002"))
            .await
            .unwrap();
        store
            .update_session_status("sess_002", SessionStatus::Completed)
            .await
            .unwrap();

        let loaded = store.get_session("sess_002").await.unwrap().unwrap();
        assert_eq!(loaded.status, SessionStatus::Completed);
    }

    #[tokio::test]
    async fn test_update_session_stats() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_stats"))
            .await
            .unwrap();
        store
            .update_session_stats("sess_stats", 5, 10, 1500)
            .await
            .unwrap();

        let loaded = store.get_session("sess_stats").await.unwrap().unwrap();
        assert_eq!(loaded.turn_count, 5);
        assert_eq!(loaded.tool_call_count, 10);
        assert_eq!(loaded.total_tokens, 1500);
    }

    #[tokio::test]
    async fn test_delete_session() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_del"))
            .await
            .unwrap();

        let deleted = store.delete_session("sess_del").await.unwrap();
        assert!(deleted);

        let loaded = store.get_session("sess_del").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_session() {
        let store = UnifiedStore::in_memory().unwrap();
        let deleted = store.delete_session("nonexistent").await.unwrap();
        assert!(!deleted);
    }

    // ── Turn CRUD ──

    #[tokio::test]
    async fn test_save_and_get_turns() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_turns"))
            .await
            .unwrap();

        let turn = Turn {
            turn_id: "turn_001".into(),
            session_id: "sess_turns".into(),
            turn_index: 0,
            role: "user".into(),
            content: "hello".into(),
            thinking: None,
            tool_calls: vec![],
            created_at: Utc::now(),
            tokens_used: 10,
        };
        store.save_turn(&turn).await.unwrap();

        let turns = store.get_turns("sess_turns").await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].content, "hello");
        assert_eq!(turns[0].turn_index, 0);
    }

    #[tokio::test]
    async fn test_turns_with_tool_calls() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_tc"))
            .await
            .unwrap();

        let turn = Turn {
            turn_id: "turn_010".into(),
            session_id: "sess_tc".into(),
            turn_index: 0,
            role: "assistant".into(),
            content: "calling tool".into(),
            thinking: Some("thinking...".into()),
            tool_calls: vec![],
            created_at: Utc::now(),
            tokens_used: 50,
        };
        store.save_turn(&turn).await.unwrap();

        let call = ToolCallRecord {
            call_id: "call_001".into(),
            tool_name: "bash".into(),
            input_summary: "ls -la".into(),
            output_summary: "file1 file2".into(),
            success: true,
            duration_ms: 100,
        };
        store
            .save_tool_call(&call, "turn_010", "sess_tc")
            .await
            .unwrap();

        let turns = store.get_turns("sess_tc").await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tool_calls.len(), 1);
        assert_eq!(turns[0].tool_calls[0].tool_name, "bash");
        assert!(turns[0].tool_calls[0].success);
    }

    // ── ToolCall ──

    #[tokio::test]
    async fn test_get_tool_calls() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_tc2"))
            .await
            .unwrap();

        let turn = Turn {
            turn_id: "turn_020".into(),
            session_id: "sess_tc2".into(),
            turn_index: 0,
            role: "assistant".into(),
            content: "".into(),
            thinking: None,
            tool_calls: vec![],
            created_at: Utc::now(),
            tokens_used: 0,
        };
        store.save_turn(&turn).await.unwrap();

        for i in 0..3 {
            let call = ToolCallRecord {
                call_id: format!("call_{}", i),
                tool_name: format!("tool_{}", i),
                input_summary: "input".into(),
                output_summary: "output".into(),
                success: i != 1,
                duration_ms: 50,
            };
            store
                .save_tool_call(&call, "turn_020", "sess_tc2")
                .await
                .unwrap();
        }

        let calls = store.get_tool_calls("sess_tc2").await.unwrap();
        assert_eq!(calls.len(), 3);
    }

    // ── Transcript ──

    #[tokio::test]
    async fn test_append_and_get_transcript() {
        let store = UnifiedStore::in_memory().unwrap();

        let entry = TranscriptEntry {
            id: "msg_001".into(),
            parent_id: None,
            role: "user".into(),
            content: "Hello, agent!".into(),
            tool_calls: None,
            timestamp: Utc::now(),
        };
        store
            .append_transcript("sess_tr", &entry)
            .await
            .unwrap();

        let entry2 = TranscriptEntry {
            id: "msg_002".into(),
            parent_id: Some("msg_001".into()),
            role: "assistant".into(),
            content: "Hello, user!".into(),
            tool_calls: None,
            timestamp: Utc::now(),
        };
        store
            .append_transcript("sess_tr", &entry2)
            .await
            .unwrap();

        let transcript = store.get_transcript("sess_tr").await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].role, "user");
        assert_eq!(transcript[1].role, "assistant");
        assert_eq!(transcript[1].parent_id.as_deref(), Some("msg_001"));
    }

    #[tokio::test]
    async fn test_transcript_with_tool_calls() {
        let store = UnifiedStore::in_memory().unwrap();

        let tool_call_value = serde_json::json!({
            "id": "call_abc",
            "function": {"name": "bash", "arguments": "{}"}
        });

        let entry = TranscriptEntry {
            id: "msg_010".into(),
            parent_id: None,
            role: "assistant".into(),
            content: "".into(),
            tool_calls: Some(vec![tool_call_value.clone()]),
            timestamp: Utc::now(),
        };
        store
            .append_transcript("sess_tr2", &entry)
            .await
            .unwrap();

        let transcript = store.get_transcript("sess_tr2").await.unwrap();
        assert_eq!(transcript.len(), 1);
        let tc = transcript[0].tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0], tool_call_value);
    }

    // ── Agent CRUD ──

    #[tokio::test]
    async fn test_create_and_get_agent() {
        let store = UnifiedStore::in_memory().unwrap();
        let agent = make_agent("agent_001");
        store.create_agent(&agent).await.unwrap();

        let loaded = store.get_agent("agent_001").await.unwrap().unwrap();
        assert_eq!(loaded.name, "test-agent");
        assert_eq!(loaded.model, "deepseek-chat");
    }

    #[tokio::test]
    async fn test_list_agents() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_agent(&make_agent("agent_a"))
            .await
            .unwrap();
        store
            .create_agent(&make_agent("agent_b"))
            .await
            .unwrap();

        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_update_agent() {
        let store = UnifiedStore::in_memory().unwrap();
        let mut agent = make_agent("agent_upd");
        store.create_agent(&agent).await.unwrap();

        agent.name = "updated-agent".into();
        agent.model = "gpt-4".into();
        let updated = store.update_agent("agent_upd", &agent).await.unwrap();
        assert!(updated);

        let loaded = store.get_agent("agent_upd").await.unwrap().unwrap();
        assert_eq!(loaded.name, "updated-agent");
        assert_eq!(loaded.model, "gpt-4");
    }

    #[tokio::test]
    async fn test_delete_agent() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_agent(&make_agent("agent_del"))
            .await
            .unwrap();

        let deleted = store.delete_agent("agent_del").await.unwrap();
        assert!(deleted);

        let loaded = store.get_agent("agent_del").await.unwrap();
        assert!(loaded.is_none());
    }

    // ── Session Snapshots (AgentStore rollback) ──

    #[tokio::test]
    async fn test_save_and_get_session_snapshots() {
        let store = UnifiedStore::in_memory().unwrap();
        let now = Utc::now().to_rfc3339();

        let snap = SessionSnapshot {
            snapshot_id: "snap_001".into(),
            session_id: "sess_snap".into(),
            agent_id: "agent_001".into(),
            turn_index: 0,
            tool_name: "edit".into(),
            target_path: "/tmp/test.rs".into(),
            content_before: Some("fn main() {}".into()),
            created_at: now,
        };
        store.save_session_snapshot(&snap).await.unwrap();

        let snaps = store.get_session_snapshots("sess_snap").await.unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].target_path, "/tmp/test.rs");
        assert_eq!(
            snaps[0].content_before.as_deref(),
            Some("fn main() {}")
        );
    }

    #[tokio::test]
    async fn test_rollback_to_turn() {
        let store = UnifiedStore::in_memory().unwrap();
        let now = Utc::now().to_rfc3339();

        for i in 0..3 {
            let snap = SessionSnapshot {
                snapshot_id: format!("snap_rb_{}", i),
                session_id: "sess_rb".into(),
                agent_id: "agent_001".into(),
                turn_index: i,
                tool_name: "edit".into(),
                target_path: format!("/tmp/file_{}.rs", i),
                content_before: Some(format!("content_{}", i)),
                created_at: now.clone(),
            };
            store.save_session_snapshot(&snap).await.unwrap();
        }

        let actions = store.rollback_to_turn("sess_rb", 1).await.unwrap();
        // turn_index >= 1: file_2.rs (turn 2) then file_1.rs (turn 1)
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].target_path, "/tmp/file_2.rs");
        assert_eq!(actions[1].target_path, "/tmp/file_1.rs");
    }

    #[tokio::test]
    async fn test_cleanup_session_snapshots_after() {
        let store = UnifiedStore::in_memory().unwrap();
        let now = Utc::now().to_rfc3339();

        for i in 0..5 {
            let snap = SessionSnapshot {
                snapshot_id: format!("snap_cl_{}", i),
                session_id: "sess_cl".into(),
                agent_id: "agent_001".into(),
                turn_index: i,
                tool_name: "edit".into(),
                target_path: format!("/tmp/f_{}.rs", i),
                content_before: None,
                created_at: now.clone(),
            };
            store.save_session_snapshot(&snap).await.unwrap();
        }

        store
            .cleanup_session_snapshots_after("sess_cl", 3)
            .await
            .unwrap();

        let snaps = store.get_session_snapshots("sess_cl").await.unwrap();
        assert_eq!(snaps.len(), 3); // turns 0, 1, 2 remain
    }

    // ── Audit Report ──

    #[tokio::test]
    async fn test_audit_report() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_audit"))
            .await
            .unwrap();

        let turn = Turn {
            turn_id: "turn_audit".into(),
            session_id: "sess_audit".into(),
            turn_index: 0,
            role: "assistant".into(),
            content: "audit test".into(),
            thinking: None,
            tool_calls: vec![],
            created_at: Utc::now(),
            tokens_used: 100,
        };
        store.save_turn(&turn).await.unwrap();

        let call_ok = ToolCallRecord {
            call_id: "call_ok".into(),
            tool_name: "read".into(),
            input_summary: "file.rs".into(),
            output_summary: "content".into(),
            success: true,
            duration_ms: 10,
        };
        let call_fail = ToolCallRecord {
            call_id: "call_fail".into(),
            tool_name: "bash".into(),
            input_summary: "rm -rf /".into(),
            output_summary: "denied".into(),
            success: false,
            duration_ms: 5,
        };
        store
            .save_tool_call(&call_ok, "turn_audit", "sess_audit")
            .await
            .unwrap();
        store
            .save_tool_call(&call_fail, "turn_audit", "sess_audit")
            .await
            .unwrap();

        let report = store.audit_report("sess_audit").await.unwrap();
        assert_eq!(report.session.session_id, "sess_audit");
        assert_eq!(report.turns.len(), 1);
        assert_eq!(report.tool_calls.len(), 2);
        assert_eq!(report.successful_calls, 1);
        assert_eq!(report.failed_calls, 1);
        assert_eq!(report.total_duration_ms, 15);
    }

    #[tokio::test]
    async fn test_audit_report_session_not_found() {
        let store = UnifiedStore::in_memory().unwrap();
        let result = store.audit_report("nonexistent").await;
        assert!(result.is_err());
    }

    // ── Store-level Snapshot (from SessionStore) ──

    #[tokio::test]
    async fn test_save_and_get_snapshot() {
        let store = UnifiedStore::in_memory().unwrap();
        store
            .create_session(&make_meta("sess_ss"))
            .await
            .unwrap();

        let mut file_hashes = std::collections::HashMap::new();
        file_hashes.insert("file.rs".into(), "abc123".into());

        let snapshot = Snapshot {
            snapshot_id: "ss_001".into(),
            session_id: "sess_ss".into(),
            snapshot_type: "pre_tool".into(),
            created_at: Utc::now(),
            file_hashes,
            metadata: "{\"key\":\"value\"}".into(),
        };
        store.save_snapshot(&snapshot).await.unwrap();

        let snaps = store.get_snapshots("sess_ss").await.unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].snapshot_type, "pre_tool");
        assert_eq!(
            snaps[0].file_hashes.get("file.rs").map(|s| s.as_str()),
            Some("abc123")
        );
    }

    // ── generate_session_id re-export ──

    #[test]
    fn test_generate_session_id_reexport() {
        let id = generate_session_id();
        assert!(id.starts_with("session_"));
    }
}
