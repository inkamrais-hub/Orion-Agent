pub mod provider;
pub mod guardrail;
pub mod cache;
pub mod context;
pub mod audit;
pub mod r#loop;
pub mod tool_executor;
pub mod providers;
pub mod workspace;
pub mod orionignore;
pub mod hooks;
pub mod execpolicy;
pub mod permission_broker;
pub mod goal;
pub mod agent;
pub mod prompt;

/// Provider 标识
pub type ProviderId = String;

/// Token 计数
pub type TokenCount = u64;
