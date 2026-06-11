use std::sync::Arc;
use tokio::sync::mpsc;

use crate::agent::registry::AgentRegistry;
use crate::core::cache::GlobalCache;
use crate::core::execpolicy::ExecPolicy;
use crate::core::hooks::HookEngine;
use crate::core::provider::Provider;
use crate::core::r#loop::{
    EventCallback, LoopEvent, LoopOutcome, ModelCaps, SimpleLoopConfig, SimpleLoopContext,
    run_simple_loop,
};
use crate::tools::Tool;
use crate::tools::registry::ToolRegistry;

/// reasoning_effort 到 max_output_tokens 的映射
fn resolve_max_output_tokens(reasoning_effort: &str) -> u32 {
    match reasoning_effort {
        "low" => 2048,
        "medium" => 4096,
        "high" => 8192,
        "max" => 16384,
        "xhigh" => 32768,
        _ => 4096,
    }
}

// ============================================================
//  AgentConfig — 面向用户的简洁配置
// ============================================================

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    pub max_turns: u64,
    pub max_tool_calls: u64,
    pub token_budget: u64,
    pub thinking: bool,
    pub reasoning_effort: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "agent".into(),
            model: "deepseek-chat".into(),
            system_prompt: String::new(),
            max_turns: 20,
            max_tool_calls: 30,
            token_budget: 128_000,
            thinking: false,
            reasoning_effort: "medium".into(),
        }
    }
}

// ============================================================
//  AgentEvent — 流式事件
// ============================================================

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thinking(String),
    Text(String),
    ToolStart {
        name: String,
        input: serde_json::Value,
    },
    ToolEnd {
        name: String,
        result: String,
        success: bool,
        duration_ms: u64,
    },
    TurnComplete {
        turn: u64,
    },
    Done {
        message: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    Error(String),
}

// ============================================================
//  AgentBuilder
// ============================================================

pub struct AgentBuilder {
    config: AgentConfig,
    provider: Option<Arc<dyn Provider>>,
    tools: ToolRegistry,
    cache: Option<GlobalCache>,
    hook_engine: Option<HookEngine>,
    exec_policy: Option<ExecPolicy>,
    registry: Option<Arc<AgentRegistry>>,
    lazy_mode: bool,
}

impl AgentBuilder {
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
            provider: None,
            tools: ToolRegistry::new(),
            cache: None,
            hook_engine: None,
            exec_policy: None,
            registry: None,
            lazy_mode: false,
        }
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.config.name = name.into();
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = prompt.into();
        self
    }

    pub fn max_turns(mut self, n: u64) -> Self {
        self.config.max_turns = n;
        self
    }

    pub fn max_tool_calls(mut self, n: u64) -> Self {
        self.config.max_tool_calls = n;
        self
    }

    pub fn token_budget(mut self, n: u64) -> Self {
        self.config.token_budget = n;
        self
    }

    pub fn thinking(mut self, enabled: bool) -> Self {
        self.config.thinking = enabled;
        self
    }

    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.config.reasoning_effort = effort.into();
        self
    }

    pub fn provider(mut self, provider: Arc<dyn Provider>) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn add_tool(mut self, tool: impl Tool + 'static) -> Self {
        self.tools.register(tool);
        self
    }

    pub fn cache(mut self, cache: GlobalCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn hook_engine(mut self, engine: HookEngine) -> Self {
        self.hook_engine = Some(engine);
        self
    }

    pub fn exec_policy(mut self, policy: ExecPolicy) -> Self {
        self.exec_policy = Some(policy);
        self
    }

    pub fn registry(mut self, registry: Arc<AgentRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// 启用延迟装载模式 — LLM 初始只看到元工具 (list_categories + load_tool)，
    /// 需要时按名称加载具体工具 Schema，减少每次对话的 Token 消耗。
    pub fn lazy_tools(mut self) -> Self {
        self.lazy_mode = true;
        self
    }

    pub fn build(self) -> crate::Result<Agent> {
        let provider = self.provider.ok_or_else(|| {
            crate::Error::Config("Provider is required. Call .provider() before .build()".into())
        })?;

        let tools = if self.lazy_mode {
            // 延迟装载模式：使用 Arc::new_cyclic 打破 ToolRegistry ↔ 元工具 循环引用
            let mut tools = self.tools;
            tools.enable_lazy_mode();
            Arc::new_cyclic(|weak| {
                crate::tools::register_meta_tools(&mut tools, weak.clone());
                tools
            })
        } else {
            Arc::new(self.tools)
        };

        Ok(Agent {
            config: self.config,
            provider,
            tools,
            cache: self.cache.unwrap_or_else(|| GlobalCache::new(1024, 300, 64)),
            hook_engine: self.hook_engine.map(|e| Arc::new(tokio::sync::Mutex::new(e))),
            exec_policy: self.exec_policy.map(Arc::new),
            registry: self.registry,
        })
    }
}

// ============================================================
//  Agent
// ============================================================

pub struct Agent {
    config: AgentConfig,
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    cache: GlobalCache,
    hook_engine: Option<Arc<tokio::sync::Mutex<HookEngine>>>,
    exec_policy: Option<Arc<ExecPolicy>>,
    registry: Option<Arc<AgentRegistry>>,
}

impl Agent {
    /// 创建 AgentBuilder
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    /// 获取 provider 引用
    pub fn provider(&self) -> &dyn Provider {
        &*self.provider
    }

    /// 获取工具注册表引用
    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    /// 获取配置引用
    pub fn config(&self) -> &AgentConfig {
        &self.config
    }

    /// 获取缓存引用
    pub fn cache(&self) -> &GlobalCache {
        &self.cache
    }

    /// 获取 Hook 引擎引用
    pub fn hook_engine(&self) -> Option<Arc<tokio::sync::Mutex<HookEngine>>> {
        self.hook_engine.clone()
    }

    /// 获取执行策略引用
    pub fn exec_policy(&self) -> Option<Arc<ExecPolicy>> {
        self.exec_policy.clone()
    }

    /// 获取注册表引用
    pub fn registry(&self) -> Option<Arc<AgentRegistry>> {
        self.registry.clone()
    }


    /// 从 AgentConfig 构建 SimpleLoopConfig
    fn build_loop_config(&self, session_id: Option<String>) -> SimpleLoopConfig {
        SimpleLoopConfig {
            model: self.config.model.clone(),
            system_prompt: self.config.system_prompt.clone(),
            max_turns: self.config.max_turns,
            max_tool_calls: self.config.max_tool_calls,
            token_budget: self.config.token_budget,
            agent_id: self.config.name.clone(),
            session_id: session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            model_caps: ModelCaps {
                thinking: self.config.thinking,
                prompt_cache: false,
                max_output_tokens: resolve_max_output_tokens(&self.config.reasoning_effort),
            },
            compaction_ratio: crate::config::OrionConfig::load_cached().agent.compaction_ratio,
        }
    }

    /// 单次对话 — 阻塞等待完整结果
    pub async fn chat(&self, input: &str) -> crate::Result<String> {
        let loop_config = self.build_loop_config(None);

        let ctx = SimpleLoopContext {
            event_callback: None,
            registry: self.registry.clone(),
            hook_engine: self.hook_engine.clone(),
            exec_policy: self.exec_policy.clone(),
            guardrails: None,
            rollout: None,
            goal_manager: None,
            images: None,
        };

        let outcome = run_simple_loop(
            &*self.provider,
            &self.tools,
            &self.cache,
            &loop_config,
            input,
            ctx,
        )
        .await;

        match outcome {
            LoopOutcome::Completed { message, .. } => Ok(message),
            LoopOutcome::MaxTurnsReached { message, .. } => Ok(message),
            LoopOutcome::BudgetExceeded { .. } => {
                Err(crate::Error::Agent("Token budget exceeded".into()))
            }
            LoopOutcome::GuardrailDenied { reason } => Err(crate::Error::Guardrail(reason)),
            LoopOutcome::Error { message } => Err(crate::Error::Agent(message)),
        }
    }

    /// 流式对话 — 返回事件接收器
    pub fn chat_stream(
        &self,
        input: &str,
        session_id: Option<String>,
    ) -> crate::Result<mpsc::UnboundedReceiver<AgentEvent>> {
        let (tx, rx) = mpsc::unbounded_channel();

        let config = self.config.clone();
        let provider = Arc::clone(&self.provider);
        let tools = Arc::clone(&self.tools);
        let cache = self.cache.clone();
        let hook_engine = self.hook_engine.clone();
        let exec_policy = self.exec_policy.clone();
        let registry = self.registry.clone();
        let input = input.to_string();
        let session_id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        tokio::spawn(async move {
            let AgentConfig {
                name,
                model,
                system_prompt,
                max_turns,
                max_tool_calls,
                token_budget,
                thinking,
                reasoning_effort,
                ..
            } = config;

            let loop_config = SimpleLoopConfig {
                model,
                system_prompt,
                max_turns,
                max_tool_calls,
                token_budget,
                agent_id: name,
                session_id,
                model_caps: ModelCaps {
                    thinking,
                    prompt_cache: false,
                    max_output_tokens: resolve_max_output_tokens(&reasoning_effort),
                },
                compaction_ratio: crate::config::OrionConfig::load_cached().agent.compaction_ratio,
            };

            let event_tx = tx.clone();
            let event_callback: EventCallback = Arc::new(move |event: &LoopEvent| {
                let agent_event = match event {
                    LoopEvent::ThinkingDelta { text } => AgentEvent::Thinking(text.clone()),
                    LoopEvent::TextDelta(text) => AgentEvent::Text(text.clone()),
                    LoopEvent::ToolStart {
                        tool_name, input, ..
                    } => AgentEvent::ToolStart {
                        name: tool_name.clone(),
                        input: input.clone(),
                    },
                    LoopEvent::ToolEnd {
                        tool_name,
                        result,
                        is_error,
                        duration_ms,
                        ..
                    } => AgentEvent::ToolEnd {
                        name: tool_name.clone(),
                        result: result.clone(),
                        success: !is_error,
                        duration_ms: *duration_ms,
                    },
                    LoopEvent::TurnComplete { turn } => {
                        AgentEvent::TurnComplete { turn: *turn }
                    }
                    LoopEvent::Error(msg) => AgentEvent::Error(msg.clone()),
                };
                let _ = event_tx.send(agent_event);
            });

            let ctx = SimpleLoopContext {
                event_callback: Some(event_callback),
                registry,
                hook_engine,
                exec_policy: exec_policy.clone(),
                guardrails: None,
                rollout: None,
                goal_manager: None,
                images: None,
            };

            let outcome = run_simple_loop(
                &*provider,
                &tools,
                &cache,
                &loop_config,
                &input,
                ctx,
            )
            .await;

            let agent_event = match outcome {
                LoopOutcome::Completed { message, usage } => AgentEvent::Done {
                    message,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                },
                LoopOutcome::MaxTurnsReached { message, usage } => AgentEvent::Done {
                    message,
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                },
                LoopOutcome::BudgetExceeded { usage } => AgentEvent::Error(format!(
                    "Token budget exceeded (in: {}, out: {})",
                    usage.input_tokens, usage.output_tokens
                )),
                LoopOutcome::GuardrailDenied { reason } => AgentEvent::Error(reason),
                LoopOutcome::Error { message } => AgentEvent::Error(message),
            };

            let _ = tx.send(agent_event);
        });

        Ok(rx)
    }
}
