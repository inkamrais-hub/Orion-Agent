//! Agent 注册表 — 管理 Agent 间通信通道
//!
//! 提供 Agent 注册、消息路由、广播、请求-响应功能

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, RwLock};
use crate::agent::{AgentId, runtime::AgentMessage};
use crate::agent::protocol::A2AMessage;

/// Agent 通信通道发送端
pub type AgentTx = mpsc::UnboundedSender<AgentMessage>;

/// 等待回复的超时时间 (秒)
const A2A_REPLY_TIMEOUT_SECS: u64 = 60;

/// Agent 注册表
pub struct AgentRegistry {
    agents: RwLock<HashMap<AgentId, AgentTx>>,
    /// 等待回复的 oneshot 发送端 (correlation_id → sender)
    pending_replies: RwLock<HashMap<String, oneshot::Sender<A2AMessage>>>,
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
            pending_replies: RwLock::new(HashMap::new()),
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

    // ── 请求-响应模式 (correlation ID + oneshot) ──

    /// 发送 A2A 消息并等待目标 Agent 回复
    ///
    /// 生成唯一 correlation_id，发送消息后阻塞等待目标 Agent 通过
    /// `deliver_reply()` 投递回复。超时返回错误。
    pub async fn send_and_wait(
        &self,
        to: &AgentId,
        a2a: A2AMessage,
        timeout_secs: Option<u64>,
    ) -> crate::Result<A2AMessage> {
        let correlation_id = uuid::Uuid::new_v4().to_string();
        let (reply_tx, reply_rx) = oneshot::channel();

        // 注册 pending reply
        self.pending_replies.write().await.insert(correlation_id.clone(), reply_tx);

        // 构造带 correlation_id 的消息
        let msg = AgentMessage {
            from: a2a.from_agent().clone(),
            to: to.clone(),
            content: a2a.to_json(),
            tool_call: None,
            metadata: Some(serde_json::json!({
                "correlation_id": correlation_id,
                "expects_reply": true,
            })),
        };

        // 发送
        if let Err(e) = self.send(to, msg).await {
            self.pending_replies.write().await.remove(&correlation_id);
            return Err(e);
        }

        // 等待回复 (带超时)
        let timeout = std::time::Duration::from_secs(
            timeout_secs.unwrap_or(A2A_REPLY_TIMEOUT_SECS)
        );

        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(reply)) => {
                tracing::info!(correlation_id = %correlation_id, from = %to, "A2A reply received");
                Ok(reply)
            }
            Ok(Err(_)) => {
                self.pending_replies.write().await.remove(&correlation_id);
                Err(crate::Error::Agent(format!(
                    "A2A reply channel dropped for correlation_id={}", correlation_id
                )))
            }
            Err(_) => {
                self.pending_replies.write().await.remove(&correlation_id);
                Err(crate::Error::Agent(format!(
                    "A2A reply timeout ({}s) for correlation_id={}",
                    timeout.as_secs(), correlation_id
                )))
            }
        }
    }

    /// 投递回复 — 由接收端 Agent 调用
    ///
    /// 根据 correlation_id 找到等待中的 oneshot 并发送回复。
    /// 返回 true 表示成功投递，false 表示无匹配等待者。
    pub async fn deliver_reply(&self, correlation_id: &str, reply: A2AMessage) -> bool {
        let mut pending = self.pending_replies.write().await;
        if let Some(tx) = pending.remove(correlation_id) {
            tx.send(reply).is_ok()
        } else {
            tracing::warn!(correlation_id = %correlation_id, "No pending reply for this correlation_id");
            false
        }
    }

    /// 清理超时的 pending replies (可定期调用)
    pub async fn cleanup_stale_replies(&self) {
        let pending = self.pending_replies.read().await;
        let count = pending.len();
        drop(pending);
        if count > 0 {
            tracing::debug!(pending_count = count, "A2A pending replies");
        }
    }
}
