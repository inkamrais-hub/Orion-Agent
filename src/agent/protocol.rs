//! A2A 通信协议
//!
//! 定义 Agent 间通信的消息类型

use serde::{Deserialize, Serialize};
use crate::agent::AgentId;

/// A2A 消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum A2AMessage {
    /// 请求信息
    RequestInfo {
        from: AgentId,
        query: String,
    },
    /// 分享结果
    ShareResult {
        from: AgentId,
        task_id: String,
        content: String,
    },
    /// 请求帮助
    RequestHelp {
        from: AgentId,
        task_id: String,
        issue: String,
    },
    /// 回应
    Response {
        from: AgentId,
        to: String,
        content: String,
    },
    /// 广播状态更新
    StatusUpdate {
        from: AgentId,
        status: String,
        progress: f32,
    },
}

impl A2AMessage {
    pub fn from_agent(&self) -> &AgentId {
        match self {
            Self::RequestInfo { from, .. } => from,
            Self::ShareResult { from, .. } => from,
            Self::RequestHelp { from, .. } => from,
            Self::Response { from, .. } => from,
            Self::StatusUpdate { from, .. } => from,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}
