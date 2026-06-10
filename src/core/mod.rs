pub mod provider;
pub mod guardrail;
pub mod cache;
pub mod context;
pub mod audit; // DEPRECATED: 使用 crate::audit 代替
pub mod r#loop;
pub mod tool_executor;
pub mod providers;
pub mod workspace;
pub mod orionignore;
pub mod hooks;
pub mod execpolicy;
pub mod goal;
pub mod agent;

/// Provider 标识
pub type ProviderId = String;

/// Token 计数
pub type TokenCount = u64;
