//! 猎户座 Agent (Orion Agent) — 积木化 Rust Agent 框架
//!
//! 每个模块都是独立积木，可按需组合:
//!
//! ```text
//! agent-framework
//! ├── core/           # 核心抽象层
//! │   ├── provider    # LLM 提供商抽象
//! │   ├── guardrail   # 护栏系统 (Permission/Budget/Hook)
//! │   ├── cache       # 分层缓存系统
//! │   ├── context     # 上下文管理 + 压缩
//! │   └── loop        # 核心查询循环
//! ├── tools/          # 工具系统
//! │   └── registry   # 工具注册表
//! └── agent/          # Agent 运行时
//!     ├── runtime    # Agent 执行体
//!     └── lanes      # 执行车道 (Lane 系统)
//! ```

pub mod core;
pub mod tools;
pub mod agent;
pub mod config;
pub mod orchestrator;
pub mod ui;
pub mod session;
pub mod cli;
pub mod index;
pub mod logging;
pub mod gateway;
pub mod model;
pub mod audit;
#[cfg(feature = "api")]
pub mod api;

// 框架级别的类型别名
pub mod prelude {
    pub use crate::agent::{AgentId, LaneId, SessionId};
    pub use crate::core::agent::{Agent, AgentBuilder, AgentConfig, AgentEvent};
    pub use crate::core::cache::CacheStats;
    pub use crate::core::context::{ContextUsage, CompactionStrategy};
    pub use crate::core::guardrail::GuardResult;
    pub use crate::core::provider::{ProviderRequest, StreamEvent};
    pub use crate::core::ProviderId;
    pub use crate::tools::ToolResult;
}

/// 统一错误类型
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Guardrail denied: {0}")]
    Guardrail(String),

    #[error("Context window exceeded")]
    ContextWindowExceeded,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Agent runtime error: {0}")]
    Agent(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;
