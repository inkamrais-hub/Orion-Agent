//! Rollout — JSONL 不可变事件流
//!
//! 设计: JSONL 不可变追加 + SQLite 索引 (双轨)
//!
//! 好处:
//! - JSONL 保证数据不丢 (即使崩溃，已写入的还在)
//! - SQLite 保证查询快 (按 ID/时间/状态秒查)
//! - 可以"回放"整个对话过程

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::io::Write;
use chrono::{DateTime, Utc};

/// Rollout 事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RolloutEvent {
    /// 会话开始
    SessionStart {
        session_id: String,
        agent_name: String,
        model: String,
        timestamp: DateTime<Utc>,
    },
    /// 用户输入
    UserInput {
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// LLM 响应
    LlmResponse {
        content: String,
        thinking: Option<String>,
        timestamp: DateTime<Utc>,
    },
    /// 工具调用开始
    ToolStart {
        tool_name: String,
        input: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    /// 工具调用结束
    ToolEnd {
        tool_name: String,
        success: bool,
        output: String,
        duration_ms: u64,
        timestamp: DateTime<Utc>,
    },
    /// 会话结束
    SessionEnd {
        session_id: String,
        reason: String,
        timestamp: DateTime<Utc>,
    },
    /// 错误
    Error {
        message: String,
        source: String,
        timestamp: DateTime<Utc>,
    },
}

/// Rollout 记录器
pub struct RolloutRecorder {
    /// JSONL 文件路径
    file_path: PathBuf,
    /// 文件句柄
    file: std::fs::File,
    /// 已写入事件数
    event_count: u64,
}

impl RolloutRecorder {
    /// 创建新的记录器
    pub fn new(session_id: &str) -> crate::Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into());
        let rollout_dir = PathBuf::from(home).join(".orion").join("sessions").join(session_id);
        std::fs::create_dir_all(&rollout_dir)?;

        let file_path = rollout_dir.join("rollout.jsonl");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;

        Ok(Self {
            file_path,
            file,
            event_count: 0,
        })
    }

    /// 记录事件 (追加到 JSONL)
    pub fn record(&mut self, event: &RolloutEvent) -> crate::Result<()> {
        let json = serde_json::to_string(event)?;
        writeln!(self.file, "{}", json)?;
        self.file.flush()?;
        self.event_count += 1;
        Ok(())
    }

    /// 会话开始
    pub fn session_start(&mut self, session_id: &str, agent_name: &str, model: &str) -> crate::Result<()> {
        self.record(&RolloutEvent::SessionStart {
            session_id: session_id.to_string(),
            agent_name: agent_name.to_string(),
            model: model.to_string(),
            timestamp: Utc::now(),
        })
    }

    /// 用户输入
    pub fn user_input(&mut self, content: &str) -> crate::Result<()> {
        self.record(&RolloutEvent::UserInput {
            content: content.to_string(),
            timestamp: Utc::now(),
        })
    }

    /// LLM 响应
    pub fn llm_response(&mut self, content: &str, thinking: Option<&str>) -> crate::Result<()> {
        self.record(&RolloutEvent::LlmResponse {
            content: content.to_string(),
            thinking: thinking.map(|s| s.to_string()),
            timestamp: Utc::now(),
        })
    }

    /// 工具调用开始
    pub fn tool_start(&mut self, tool_name: &str, input: &serde_json::Value) -> crate::Result<()> {
        self.record(&RolloutEvent::ToolStart {
            tool_name: tool_name.to_string(),
            input: input.clone(),
            timestamp: Utc::now(),
        })
    }

    /// 工具调用结束
    pub fn tool_end(&mut self, tool_name: &str, success: bool, output: &str, duration_ms: u64) -> crate::Result<()> {
        self.record(&RolloutEvent::ToolEnd {
            tool_name: tool_name.to_string(),
            success,
            output: output.to_string(),
            duration_ms,
            timestamp: Utc::now(),
        })
    }

    /// 会话结束
    pub fn session_end(&mut self, session_id: &str, reason: &str) -> crate::Result<()> {
        self.record(&RolloutEvent::SessionEnd {
            session_id: session_id.to_string(),
            reason: reason.to_string(),
            timestamp: Utc::now(),
        })
    }

    /// 错误
    pub fn error(&mut self, message: &str, source: &str) -> crate::Result<()> {
        self.record(&RolloutEvent::Error {
            message: message.to_string(),
            source: source.to_string(),
            timestamp: Utc::now(),
        })
    }

    /// 获取已写入事件数
    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// 获取文件路径
    pub fn file_path(&self) -> &PathBuf {
        &self.file_path
    }
}

/// 从 JSONL 文件回放事件
pub fn replay(file_path: &PathBuf) -> crate::Result<Vec<RolloutEvent>> {
    let content = std::fs::read_to_string(file_path)?;
    let mut events = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<RolloutEvent>(line) {
            Ok(event) => events.push(event),
            Err(e) => {
                log_warn!("rollout", "跳过无效行: {}", e);
            }
        }
    }

    Ok(events)
}

// ── 依赖 ──────────────────────────────────────────────────

use crate::log_warn;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rollout_roundtrip() {
        let session_id = format!("test_rollout_{}", uuid::Uuid::new_v4());
        
        // 写入
        {
            let mut recorder = RolloutRecorder::new(&session_id).unwrap();
            recorder.session_start(&session_id, "main", "deepseek-chat").unwrap();
            recorder.user_input("hello").unwrap();
            recorder.llm_response("hi there", None).unwrap();
            recorder.session_end(&session_id, "user_exit").unwrap();
            assert_eq!(recorder.event_count(), 4);
        }

        // 回放
        let actual_path = crate::config::data_dir_path()
            .join("sessions")
            .join(&session_id)
            .join("rollout.jsonl");
        
        let events = replay(&actual_path).unwrap();
        assert_eq!(events.len(), 4);

        // 清理
        let _ = std::fs::remove_dir_all(actual_path.parent().unwrap());
    }
}
