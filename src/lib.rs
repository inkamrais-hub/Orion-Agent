//! 猎户座 Agent (Orion Agent) — 积木化 Rust Agent 框架
//!
//! 每个模块都是独立积木，可按需组合:
//!
//! ```text
//! agent-framework
//! ├── core/           # 核心抽象层
//! │   ├── provider    # LLM 提供商抽象 (OpenAI/DeepSeek 兼容)
//! │   ├── agent       # Agent 构建器 + 多轮对话 + 流式事件
//! │   ├── loop        # 核心查询循环 (tool calling loop)
//! │   ├── guardrail   # 护栏系统 (Permission/Budget/Hook)
//! │   ├── cache       # 分层缓存系统
//! │   └── context     # 上下文管理 + 压缩
//! ├── tools/          # 工具系统
//! │   ├── registry    # 工具注册表 (HashMap + 延迟装载)
//! │   ├── agent_tool  # Sub-Agent 工具 (可配置工具集 + 结果缓存)
//! │   └── a2a_message # A2A 通信工具 (请求-响应模式)
//! ├── orchestrator/   # 多 Agent 编排系统
//! │   ├── coordinator # DAG 任务规划 + 并行执行
//! │   ├── map_reduce  # Map-Reduce 并行编排
//! │   └── plan        # 任务规划与依赖解析
//! ├── agent/          # Agent 运行时
//! │   ├── runtime     # Agent 执行体
//! │   ├── registry    # Agent 注册表 (A2A 路由 + 请求-响应)
//! │   ├── protocol    # A2A 通信协议
//! │   └── lanes       # 执行车道 (Lane 系统)
//! └── gateway/        # 元系统层 (CLI/配置/会话管理)
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

// 框架级别的类型别名 — 常用 API 统一入口
pub mod prelude {
    // ── 核心标识 ──
    pub use crate::agent::{AgentId, LaneId, SessionId};

    // ── Agent 构建与事件 ──
    pub use crate::core::agent::{Agent, AgentBuilder, AgentConfig, AgentEvent};

    // ── 核心循环 ──
    pub use crate::core::r#loop::{
        SimpleLoopConfig, SimpleLoopContext, LoopOutcome, LoopEvent, ModelCaps,
    };

    // ── Provider 抽象 ──
    pub use crate::core::provider::{
        Provider, ProviderRequest, StreamEvent, Message, Role, ContentBlock,
    };
    pub use crate::core::ProviderId;

    // ── 缓存 ──
    pub use crate::core::cache::{GlobalCache, CacheStats};

    // ── 上下文管理 ──
    pub use crate::core::context::{ContextUsage, CompactionStrategy};

    // ── 护栏 ──
    pub use crate::core::guardrail::GuardResult;

    // ── 工具系统 ──
    pub use crate::tools::{Tool, ToolResult, ToolContext};
    pub use crate::tools::registry::ToolRegistry;

    // ── 编排系统 ──
    pub use crate::orchestrator::{
        Orchestrator, OrchestratorConfig, OrchestratorMode, OrchestratorResult,
    };
    pub use crate::orchestrator::coordinator::{Coordinator, CoordinatorConfig};
    pub use crate::orchestrator::plan::{TaskPlan, SubTask as PlanSubTask, TaskStatus};
    pub use crate::orchestrator::map_reduce::{MapReduceOrchestrator, SwarmSummary};

    // ── Agent 通信 ──
    pub use crate::agent::registry::AgentRegistry;
    pub use crate::agent::protocol::A2AMessage;
    pub use crate::agent::runtime::AgentMessage;
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
