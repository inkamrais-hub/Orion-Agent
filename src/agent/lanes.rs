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
