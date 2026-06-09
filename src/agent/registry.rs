//! Agent 注册表 — 管理 Agent 间通信通道
//!
//! 提供 Agent 注册、消息路由、广播功能

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use crate::agent::{AgentId, runtime::AgentMessage};
use crate::agent::protocol::A2AMessage;

/// Agent 通信通道发送端
pub type AgentTx = mpsc::UnboundedSender<AgentMessage>;

/// Agent 注册表
pub struct AgentRegistry {
    agents: RwLock<HashMap<AgentId, AgentTx>>,
}

impl std::fmt::Debug for AgentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRegistry").finish()
    }
}

impl AgentRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            agents: RwLock::new(HashMap::new()),
        })
    }

    /// 注册一个 Agent，返回接收端
    pub async fn register(&self, id: AgentId) -> mpsc::UnboundedReceiver<AgentMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.agents.write().await.insert(id, tx);
        rx
    }

    /// 注册一个 Agent，使用已有的发送端
    pub async fn register_with_tx(&self, id: AgentId, tx: AgentTx) {
        self.agents.write().await.insert(id, tx);
    }

    /// 注销 Agent
    pub async fn unregister(&self, id: &AgentId) {
        self.agents.write().await.remove(id);
    }

    /// 发送消息给指定 Agent
    pub async fn send(&self, to: &AgentId, msg: AgentMessage) -> crate::Result<()> {
        let agents = self.agents.read().await;
        match agents.get(to) {
            Some(tx) => tx.send(msg)
                .map_err(|e| crate::Error::Agent(format!("Send to {} failed: {}", to, e))),
            None => Err(crate::Error::Agent(format!("Agent {} not found", to))),
        }
    }

    /// 广播消息给所有 Agent (除发送者)
    pub async fn broadcast(&self, from: &AgentId, msg: AgentMessage) {
        let agents = self.agents.read().await;
        for (id, tx) in agents.iter() {
            if id != from {
                let _ = tx.send(msg.clone());
            }
        }
    }

    /// 已注册 Agent 数量
    pub async fn count(&self) -> usize {
        self.agents.read().await.len()
    }

    /// 列出所有已注册 Agent ID
    pub async fn list_agents(&self) -> Vec<AgentId> {
        self.agents.read().await.keys().cloned().collect()
    }

    /// 发送 A2A 协议消息
    pub async fn send_a2a(&self, to: &AgentId, a2a: A2AMessage) -> crate::Result<()> {
        let msg = AgentMessage {
            from: a2a.from_agent().clone(),
            to: to.clone(),
            content: a2a.to_json(),
            tool_call: None,
            metadata: None,
        };
        self.send(to, msg).await
    }

    /// 广播 A2A 协议消息
    pub async fn broadcast_a2a(&self, from: &AgentId, a2a: A2AMessage) {
        let msg = AgentMessage {
            from: from.clone(),
            to: "*".to_string(),
            content: a2a.to_json(),
            tool_call: None,
            metadata: None,
        };
        self.broadcast(from, msg).await;
    }
}
