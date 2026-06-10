use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::AgentId;

// ============================================================
//  积木: Agent 间消息
//  参考: OpenClaw ACP 协议风格
// ============================================================

/// Agent 间消息
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
    pub tool_call: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Agent 通信通道
pub type AgentChannel = mpsc::UnboundedSender<AgentMessage>;

/// 可接收消息的 Agent
#[async_trait]
pub trait MessageHandler: Send {
    async fn handle_message(&mut self, msg: AgentMessage) -> crate::Result<()>;
}
