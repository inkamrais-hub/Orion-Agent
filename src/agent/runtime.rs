// Internal: suppress deprecation warnings within this module.
// The deprecated items here reference each other; external consumers
// will still see the deprecation notices.
#![allow(deprecated)]

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::{AgentId, SessionId};
use crate::core::audit::{AuditLogger, AuditEvent};
use crate::core::provider::Provider;
use crate::core::guardrail::GuardrailChain;
use crate::core::context::ContextManager;
use crate::core::cache::GlobalCache;
use crate::tools::registry::ToolRegistry;

// ============================================================
//  积木: Agent Runtime (Agent 执行体)
// ============================================================

/// Agent configuration
///
/// Tightly coupled to [`AgentRuntime`]; retained for API compatibility.
#[deprecated(note = "AgentConfig is coupled to the deprecated AgentRuntime. Use core::agent::Agent instead.")]
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub model: String,
    pub system_prompt: String,
    pub max_turns: u64,
    pub max_tokens_per_turn: u64,
}

/// Agent runtime
///
/// This struct is **not** instantiated in the main execution flow.
/// The primary agent loop lives in `core::agent::Agent`; `AgentRuntime`
/// is retained only for external API consumers and integration tests.
#[deprecated(note = "Use core::agent::Agent instead. AgentRuntime is retained for API compatibility but is not used in the main execution loop.")]
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

// Suppress deprecation warnings inside this file's own impl blocks;
// the methods remain available for external consumers.
#[allow(deprecated)]
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

    /// 启用审计日志
    pub async fn enable_audit(&mut self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let logger = AuditLogger::create(path).await?;
        self.audit = Some(logger);
        Ok(())
    }

    /// 记录审计事件
    pub async fn record_audit(&mut self, event: AuditEvent) {
        if let Some(audit) = &mut self.audit {
            let _ = audit.record(event).await;
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
