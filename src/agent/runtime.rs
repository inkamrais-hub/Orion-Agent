use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::{AgentId, SessionId};
use crate::audit::{AuditLogger, AuditEvent};
use crate::core::provider::Provider;
use crate::core::guardrail::GuardrailChain;
use crate::core::context::ContextManager;
use crate::core::cache::GlobalCache;
use crate::tools::registry::ToolRegistry;

// ============================================================
//  积木: Agent Runtime (Agent 执行体)
// ============================================================

/// Agent 配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub model: String,
    pub system_prompt: String,
    pub max_turns: u64,
    pub max_tokens_per_turn: u64,
}

/// Agent 运行时
pub struct AgentRuntime {
    pub config: AgentConfig,
    pub provider: Box<dyn Provider>,
    pub tools: ToolRegistry,
    pub guardrails: GuardrailChain,
    pub context_manager: ContextManager,
    pub cache: GlobalCache,
    /// 可选审计日志 (None = 不记录)
    pub audit: Option<AuditLogger>,
    /// Agent 注册表 (A2A 通信)
    pub registry: Option<std::sync::Arc<crate::agent::registry::AgentRegistry>>,
}

impl AgentRuntime {
    pub fn new(
        config: AgentConfig,
        provider: Box<dyn Provider>,
        tools: ToolRegistry,
        guardrails: GuardrailChain,
    ) -> Self {
        let tokens_per_turn = config.max_tokens_per_turn;
        Self {
            config,
            provider,
            tools,
            guardrails,
            context_manager: ContextManager::new(tokens_per_turn),
            cache: GlobalCache::new(1024, 300, 64),
            audit: None,
            registry: None,
        }
    }

    /// 启用审计日志 (新 API: 同步缓冲写入)
    pub fn enable_audit(&mut self) {
        self.audit = Some(AuditLogger::new());
    }

    /// 记录审计事件
    pub fn record_audit(&mut self, event: AuditEvent) {
        if let Some(audit) = &mut self.audit {
            audit.log(event, "agent_runtime");
        }
    }
}

// ============================================================
//  积木: Agent 间消息
//  参考: OpenClaw ACP 协议风格
// ============================================================

/// Agent 间消息
#[derive(Debug, Clone)]
pub struct AgentMessage {
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
    pub tool_call: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Agent 通信通道
pub type AgentChannel = mpsc::UnboundedSender<AgentMessage>;

/// 可接收消息的 Agent
#[async_trait]
pub trait MessageHandler: Send {
    async fn handle_message(&mut self, msg: AgentMessage) -> crate::Result<()>;
}
