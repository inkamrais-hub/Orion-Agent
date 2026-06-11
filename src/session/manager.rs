//! Session 管理器
//!
//! 负责:
//!   - 创建/恢复/列出/删除 Session
//!   - 追加对话记录到 .jsonl
//!   - 维护 sessions.json 索引

use std::path::PathBuf;
use std::collections::HashMap;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use chrono::Utc;

use super::{SessionEntry, SessionStatus, TranscriptEntry};

/// 验证 session_id 是否安全
fn validate_session_id(id: &str) -> crate::Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(crate::Error::Agent(format!(
            "Invalid session_id: '{}'",
            id
        )));
    }
    Ok(())
}

/// Session 管理器
pub struct SessionManager {
    session_dir: PathBuf,
    index_path: PathBuf,
}

impl SessionManager {
    /// 打开 session 目录 (不存在则创建)
    pub async fn open() -> crate::Result<Self> {
        let session_dir = session_dir()?;
        fs::create_dir_all(&session_dir).await.map_err(|e| {
            crate::Error::Io(std::io::Error::other(
                format!("Cannot create {:?}: {}", session_dir, e),
            ))
        })?;
        let index_path = session_dir.join("sessions.json");
        Ok(Self { session_dir, index_path })
    }

    /// 创建新 session
    pub async fn create(&self, model: &str) -> crate::Result<String> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        let entry = SessionEntry {
            session_id: session_id.clone(),
            created_at: now,
            updated_at: now,
            model: model.to_string(),
            turn_count: 0,
            total_tokens: 0,
            status: SessionStatus::Active,
        };

        // 更新索引
        let mut index = self.load_index().await;
        index.insert(format!("agent:{}", session_id), entry);
        self.save_index(&index).await?;

        // 创建空 .jsonl
        let jsonl_path = self.session_dir.join(format!("{}.jsonl", session_id));
        fs::write(&jsonl_path, "").await?;

        tracing::info!(session_id = %session_id, model = %model, "Session created");
        Ok(session_id)
    }

    /// 恢复 session (读取对话记录)
    pub async fn restore(&self, session_id: &str) -> crate::Result<Vec<TranscriptEntry>> {
        validate_session_id(session_id)?;
        let jsonl_path = self.session_dir.join(format!("{}.jsonl", session_id));
        let content = fs::read_to_string(&jsonl_path).await.unwrap_or_default();

        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() { continue; }
            if let Ok(entry) = serde_json::from_str::<TranscriptEntry>(line) {
                entries.push(entry);
            }
        }

        tracing::info!(session_id = %session_id, messages = entries.len(), "Session restored");
        Ok(entries)
    }

    /// 追加对话消息
    pub async fn append_transcript(&self, session_id: &str, entry: &TranscriptEntry) -> crate::Result<()> {
        validate_session_id(session_id)?;
        let jsonl_path = self.session_dir.join(format!("{}.jsonl", session_id));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&jsonl_path)
            .await?;

        let line = serde_json::to_string(entry)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }

    /// 更新 session 元数据
    pub async fn update(&self, session_id: &str, update_fn: impl FnOnce(&mut SessionEntry)) -> crate::Result<()> {
        validate_session_id(session_id)?;
        let mut index = self.load_index().await;
        let key = format!("agent:{}", session_id);
        if let Some(entry) = index.get_mut(&key) {
            update_fn(entry);
            entry.updated_at = Utc::now();
            self.save_index(&index).await?;
        }
        Ok(())
    }

    /// 列出所有 session
    pub async fn list(&self) -> crate::Result<Vec<SessionEntry>> {
        let index = self.load_index().await;
        Ok(index.into_values().collect())
    }

    /// 搜索 session (按内容关键词)
    pub async fn search(&self, query: &str) -> crate::Result<Vec<(SessionEntry, String)>> {
        let index = self.load_index().await;
        let mut results = Vec::new();

        for entry in index.values() {
            let jsonl_path = self.session_dir.join(format!("{}.jsonl", entry.session_id));
            if let Ok(content) = fs::read_to_string(&jsonl_path).await {
                if content.to_lowercase().contains(&query.to_lowercase()) {
                    // 提取第一个匹配行作为摘要
                    let snippet = content.lines()
                        .find(|l| l.to_lowercase().contains(&query.to_lowercase()))
                        .unwrap_or("")
                        .to_string();
                    results.push((entry.clone(), snippet));
                }
            }
        }

        Ok(results)
    }

    /// 删除 session
    pub async fn delete(&self, session_id: &str) -> crate::Result<()> {
        validate_session_id(session_id)?;
        let jsonl_path = self.session_dir.join(format!("{}.jsonl", session_id));
        let _ = fs::remove_file(&jsonl_path).await;

        let mut index = self.load_index().await;
        index.remove(&format!("agent:{}", session_id));
        self.save_index(&index).await?;

        tracing::info!(session_id = %session_id, "Session deleted");
        Ok(())
    }

    /// 加载索引
    async fn load_index(&self) -> HashMap<String, SessionEntry> {
        match fs::read_to_string(&self.index_path).await {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    /// 保存索引
    async fn save_index(&self, index: &HashMap<String, SessionEntry>) -> crate::Result<()> {
        let json = serde_json::to_string_pretty(index)?;
        fs::write(&self.index_path, json).await?;
        Ok(())
    }
}

/// Session 目录
fn session_dir() -> crate::Result<PathBuf> {
    Ok(crate::config::data_dir_path().join("sessions"))
}
