pub mod runtime;
pub mod lanes;
pub mod protocol;
pub mod registry;
pub mod store;

use uuid::Uuid;

/// Agent 标识
pub type AgentId = String;

/// 会话标识
pub type SessionId = String;

/// Lane 标识
pub type LaneId = String;

/// 生成新会话 ID
pub fn new_session_id() -> SessionId {
    Uuid::new_v4().to_string()
}

/// 生成新 Agent ID
pub fn new_agent_id() -> AgentId {
    format!("agent_{}", Uuid::new_v4().to_string().split('-').next().unwrap_or("0"))
}
