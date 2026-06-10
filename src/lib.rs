//! 猎户座 Agent (Orion Agent) — 积木化 Rust Agent 框架
//!
//! 每个模块都是独立积木，可按需组合:
//!
//! ```text
//! agent-framework
//! ├── core/           # 核心抽象层
//! │   ├── provider    # LLM 提供商抽象
//! │   ├── agent       # Agent 构建器 + 对话接口
//! │   ├── loop        # 核心查询循环
//! │   ├── prompt      # 三段式 Prompt 构建器
//! │   ├── permission_broker # 统一安全决策
//! │   ├── exec_mode   # 执行模式 (Assist/Auto/Plan)
//! │   ├── execpolicy  # 命令权限策略
//! │   ├── guardrail   # 护栏系统
//! │   ├── cache       # 分层缓存
//! │   └── context     # 上下文管理
//! ├── tools/          # 工具系统
//! │   └── registry   # 工具注册表
//! ├── session/        # 会话持久化
//! │   ├── unified    # UnifiedStore (单一 SQLite)
//! │   ├── backend    # SessionBackend trait
//! │   └── memory     # 项目维度记忆
//! └── agent/          # Agent 间通信
//!     ├── protocol   # A2A 协议
//!     └── registry   # Agent 注册表
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
