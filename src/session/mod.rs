//! Session 管理 — SQLite 持久化
//!
//! 功能:
//! - Session 创建/查询/恢复
//! - 消息历史持久化 (每轮对话)
//! - 工具调用记录
//! - 审计报告
//! - 快照接口预留

pub mod backend;
pub mod memory;
pub mod store;
pub mod files;
pub mod rollout;
pub mod sandbox;
pub mod unified;

pub use unified::UnifiedStore;
