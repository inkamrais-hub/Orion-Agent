//! 轻量级 Session Memory — 从对话中提取关键知识，跨 session 持久化
//!
//! 存储位置: ~/.orion/memories.json

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 记忆分类
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryCategory {
    UserPreference,  // 用户习惯、工具偏好
    ProjectFact,     // 代码架构、语言、框架
    CodePattern,     // 编码风格、约定
    Decision,        // 重要决策
    Constraint,      // 发现的限制
}

/// 单条记忆
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub category: MemoryCategory,
    pub content: String,
    pub confidence: f32,  // 0.0 - 1.0
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub session_id: String,
}

/// Session Memory 管理器
pub struct SessionMemory {
    memories: Vec<MemoryEntry>,
    file_path: PathBuf,
}

impl SessionMemory {
    /// 从 ~/.orion/memories.json 加载
    pub fn load() -> Self {
        let dir = memory_dir();
        let file_path = dir.join("memories.json");
        let memories = if file_path.exists() {
            let content = std::fs::read_to_string(&file_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self { memories, file_path }
    }

    /// 保存到磁盘
    pub fn save(&self) -> crate::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.memories)?;
        std::fs::write(&self.file_path, json)?;
        Ok(())
    }

    /// 添加新记忆（自动去重）
    pub fn add(&mut self, category: MemoryCategory, content: String, confidence: f32, session_id: &str) {
        if self.memories.iter().any(|m| m.content == content) {
            return;
        }
        self.memories.push(MemoryEntry {
            category,
            content,
            confidence,
            timestamp: chrono::Utc::now(),
            session_id: session_id.to_string(),
        });
    }

    /// 获取高置信度记忆，注入 system prompt
    pub fn as_context(&self) -> String {
        if self.memories.is_empty() {
            return String::new();
        }
        let mut parts = vec!["[Learned from previous sessions]".to_string()];
        for m in &self.memories {
            if m.confidence >= 0.7 {
                let cat = match m.category {
                    MemoryCategory::UserPreference => "Preference",
                    MemoryCategory::ProjectFact => "Project",
                    MemoryCategory::CodePattern => "Pattern",
                    MemoryCategory::Decision => "Decision",
                    MemoryCategory::Constraint => "Constraint",
                };
                parts.push(format!("- [{}] {}", cat, m.content));
            }
        }
        parts.join("\n")
    }

    /// 记忆条数
    pub fn len(&self) -> usize {
        self.memories.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
    }

    /// 按分类筛选
    pub fn by_category(&self, category: &MemoryCategory) -> Vec<&MemoryEntry> {
        self.memories.iter().filter(|m| &m.category == category).collect()
    }
}

/// 基于规则从一轮对话中提取记忆（无 LLM 调用）
pub fn extract_memories(user_input: &str, response: &str, _session_id: &str) -> Vec<(MemoryCategory, String)> {
    let mut memories = Vec::new();
    let lower = user_input.to_lowercase();

    // 检测用户偏好
    if lower.contains("用edit") || lower.contains("use edit") {
        memories.push((MemoryCategory::UserPreference, "User prefers using edit tool for small changes".into()));
    }
    if lower.contains("不要") || lower.contains("don't") {
        memories.push((MemoryCategory::UserPreference, format!("User instruction: {}", user_input)));
    }

    // 从工具结果中检测项目事实
    if response.contains("Cargo.toml") {
        memories.push((MemoryCategory::ProjectFact, "Project uses Rust/Cargo".into()));
    }
    if response.contains("package.json") {
        memories.push((MemoryCategory::ProjectFact, "Project uses Node.js/npm".into()));
    }

    memories
}

fn memory_dir() -> PathBuf {
    crate::config::data_dir_path()
}
