use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::agent::{AgentId, LaneId, SessionId};

// ============================================================
//  积木: Lanes (执行车道系统)
//  职责: 管理 Agent 执行队列, 防止资源竞争和死锁
//  参考: OpenClaw 的 lane 设计
//  核心: 每个 session + lane 组合有一条独立序列化队列
// ============================================================

/// Lane 执行许可
#[derive(Debug, Clone)]
pub struct LaneToken {
    pub session_id: SessionId,
    pub lane_id: LaneId,
    pub agent_id: AgentId,
}

/// Lane 管理器
pub struct LaneManager {
    /// 活跃的 lane 令牌
    active: Arc<Mutex<HashMap<(SessionId, LaneId), AgentId>>>,
    /// 等待队列
    pending: Arc<Mutex<HashMap<(SessionId, LaneId), Vec<AgentId>>>>,
    /// Wakeup signal for agents waiting in the pending queue
    notify: tokio::sync::Notify,
}

impl LaneManager {
    pub fn new() -> Self {
        Self {
            active: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notify: tokio::sync::Notify::new(),
        }
    }

    /// 尝试获取 lane 许可 (非阻塞)
    pub async fn try_acquire(
        &self,
        session_id: &str,
        lane_id: &str,
        agent_id: &str,
    ) -> Option<LaneToken> {
        let key = (session_id.to_string(), lane_id.to_string());
        let mut active = self.active.lock().await;

        if active.contains_key(&key) {
            // lane 被占用, 加入等待队列
            let mut pending = self.pending.lock().await;
            pending.entry(key).or_default().push(agent_id.to_string());
            return None;
        }

        active.insert(key, agent_id.to_string());
        Some(LaneToken {
            session_id: session_id.to_string(),
            lane_id: lane_id.to_string(),
            agent_id: agent_id.to_string(),
        })
    }

    /// 释放 lane 许可
    pub async fn release(&self, token: &LaneToken) {
        let key = (token.session_id.clone(), token.lane_id.clone());
        let mut active = self.active.lock().await;

        // Verify the token owner — only the current holder may release
        if let Some(current_agent) = active.get(&key) {
            if current_agent != &token.agent_id {
                tracing::warn!(
                    "Lane release: agent {} tried to release lane owned by {}",
                    token.agent_id, current_agent
                );
                return;
            }
        }

        active.remove(&key);

        // Promote the next agent from the pending queue
        let mut pending = self.pending.lock().await;
        if let Some(queue) = pending.get_mut(&key) {
            if let Some(next_agent) = queue.pop() {
                active.insert(key.clone(), next_agent.clone());
                tracing::info!("Lane promoted agent {} to active", next_agent);
            }
            if queue.is_empty() {
                pending.remove(&key);
            }
        }

        // Signal any waiters that a slot may be available
        self.notify.notify_one();
    }

    /// 当前活跃的 agent 数量
    pub async fn active_count(&self) -> usize {
        self.active.lock().await.len()
    }

    /// Current pending queue total length
    pub async fn pending_count(&self) -> usize {
        self.pending.lock().await.values().map(|v| v.len()).sum()
    }

    /// Returns a reference to the internal `Notify` so callers that received
    /// `None` from `try_acquire` can `await notify.notified()` instead of
    /// polling.
    pub fn wait_notify(&self) -> &tokio::sync::Notify {
        &self.notify
    }
}

// ============================================================
//  内置 Lane 常量 (参考 OpenClaw)
// ============================================================

pub const LANE_DEFAULT: &str = "default";
pub const LANE_NESTED: &str = "nested";
pub const LANE_SUBAGENT: &str = "subagent";
pub const LANE_CRON: &str = "cron";

/// 为 session 生成序列化 lane key (防止嵌套死锁)
pub fn resolve_nested_lane(session_id: &str) -> String {
    format!("nested:{}", session_id)
}
