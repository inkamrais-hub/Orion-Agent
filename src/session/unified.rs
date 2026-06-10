//! Unified storage backend — single SQLite database
//!
//! Consolidates three previously separate storage concerns into one:
//! - Agent configs and file snapshots (formerly `AgentStore`)
//! - Sessions, turns, and tool calls (formerly `SessionStore`)
//! - Session transcripts (formerly JSONL-based `SessionManager`)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::session::store::{SessionAuditReport, ToolCallRecord, Turn};

// ============================================================
//  Types
// ============================================================

/// Unified session status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Failed,
    Deleted,
}

impl SessionStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Deleted => "deleted",
        }
    }

    // Intentionally returns Self with a default (Active) rather than Result; not the same as FromStr
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "deleted" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

/// Unified session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub agent_name: String,
    pub model: String,
    pub working_dir: String,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
    pub turn_count: u64,
    pub tool_call_count: u64,
    pub total_tokens: u64,
}

/// Transcript entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<serde_json::Value>,
    pub timestamp: String,
}

/// Agent configuration model (persisted to the `agents` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigModel {
    pub id: String,
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    /// JSON array, e.g. `["read", "bash", "edit"]`
    pub tools_json: String,
    /// JSON array, e.g. `[{"name":"fs","command":"npx","args":["-y","@anthropic/mcp-fs"]}]`
    pub mcp_servers_json: String,
    pub max_turns: i64,
    pub max_tool_calls: i64,
    pub token_budget: i64,
    pub thinking: bool,
    pub reasoning_effort: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Session rollback snapshot (file backup taken before a tool executes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub turn_index: i64,
    pub tool_name: String,
    pub target_path: String,
    /// `None` = file did not exist before (newly created in this session).
    pub content_before: Option<String>,
    pub created_at: String,
}

/// Rollback action: describes how to restore a single file.
#[derive(Debug, Clone)]
pub struct RollbackAction {
    pub target_path: String,
    /// `Some` = restore file content, `None` = delete file (created this session).
    pub content_before: Option<String>,
}

// ============================================================
//  UnifiedStore
// ============================================================

/// Unified storage backed by a single SQLite database.
///
/// Consolidates agent configs, file snapshots, sessions, turns,
/// tool calls, and transcripts into one `orion.db` file.
pub struct UnifiedStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl UnifiedStore {
    /// Open the unified database at the default path
    /// (`data_dir_path()/orion.db`). Creates tables if needed.
    pub async fn open() -> crate::Result<Arc<Self>> {
        let path = crate::config::data_dir_path().join("orion.db");
        let store = Self::open_path(&path).await?;
        Ok(Arc::new(store))
    }

    /// Open with a custom path (for testing, pass `:memory:`).
    pub async fn open_path(path: &Path) -> crate::Result<Self> {
        let is_memory = path.to_str() == Some(":memory:");

        if !is_memory {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = if is_memory {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        Self::init_tables(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
        })
    }

    /// Create all tables and indexes.
    fn init_tables(conn: &Connection) -> crate::Result<()> {
        conn.execute_batch(
            "-- From AgentStore
             CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                model TEXT NOT NULL DEFAULT 'deepseek-chat',
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
                agent_id TEXT NOT NULL DEFAULT '',
                turn_index INTEGER NOT NULL,
                tool_name TEXT NOT NULL DEFAULT '',
                target_path TEXT NOT NULL,
                content_before TEXT,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_snapshots_session ON session_snapshots(session_id);
            CREATE INDEX IF NOT EXISTS idx_snapshots_session_turn ON session_snapshots(session_id, turn_index);

            -- From SessionStore
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                agent_name TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                working_dir TEXT NOT NULL DEFAULT '',
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
                content TEXT NOT NULL DEFAULT '',
                thinking TEXT,
                created_at TEXT NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );
            CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_id);

            CREATE TABLE IF NOT EXISTS tool_calls (
                call_id TEXT PRIMARY KEY,
                turn_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                input_summary TEXT NOT NULL DEFAULT '',
                output_summary TEXT NOT NULL DEFAULT '',
                success INTEGER NOT NULL DEFAULT 1,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (turn_id) REFERENCES turns(turn_id),
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);

            -- New: replaces JSONL transcripts
            CREATE TABLE IF NOT EXISTS transcripts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                parent_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                tool_calls_json TEXT,
                timestamp TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(session_id)
            );
            CREATE INDEX IF NOT EXISTS idx_transcripts_session ON transcripts(session_id);",
        )?;
        Ok(())
    }

    // ── Agent Config Operations ──────────────────────────

    /// List all agent configurations.
    pub async fn list_agents(&self) -> crate::Result<Vec<AgentConfigModel>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                    max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                    created_at, updated_at
             FROM agents ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_agent_model)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get a single agent configuration by ID.
    pub async fn get_agent(&self, id: &str) -> crate::Result<Option<AgentConfigModel>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                    max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                    created_at, updated_at
             FROM agents WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], Self::row_to_agent_model)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Create a new agent configuration.
    pub async fn create_agent(&self, config: &AgentConfigModel) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO agents (id, name, model, system_prompt, tools_json, mcp_servers_json,
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
    }

    /// Update an existing agent configuration. Returns `true` if a row was updated.
    pub async fn update_agent(
        &self,
        id: &str,
        config: &AgentConfigModel,
    ) -> crate::Result<bool> {
        let conn = self.conn.lock().await;
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
    }

    /// Delete an agent configuration. Returns `true` if a row was deleted.
    pub async fn delete_agent(&self, id: &str) -> crate::Result<bool> {
        let conn = self.conn.lock().await;
        let rows = conn.execute("DELETE FROM agents WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }

    // ── File Snapshot Operations ─────────────────────────

    /// Save a file snapshot (pre-tool backup).
    pub async fn save_snapshot(&self, snapshot: &SessionSnapshot) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO session_snapshots
                (snapshot_id, session_id, agent_id, turn_index, tool_name, target_path, content_before, created_at)
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
    }

    /// Get all snapshots for a session (ordered by turn_index DESC, created_at DESC).
    pub async fn get_snapshots(
        &self,
        session_id: &str,
    ) -> crate::Result<Vec<SessionSnapshot>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT snapshot_id, session_id, agent_id, turn_index, tool_name,
                    target_path, content_before, created_at
             FROM session_snapshots
             WHERE session_id = ?1
             ORDER BY turn_index DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], Self::row_to_snapshot)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Rollback to a given turn: returns the file-restore actions needed.
    ///
    /// Selects snapshots with `turn_index >= target_turn`, then for each
    /// `target_path` keeps only the most recent snapshot (latest change is
    /// restored first).
    pub async fn rollback_to_turn(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>> {
        let snapshots = self.get_snapshots(session_id).await?;

        let relevant: Vec<_> = snapshots
            .into_iter()
            .filter(|s| s.turn_index >= turn_index as i64)
            .collect();

        let mut seen = std::collections::HashSet::new();
        let mut actions = Vec::new();
        for snap in relevant {
            if seen.insert(snap.target_path.clone()) {
                actions.push(RollbackAction {
                    target_path: snap.target_path,
                    content_before: snap.content_before,
                });
            }
        }
        Ok(actions)
    }

    /// Delete all snapshots at or after `turn_index`.
    pub async fn cleanup_snapshots_after(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM session_snapshots
             WHERE session_id = ?1 AND turn_index >= ?2",
            params![session_id, turn_index],
        )?;
        Ok(())
    }

    // ── Session Operations ───────────────────────────────

    /// Create a new session.
    pub async fn create_session(&self, meta: &SessionMeta) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO sessions (session_id, agent_name, model, working_dir, status,
                                   created_at, updated_at, turn_count, tool_call_count, total_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                meta.session_id,
                meta.agent_name,
                meta.model,
                meta.working_dir,
                meta.status.as_str(),
                meta.created_at,
                meta.updated_at,
                meta.turn_count,
                meta.tool_call_count,
                meta.total_tokens,
            ],
        )?;
        Ok(())
    }

    /// Get a single session by ID.
    pub async fn get_session(&self, session_id: &str) -> crate::Result<Option<SessionMeta>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status,
                    created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![session_id], Self::row_to_unified_meta)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// List the most recent sessions.
    pub async fn list_sessions(&self, limit: u32) -> crate::Result<Vec<SessionMeta>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status,
                    created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], Self::row_to_unified_meta)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Update session status.
    pub async fn update_session_status(
        &self,
        session_id: &str,
        status: SessionStatus,
    ) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE session_id = ?3",
            params![status.as_str(), Utc::now().to_rfc3339(), session_id],
        )?;
        Ok(())
    }

    /// Update session statistics.
    pub async fn update_session_stats(
        &self,
        session_id: &str,
        turn_count: u64,
        tool_call_count: u64,
        total_tokens: u64,
    ) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE sessions SET turn_count = ?1, tool_call_count = ?2,
                    total_tokens = ?3, updated_at = ?4
             WHERE session_id = ?5",
            params![
                turn_count,
                tool_call_count,
                total_tokens,
                Utc::now().to_rfc3339(),
                session_id,
            ],
        )?;
        Ok(())
    }

    /// Delete a session and all related turns, tool calls, transcripts, and snapshots.
    pub async fn delete_session(&self, session_id: &str) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM tool_calls WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.execute(
            "DELETE FROM turns WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.execute(
            "DELETE FROM transcripts WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.execute(
            "DELETE FROM session_snapshots WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Search sessions by matching against session metadata and transcript content.
    /// Returns `(SessionMeta, snippet)` pairs.
    pub async fn search_sessions(
        &self,
        query: &str,
    ) -> crate::Result<Vec<(SessionMeta, String)>> {
        let conn = self.conn.lock().await;
        let pattern = format!("%{}%", query);
        let mut results = Vec::new();

        // Search in session metadata fields
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status,
                    created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions
             WHERE agent_name LIKE ?1 OR model LIKE ?1 OR working_dir LIKE ?1
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(params![pattern], Self::row_to_unified_meta)?;
        for row in rows {
            let meta = row?;
            let snippet = format!(
                "Match in session: agent={}, model={}",
                meta.agent_name, meta.model
            );
            results.push((meta, snippet));
        }

        // Search in transcript content
        let mut stmt2 = conn.prepare(
            "SELECT DISTINCT t.session_id, t.content
             FROM transcripts t
             WHERE t.content LIKE ?1",
        )?;
        let transcript_rows =
            stmt2.query_map(params![pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                ))
            })?;
        for row in transcript_rows {
            let (sid, content) = row?;
            if !results.iter().any(|(m, _)| m.session_id == sid) {
                if let Some(meta) = Self::get_session_sync(&conn, &sid)? {
                    let snippet = if content.len() > 100 {
                        format!("{}...", &content[..100])
                    } else {
                        content
                    };
                    results.push((meta, snippet));
                }
            }
        }

        Ok(results)
    }

    // ── Transcript Operations ────────────────────────────

    /// Append a transcript entry.
    pub async fn append_transcript(&self, entry: &TranscriptEntry) -> crate::Result<()> {
        let conn = self.conn.lock().await;
        let tool_calls_json = entry
            .tool_calls
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO transcripts (id, session_id, parent_id, role, content, tool_calls_json, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.session_id,
                entry.parent_id,
                entry.role,
                entry.content,
                tool_calls_json,
                entry.timestamp,
            ],
        )?;
        Ok(())
    }

    /// Get all transcript entries for a session (ordered by timestamp).
    pub async fn get_transcripts(
        &self,
        session_id: &str,
    ) -> crate::Result<Vec<TranscriptEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, parent_id, role, content, tool_calls_json, timestamp
             FROM transcripts WHERE session_id = ?1 ORDER BY timestamp",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let tool_calls_json: Option<String> = row.get(5)?;
            let tool_calls = tool_calls_json
                .and_then(|s| serde_json::from_str(&s).ok());
            Ok(TranscriptEntry {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_id: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                tool_calls,
                timestamp: row.get(6)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ── Turn / ToolCall Operations ───────────────────────

    /// Save a conversation turn.
    pub async fn save_turn(&self, turn: &Turn) -> crate::Result<()> {
        let conn = self.conn.lock().await;
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

    /// Get all turns for a session (with their tool calls).
    pub async fn get_turns(&self, session_id: &str) -> crate::Result<Vec<Turn>> {
        let conn = self.conn.lock().await;

        // First, collect turn row data
        #[allow(clippy::type_complexity)] // SQL row tuple; a type alias would add indirection for a one-off query
        let turn_rows: Vec<(String, u32, String, String, Option<String>, String, u32)> = {
            let mut stmt = conn.prepare(
                "SELECT turn_id, turn_index, role, content, thinking, created_at, tokens_used
                 FROM turns WHERE session_id = ?1 ORDER BY turn_index",
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
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
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };

        // Then, for each turn, fetch its tool calls
        let mut turns = Vec::new();
        for (turn_id, turn_index, role, content, thinking, created_at, tokens_used) in turn_rows {
            let tool_calls = Self::get_tool_calls_for_turn_sync(&conn, &turn_id)?;
            turns.push(Turn {
                turn_id,
                session_id: session_id.to_string(),
                turn_index,
                role,
                content,
                thinking,
                tool_calls,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .unwrap_or_default()
                    .with_timezone(&Utc),
                tokens_used,
            });
        }
        Ok(turns)
    }

    /// Save a tool call record.
    pub async fn save_tool_call(
        &self,
        call: &ToolCallRecord,
        turn_id: &str,
        session_id: &str,
    ) -> crate::Result<()> {
        let conn = self.conn.lock().await;
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

    /// Get all tool calls for a session.
    pub async fn get_tool_calls(
        &self,
        session_id: &str,
    ) -> crate::Result<Vec<ToolCallRecord>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT call_id, tool_name, input_summary, output_summary, success, duration_ms
             FROM tool_calls WHERE session_id = ?1 ORDER BY call_id",
        )?;
        let rows = stmt.query_map(params![session_id], Self::row_to_tool_call)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ── Audit ────────────────────────────────────────────

    /// Generate a session audit report.
    pub async fn audit_report(
        &self,
        session_id: &str,
    ) -> crate::Result<SessionAuditReport> {
        let unified_meta = self
            .get_session(session_id)
            .await?
            .ok_or_else(|| {
                crate::Error::Tool(format!("Session not found: {}", session_id))
            })?;
        let turns = self.get_turns(session_id).await?;
        let tool_calls = self.get_tool_calls(session_id).await?;

        let successful_calls = tool_calls.iter().filter(|c| c.success).count();
        let failed_calls = tool_calls.iter().filter(|c| !c.success).count();
        let total_duration_ms: u64 = tool_calls.iter().map(|c| c.duration_ms).sum();

        // Convert unified SessionMeta to store::SessionMeta for the audit report type
        let store_meta = crate::session::store::SessionMeta {
            session_id: unified_meta.session_id,
            agent_name: unified_meta.agent_name,
            model: unified_meta.model,
            working_dir: unified_meta.working_dir,
            status: match unified_meta.status {
                SessionStatus::Active => crate::session::store::SessionStatus::Active,
                SessionStatus::Paused => crate::session::store::SessionStatus::Paused,
                SessionStatus::Completed => {
                    crate::session::store::SessionStatus::Completed
                }
                SessionStatus::Failed => crate::session::store::SessionStatus::Failed,
                SessionStatus::Deleted => crate::session::store::SessionStatus::Deleted,
            },
            created_at: chrono::DateTime::parse_from_rfc3339(&unified_meta.created_at)
                .unwrap_or_default()
                .with_timezone(&Utc),
            updated_at: chrono::DateTime::parse_from_rfc3339(&unified_meta.updated_at)
                .unwrap_or_default()
                .with_timezone(&Utc),
            turn_count: unified_meta.turn_count as u32,
            tool_call_count: unified_meta.tool_call_count as u32,
            total_tokens: unified_meta.total_tokens,
        };

        Ok(SessionAuditReport {
            session: store_meta,
            turns,
            tool_calls,
            successful_calls,
            failed_calls,
            total_duration_ms,
        })
    }

    /// Get the database file path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    // ── Internal Helpers ─────────────────────────────────

    fn row_to_agent_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentConfigModel> {
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

    fn row_to_unified_meta(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMeta> {
        Ok(SessionMeta {
            session_id: row.get(0)?,
            agent_name: row.get(1)?,
            model: row.get(2)?,
            working_dir: row.get(3)?,
            status: SessionStatus::from_str(&row.get::<_, String>(4)?),
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            turn_count: row.get(7)?,
            tool_call_count: row.get(8)?,
            total_tokens: row.get(9)?,
        })
    }

    fn row_to_tool_call(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRecord> {
        Ok(ToolCallRecord {
            call_id: row.get(0)?,
            tool_name: row.get(1)?,
            input_summary: row.get(2)?,
            output_summary: row.get(3)?,
            success: row.get::<_, i32>(4)? != 0,
            duration_ms: row.get(5)?,
        })
    }

    /// Synchronous helper: fetch tool calls for a single turn (used inside `get_turns`).
    fn get_tool_calls_for_turn_sync(
        conn: &Connection,
        turn_id: &str,
    ) -> crate::Result<Vec<ToolCallRecord>> {
        let mut stmt = conn.prepare(
            "SELECT call_id, tool_name, input_summary, output_summary, success, duration_ms
             FROM tool_calls WHERE turn_id = ?1",
        )?;
        let rows = stmt.query_map(params![turn_id], Self::row_to_tool_call)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Synchronous helper: get a session while already holding the connection lock.
    fn get_session_sync(
        conn: &Connection,
        session_id: &str,
    ) -> crate::Result<Option<SessionMeta>> {
        let mut stmt = conn.prepare(
            "SELECT session_id, agent_name, model, working_dir, status,
                    created_at, updated_at, turn_count, tool_call_count, total_tokens
             FROM sessions WHERE session_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![session_id], Self::row_to_unified_meta)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
}

// ============================================================
//  Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    async fn test_store() -> UnifiedStore {
        UnifiedStore::open_path(Path::new(":memory:"))
            .await
            .expect("open in-memory store")
    }

    fn make_agent(id: &str) -> AgentConfigModel {
        AgentConfigModel {
            id: id.to_string(),
            name: "Test Agent".to_string(),
            model: "deepseek-chat".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            tools_json: r#"["read","write"]"#.to_string(),
            mcp_servers_json: "[]".to_string(),
            max_turns: 20,
            max_tool_calls: 30,
            token_budget: 128000,
            thinking: false,
            reasoning_effort: "medium".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_session(id: &str) -> SessionMeta {
        SessionMeta {
            session_id: id.to_string(),
            agent_name: "test-agent".to_string(),
            model: "deepseek-chat".to_string(),
            working_dir: "/tmp/test".to_string(),
            status: SessionStatus::Active,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        }
    }

    #[tokio::test]
    async fn test_open_creates_tables() {
        let store = test_store().await;
        let conn = store.conn.lock().await;

        // Verify every expected table exists
        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
                )
                .unwrap();
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap();
            let mut v = Vec::new();
            for r in rows {
                v.push(r.unwrap());
            }
            v
        };

        assert!(tables.contains(&"agents".to_string()));
        assert!(tables.contains(&"session_snapshots".to_string()));
        assert!(tables.contains(&"sessions".to_string()));
        assert!(tables.contains(&"turns".to_string()));
        assert!(tables.contains(&"tool_calls".to_string()));
        assert!(tables.contains(&"transcripts".to_string()));
    }

    #[tokio::test]
    async fn test_agent_crud() {
        let store = test_store().await;
        let agent = make_agent("agent-1");

        // Create
        store.create_agent(&agent).await.unwrap();

        // Get
        let fetched = store.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(fetched.name, "Test Agent");
        assert_eq!(fetched.model, "deepseek-chat");
        assert_eq!(fetched.max_turns, 20);

        // List
        let list = store.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);

        // Update
        let mut updated = agent.clone();
        updated.name = "Updated Agent".to_string();
        updated.max_turns = 50;
        let did_update = store.update_agent("agent-1", &updated).await.unwrap();
        assert!(did_update);

        let fetched2 = store.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(fetched2.name, "Updated Agent");
        assert_eq!(fetched2.max_turns, 50);

        // Delete
        let did_delete = store.delete_agent("agent-1").await.unwrap();
        assert!(did_delete);

        let gone = store.get_agent("agent-1").await.unwrap();
        assert!(gone.is_none());

        // Delete non-existent returns false
        let did_delete2 = store.delete_agent("agent-1").await.unwrap();
        assert!(!did_delete2);
    }

    #[tokio::test]
    async fn test_session_crud() {
        let store = test_store().await;
        let session = make_session("sess-1");

        // Create
        store.create_session(&session).await.unwrap();

        // Get
        let fetched = store.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched.agent_name, "test-agent");
        assert_eq!(fetched.status, SessionStatus::Active);

        // List
        let list = store.list_sessions(10).await.unwrap();
        assert_eq!(list.len(), 1);

        // Update status
        store
            .update_session_status("sess-1", SessionStatus::Completed)
            .await
            .unwrap();
        let fetched2 = store.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched2.status, SessionStatus::Completed);

        // Update stats
        store
            .update_session_stats("sess-1", 5, 10, 2000)
            .await
            .unwrap();
        let fetched3 = store.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched3.turn_count, 5);
        assert_eq!(fetched3.tool_call_count, 10);
        assert_eq!(fetched3.total_tokens, 2000);

        // Delete (cascades to related rows)
        store.delete_session("sess-1").await.unwrap();
        let gone = store.get_session("sess-1").await.unwrap();
        assert!(gone.is_none());
    }

    #[tokio::test]
    async fn test_transcript_roundtrip() {
        let store = test_store().await;
        let session = make_session("sess-t");
        store.create_session(&session).await.unwrap();

        let entry1 = TranscriptEntry {
            id: "t-1".to_string(),
            session_id: "sess-t".to_string(),
            parent_id: None,
            role: "user".to_string(),
            content: "Hello, agent!".to_string(),
            tool_calls: None,
            timestamp: "2025-01-01T00:00:01Z".to_string(),
        };
        let entry2 = TranscriptEntry {
            id: "t-2".to_string(),
            session_id: "sess-t".to_string(),
            parent_id: Some("t-1".to_string()),
            role: "assistant".to_string(),
            content: "Hello! How can I help?".to_string(),
            tool_calls: Some(serde_json::json!([{"name": "read", "args": {}}])),
            timestamp: "2025-01-01T00:00:02Z".to_string(),
        };

        store.append_transcript(&entry1).await.unwrap();
        store.append_transcript(&entry2).await.unwrap();

        let transcripts = store.get_transcripts("sess-t").await.unwrap();
        assert_eq!(transcripts.len(), 2);

        assert_eq!(transcripts[0].id, "t-1");
        assert_eq!(transcripts[0].role, "user");
        assert_eq!(transcripts[0].content, "Hello, agent!");
        assert!(transcripts[0].parent_id.is_none());
        assert!(transcripts[0].tool_calls.is_none());

        assert_eq!(transcripts[1].id, "t-2");
        assert_eq!(transcripts[1].role, "assistant");
        assert_eq!(transcripts[1].parent_id, Some("t-1".to_string()));
        assert!(transcripts[1].tool_calls.is_some());
    }

    #[tokio::test]
    async fn test_snapshot_operations() {
        let store = test_store().await;

        let snap1 = SessionSnapshot {
            snapshot_id: "snap-1".to_string(),
            session_id: "sess-s".to_string(),
            agent_id: "agent-1".to_string(),
            turn_index: 1,
            tool_name: "edit".to_string(),
            target_path: "/tmp/file.txt".to_string(),
            content_before: Some("original content".to_string()),
            created_at: "2025-01-01T00:00:01Z".to_string(),
        };
        let snap2 = SessionSnapshot {
            snapshot_id: "snap-2".to_string(),
            session_id: "sess-s".to_string(),
            agent_id: "agent-1".to_string(),
            turn_index: 2,
            tool_name: "edit".to_string(),
            target_path: "/tmp/file.txt".to_string(),
            content_before: Some("modified content".to_string()),
            created_at: "2025-01-01T00:00:02Z".to_string(),
        };
        let snap3 = SessionSnapshot {
            snapshot_id: "snap-3".to_string(),
            session_id: "sess-s".to_string(),
            agent_id: "agent-1".to_string(),
            turn_index: 3,
            tool_name: "write".to_string(),
            target_path: "/tmp/new_file.txt".to_string(),
            content_before: None, // file didn't exist before
            created_at: "2025-01-01T00:00:03Z".to_string(),
        };

        store.save_snapshot(&snap1).await.unwrap();
        store.save_snapshot(&snap2).await.unwrap();
        store.save_snapshot(&snap3).await.unwrap();

        // Get snapshots (ordered turn DESC, created_at DESC)
        let snapshots = store.get_snapshots("sess-s").await.unwrap();
        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].turn_index, 3); // most recent first
        assert_eq!(snapshots[2].turn_index, 1);

        // Rollback to turn 2:
        //   /tmp/file.txt     -> restore to "modified content" (snap-2, turn 2 is most recent)
        //   /tmp/new_file.txt -> delete (snap-3, content_before is None)
        let actions = store.rollback_to_turn("sess-s", 2).await.unwrap();
        assert_eq!(actions.len(), 2);
        let file_action = actions
            .iter()
            .find(|a| a.target_path == "/tmp/file.txt")
            .unwrap();
        assert_eq!(
            file_action.content_before,
            Some("modified content".to_string())
        );
        let new_file_action = actions
            .iter()
            .find(|a| a.target_path == "/tmp/new_file.txt")
            .unwrap();
        assert!(new_file_action.content_before.is_none());

        // Cleanup snapshots after turn 2
        store.cleanup_snapshots_after("sess-s", 2).await.unwrap();
        let remaining = store.get_snapshots("sess-s").await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].turn_index, 1);
    }
}
