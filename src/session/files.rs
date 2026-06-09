//! Session 文件管理
//!
//! 目录结构:
//!   ~/.config/orion/sessions/{session_id}/
//!     ├── transcript.jsonl  ← 对话记录
//!     ├── audit.jsonl       ← 审计日志
//!     └── meta.json         ← 元数据
//!
//! 删除流程:
//!   软删除: sessions/{id} → trash/{id} + SQLite 标记 deleted
//!   硬删除: 1周后物理删除 trash/{id} + SQLite 删除记录

use std::path::PathBuf;
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};

/// Session 文件管理器
pub struct SessionFileManager {
    sessions_dir: PathBuf,
    trash_dir: PathBuf,
}

impl SessionFileManager {
    /// 创建文件管理器
    pub fn new() -> Self {
        let base = crate::config::data_dir_path();
        
        Self {
            sessions_dir: base.join("sessions"),
            trash_dir: base.join("trash"),
        }
    }

    /// 初始化目录结构
    pub fn init(&self) -> crate::Result<()> {
        std::fs::create_dir_all(&self.sessions_dir)?;
        std::fs::create_dir_all(&self.trash_dir)?;
        Ok(())
    }

    /// 获取 Session 目录路径
    pub fn session_path(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }

    /// 获取 Session 文件路径
    pub fn transcript_path(&self, session_id: &str) -> PathBuf {
        self.session_path(session_id).join("transcript.jsonl")
    }

    pub fn audit_path(&self, session_id: &str) -> PathBuf {
        self.session_path(session_id).join("audit.jsonl")
    }

    pub fn meta_path(&self, session_id: &str) -> PathBuf {
        self.session_path(session_id).join("meta.json")
    }

    /// 创建 Session 目录
    pub fn create_session_dir(&self, session_id: &str) -> crate::Result<PathBuf> {
        let path = self.session_path(session_id);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// 检查 Session 目录是否存在
    pub fn session_exists(&self, session_id: &str) -> bool {
        self.session_path(session_id).exists()
    }

    /// 软删除: 移动到 trash 目录
    pub fn soft_delete(&self, session_id: &str) -> crate::Result<()> {
        let src = self.session_path(session_id);
        if !src.exists() {
            return Err(crate::Error::Tool(format!("Session 目录不存在: {}", session_id)));
        }

        let dst = self.trash_dir.join(session_id);
        
        // 如果 trash 中已存在，先删除
        if dst.exists() {
            std::fs::remove_dir_all(&dst)?;
        }

        std::fs::rename(&src, &dst)?;
        log_info!("session", "软删除 Session: {} → trash", session_id);
        Ok(())
    }

    /// 硬删除: 物理删除 trash 中的 Session
    pub fn hard_delete(&self, session_id: &str) -> crate::Result<()> {
        let path = self.trash_dir.join(session_id);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
            log_info!("session", "硬删除 Session: {}", session_id);
        }
        Ok(())
    }

    /// 恢复: 从 trash 移回 sessions
    pub fn restore(&self, session_id: &str) -> crate::Result<()> {
        let src = self.trash_dir.join(session_id);
        if !src.exists() {
            return Err(crate::Error::Tool(format!("Session 不在回收站: {}", session_id)));
        }

        let dst = self.session_path(session_id);
        if dst.exists() {
            return Err(crate::Error::Tool(format!("Session 目录已存在: {}", session_id)));
        }

        std::fs::rename(&src, &dst)?;
        log_info!("session", "恢复 Session: {} ← trash", session_id);
        Ok(())
    }

    /// 清理过期的软删除 (超过 1 周)
    pub fn cleanup_expired(&self) -> crate::Result<Vec<String>> {
        let one_week_ago = Utc::now() - Duration::weeks(1);
        let mut cleaned = Vec::new();

        if !self.trash_dir.exists() {
            return Ok(cleaned);
        }

        for entry in std::fs::read_dir(&self.trash_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // 检查 meta.json 中的删除时间
            let meta_path = path.join("meta.json");
            if let Ok(content) = std::fs::read_to_string(&meta_path) {
                if let Ok(meta) = serde_json::from_str::<SessionFileMeta>(&content) {
                    if let Some(deleted_at) = meta.deleted_at {
                        if deleted_at < one_week_ago {
                            if let Some(session_id) = path.file_name().and_then(|n| n.to_str()) {
                                std::fs::remove_dir_all(&path)?;
                                cleaned.push(session_id.to_string());
                                log_info!("session", "清理过期 Session: {}", session_id);
                            }
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }

    /// 列出所有 Session 目录
    pub fn list_session_dirs(&self) -> crate::Result<Vec<String>> {
        let mut result = Vec::new();
        if !self.sessions_dir.exists() {
            return Ok(result);
        }

        for entry in std::fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    result.push(name.to_string());
                }
            }
        }

        Ok(result)
    }

    /// 列出 trash 中的 Session
    pub fn list_trash_dirs(&self) -> crate::Result<Vec<String>> {
        let mut result = Vec::new();
        if !self.trash_dir.exists() {
            return Ok(result);
        }

        for entry in std::fs::read_dir(&self.trash_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    result.push(name.to_string());
                }
            }
        }

        Ok(result)
    }
}

impl Default for SessionFileManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Session 文件元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFileMeta {
    pub session_id: String,
    pub agent_name: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

// ── 依赖 ──────────────────────────────────────────────────

use crate::log_info;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_paths() {
        let fm = SessionFileManager::new();
        let id = "session_20240101_120000_a3f2b1c8";
        
        assert!(fm.session_path(id).ends_with(id));
        assert!(fm.transcript_path(id).ends_with("transcript.jsonl"));
        assert!(fm.audit_path(id).ends_with("audit.jsonl"));
        assert!(fm.meta_path(id).ends_with("meta.json"));
    }
}
