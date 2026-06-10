//! A2A 通信协议
//!
//! 定义 Agent 间通信的消息类型，支持 Google A2A 规范启发的
//! correlation_id、时间戳与任务生命周期状态。

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::agent::AgentId;

// ---------------------------------------------------------------------------
// Task Lifecycle (inspired by Google A2A spec)
// ---------------------------------------------------------------------------

/// 任务生命周期状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskLifecycle {
    /// 任务已提交，等待处理
    Submitted,
    /// 正在执行
    Working,
    /// 需要额外输入
    InputRequired,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Canceled,
}

/// 任务状态追踪
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    pub task_id: String,
    pub lifecycle: TaskLifecycle,
    pub correlation_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub result: Option<String>,
    pub error: Option<String>,
}

impl TaskState {
    pub fn new(task_id: String) -> Self {
        let now = Utc::now();
        Self {
            task_id,
            lifecycle: TaskLifecycle::Submitted,
            correlation_id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            result: None,
            error: None,
        }
    }

    /// 转换为 Working 状态
    pub fn mark_working(&mut self) {
        self.lifecycle = TaskLifecycle::Working;
        self.updated_at = Utc::now();
    }

    /// 标记为需要输入
    pub fn mark_input_required(&mut self) {
        self.lifecycle = TaskLifecycle::InputRequired;
        self.updated_at = Utc::now();
    }

    /// 标记完成并附带结果
    pub fn mark_completed(&mut self, result: String) {
        self.lifecycle = TaskLifecycle::Completed;
        self.updated_at = Utc::now();
        self.result = Some(result);
    }

    /// 标记失败并附带错误信息
    pub fn mark_failed(&mut self, error: String) {
        self.lifecycle = TaskLifecycle::Failed;
        self.updated_at = Utc::now();
        self.error = Some(error);
    }

    /// 标记取消
    pub fn mark_canceled(&mut self) {
        self.lifecycle = TaskLifecycle::Canceled;
        self.updated_at = Utc::now();
    }
}

// ---------------------------------------------------------------------------
// A2A Message
// ---------------------------------------------------------------------------

/// A2A 消息类型
///
/// 每条消息均携带 `correlation_id`（UUID）用于请求-响应关联，
/// 以及 `timestamp` 用于排序。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum A2AMessage {
    /// 请求信息
    RequestInfo {
        from: AgentId,
        query: String,
        correlation_id: String,
        timestamp: DateTime<Utc>,
    },
    /// 分享结果
    ShareResult {
        from: AgentId,
        task_id: String,
        content: String,
        correlation_id: String,
        timestamp: DateTime<Utc>,
    },
    /// 请求帮助
    RequestHelp {
        from: AgentId,
        task_id: String,
        issue: String,
        correlation_id: String,
        timestamp: DateTime<Utc>,
    },
    /// 回应
    Response {
        from: AgentId,
        to: String,
        content: String,
        correlation_id: String,
        timestamp: DateTime<Utc>,
    },
    /// 广播状态更新
    StatusUpdate {
        from: AgentId,
        status: String,
        progress: f32,
        correlation_id: String,
        timestamp: DateTime<Utc>,
    },
}

impl A2AMessage {
    // -- 便捷构造函数（自动生成 correlation_id 和 timestamp）--

    pub fn new_request_info(from: AgentId, query: String) -> Self {
        Self::RequestInfo {
            from,
            query,
            correlation_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    pub fn new_share_result(from: AgentId, task_id: String, content: String) -> Self {
        Self::ShareResult {
            from,
            task_id,
            content,
            correlation_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    pub fn new_request_help(from: AgentId, task_id: String, issue: String) -> Self {
        Self::RequestHelp {
            from,
            task_id,
            issue,
            correlation_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    pub fn new_response(from: AgentId, to: String, content: String) -> Self {
        Self::Response {
            from,
            to,
            content,
            correlation_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    pub fn new_status_update(from: AgentId, status: String, progress: f32) -> Self {
        Self::StatusUpdate {
            from,
            status,
            progress,
            correlation_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        }
    }

    // -- 访问器 --

    pub fn from_agent(&self) -> &AgentId {
        match self {
            Self::RequestInfo { from, .. } => from,
            Self::ShareResult { from, .. } => from,
            Self::RequestHelp { from, .. } => from,
            Self::Response { from, .. } => from,
            Self::StatusUpdate { from, .. } => from,
        }
    }

    pub fn correlation_id(&self) -> &str {
        match self {
            Self::RequestInfo { correlation_id, .. } => correlation_id,
            Self::ShareResult { correlation_id, .. } => correlation_id,
            Self::RequestHelp { correlation_id, .. } => correlation_id,
            Self::Response { correlation_id, .. } => correlation_id,
            Self::StatusUpdate { correlation_id, .. } => correlation_id,
        }
    }

    pub fn timestamp(&self) -> &DateTime<Utc> {
        match self {
            Self::RequestInfo { timestamp, .. } => timestamp,
            Self::ShareResult { timestamp, .. } => timestamp,
            Self::RequestHelp { timestamp, .. } => timestamp,
            Self::Response { timestamp, .. } => timestamp,
            Self::StatusUpdate { timestamp, .. } => timestamp,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}
