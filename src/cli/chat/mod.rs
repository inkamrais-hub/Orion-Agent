//! 持久化聊天循环 (CLI 模式入口)
//!
//! 启动 → 自动恢复 session → 循环对话 → 退出时保存
//!
//! 由 `gateway::commands` 在用户选择 chat 模式时调用。

pub mod commands;
mod loop_;

pub use loop_::run;
