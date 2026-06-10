//! Agent 配置持久化存储 (SQLite)

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Agent 配置模型 (持久化)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigModel {
    pub id: String,
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    /// JSON 数组，如 ["read", "bash", "edit"]
    pub tools_json: String,
    /// JSON 数组，如 [{"name":"fs","command":"npx","args":["-y","@anthropic/mcp-fs"]}]
    pub mcp_servers_json: String,
    pub max_turns: i64,
    pub max_tool_calls: i64,
    pub token_budget: i64,
    pub thinking: bool,
    pub reasoning_effort: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Session 回滚快照（工具执行前的文件备份）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub turn_index: i64,
    pub tool_name: String,
    pub target_path: String,
    /// None = 文件不存在（新建场景）
    pub content_before: Option<String>,
    pub created_at: String,
}

/// 回滚动作：描述单个文件如何恢复
#[derive(Debug, Clone)]
pub struct RollbackAction {
    pub target_path: String,
    /// Some = 恢复文件内容, None = 删除文件（文件是本轮新建的）
    pub content_before: Option<String>,
}

/// Agent 配置持久化存储
pub struct AgentStore {
    conn: Arc<Mutex<Connection>>,
}

impl AgentStore {
    /// 打开（或创建）SQLite 数据库
    pub fn new(db_path: &Path) -> crate::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agents (
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
            CREATE INDEX IF NOT EXISTS idx_snapshots_session ON session_snapshots(session_id);
            CREATE INDEX IF NOT EXISTS idx_snapshots_session_turn ON session_snapshots(session_id, turn_index);
            ",
        )?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// 列出全部 Agent 配置
    pub fn list(&self) -> crate::Result<Vec<AgentConfigModel>> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let mut stmt = conn.prepare(
            "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                    max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                    created_at, updated_at
             FROM agents ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_model)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// 按 ID 获取单个配置
    pub fn get(&self, id: &str) -> crate::Result<Option<AgentConfigModel>> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let mut stmt = conn.prepare(
            "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                    max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                    created_at, updated_at
             FROM agents WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], Self::row_to_model)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// 新增一条配置
    pub fn create(&self, config: &AgentConfigModel) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        conn.execute(
            "INSERT INTO agents (id, name, model, system_prompt, tools_json, mcp_servers_json,
                                max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
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

    /// 更新指定 ID 的配置
    pub fn update(&self, id: &str, config: &AgentConfigModel) -> crate::Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let now = chrono::Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE agents SET name=?1, model=?2, system_prompt=?3, tools_json=?4,
                    mcp_servers_json=?5, max_turns=?6, max_tool_calls=?7, token_budget=?8,
                    thinking=?9, reasoning_effort=?10, updated_at=?11
             WHERE id=?12",
            rusqlite::params![
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

    /// 删除指定 ID 的配置，返回是否真正删除了记录
    pub fn delete(&self, id: &str) -> crate::Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let rows = conn.execute("DELETE FROM agents WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }

    // ── 快照操作 ──────────────────────────────────────

    /// 保存回滚快照（工具执行前的文件备份）
    pub fn save_snapshot(&self, snapshot: &SessionSnapshot) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        conn.execute(
            "INSERT INTO session_snapshots
                (snapshot_id, session_id, agent_id, turn_index, tool_name, target_path, content_before, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
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

    /// 获取指定 session 的所有快照（按 turn_index DESC, created_at DESC）
    pub fn get_snapshots(&self, session_id: &str) -> crate::Result<Vec<SessionSnapshot>> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let mut stmt = conn.prepare(
            "SELECT snapshot_id, session_id, agent_id, turn_index, tool_name,
                    target_path, content_before, created_at
             FROM session_snapshots
             WHERE session_id = ?1
             ORDER BY turn_index DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id], Self::row_to_snapshot)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// 回滚到指定轮次：返回需要恢复的文件列表
    ///
    /// 筛选 `turn_index >= target_turn` 的快照，按 turn_index DESC, created_at DESC 排序，
    /// 对每个 target_path 只保留最新的快照（后改的先恢复）。
    pub fn rollback_to_turn(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        let mut stmt = conn.prepare(
            "SELECT target_path, content_before
             FROM session_snapshots
             WHERE session_id = ?1 AND turn_index >= ?2
             ORDER BY turn_index DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id, turn_index as i64], |row| {
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
    }

    /// 清理指定轮次之后的快照
    pub fn cleanup_snapshots_after(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| {
            tracing::warn!("SQLite mutex was poisoned, recovering");
            poisoned.into_inner()
        });
        conn.execute(
            "DELETE FROM session_snapshots
             WHERE session_id = ?1 AND turn_index >= ?2",
            rusqlite::params![session_id, turn_index],
        )?;
        Ok(())
    }

    // ── Async API (spawn_blocking) ─────────────────────

    /// Async wrapper for list() - runs SQLite query on blocking thread pool
    pub async fn list_async(&self) -> crate::Result<Vec<AgentConfigModel>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                        max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                        created_at, updated_at
                 FROM agents ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], Self::row_to_model)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for get() - runs SQLite query on blocking thread pool
    pub async fn get_async(&self, id: String) -> crate::Result<Option<AgentConfigModel>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT id, name, model, system_prompt, tools_json, mcp_servers_json,
                        max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                        created_at, updated_at
                 FROM agents WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map([id.as_str()], Self::row_to_model)?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for create() - runs SQLite insert on blocking thread pool
    pub async fn create_async(&self, config: AgentConfigModel) -> crate::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO agents (id, name, model, system_prompt, tools_json, mcp_servers_json,
                                    max_turns, max_tool_calls, token_budget, thinking, reasoning_effort,
                                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
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
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for update() - runs SQLite update on blocking thread pool
    pub async fn update_async(&self, id: String, config: AgentConfigModel) -> crate::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let now = chrono::Utc::now().to_rfc3339();
            let rows = conn.execute(
                "UPDATE agents SET name=?1, model=?2, system_prompt=?3, tools_json=?4,
                        mcp_servers_json=?5, max_turns=?6, max_tool_calls=?7, token_budget=?8,
                        thinking=?9, reasoning_effort=?10, updated_at=?11
                 WHERE id=?12",
                rusqlite::params![
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
                    id.as_str(),
                ],
            )?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for delete() - runs SQLite delete on blocking thread pool
    pub async fn delete_async(&self, id: String) -> crate::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let rows = conn.execute("DELETE FROM agents WHERE id = ?1", [id.as_str()])?;
            Ok(rows > 0)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for save_snapshot() - runs SQLite insert on blocking thread pool
    pub async fn save_snapshot_async(&self, snapshot: SessionSnapshot) -> crate::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "INSERT INTO session_snapshots
                    (snapshot_id, session_id, agent_id, turn_index, tool_name, target_path, content_before, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
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
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for get_snapshots() - runs SQLite query on blocking thread pool
    pub async fn get_snapshots_async(&self, session_id: String) -> crate::Result<Vec<SessionSnapshot>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT snapshot_id, session_id, agent_id, turn_index, tool_name,
                        target_path, content_before, created_at
                 FROM session_snapshots
                 WHERE session_id = ?1
                 ORDER BY turn_index DESC, created_at DESC",
            )?;
            let rows = stmt.query_map(rusqlite::params![session_id.as_str()], Self::row_to_snapshot)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for rollback_to_turn() - runs SQLite query on blocking thread pool
    pub async fn rollback_to_turn_async(
        &self,
        session_id: String,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = conn.prepare(
                "SELECT target_path, content_before
                 FROM session_snapshots
                 WHERE session_id = ?1 AND turn_index >= ?2
                 ORDER BY turn_index DESC, created_at DESC",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![session_id.as_str(), turn_index as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )?;

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
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    /// Async wrapper for cleanup_snapshots_after() - runs SQLite delete on blocking thread pool
    pub async fn cleanup_snapshots_after_async(
        &self,
        session_id: String,
        turn_index: u32,
    ) -> crate::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
            conn.execute(
                "DELETE FROM session_snapshots
                 WHERE session_id = ?1 AND turn_index >= ?2",
                rusqlite::params![session_id.as_str(), turn_index],
            )?;
            Ok(())
        })
        .await
        .map_err(|e| crate::Error::Agent(format!("spawn_blocking: {}", e)))?
    }

    // ── 内部辅助 ──────────────────────────────────────

    fn row_to_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentConfigModel> {
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
