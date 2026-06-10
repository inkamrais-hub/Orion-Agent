//! Worker — 编排器的工作节点

use crate::core::provider::Provider;
use crate::tools::registry::ToolRegistry;
use crate::core::cache::GlobalCache;
use std::sync::Arc;
use crate::agent::registry::AgentRegistry;
use crate::core::r#loop::EventCallback;

/// Worker 配置
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub id: String,
    pub model: String,
    pub max_turns: u64,
    pub max_tool_calls: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            id: "worker_0".into(),
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
            max_turns: 20,
            max_tool_calls: 30,
        }
    }
}

/// Worker 实例
pub struct Worker {
    pub config: WorkerConfig,
    pub provider: Box<dyn Provider>,
    pub tools: ToolRegistry,
    pub cache: GlobalCache,
    pub registry: Option<Arc<AgentRegistry>>,
}

impl Worker {
    pub fn new(
        config: WorkerConfig,
        provider: Box<dyn Provider>,
        tools: ToolRegistry,
        cache: GlobalCache,
        registry: Option<Arc<AgentRegistry>>,
    ) -> Self {
        Self { config, provider, tools, cache, registry }
    }

    /// 执行任务
    pub async fn execute(&self, user_input: &str, system_prompt: &str) -> crate::Result<String> {
        let worker_id = self.config.id.clone();
        let thinking_buf = std::sync::Mutex::new(String::new());
        let in_thinking = std::sync::Mutex::new(false);

        let event_cb: EventCallback = Arc::new(move |event| {
            match event {
                crate::core::r#loop::LoopEvent::ThinkingDelta { text } => {
                    if text.is_empty() {
                        *in_thinking.lock().unwrap() = true;
                        thinking_buf.lock().unwrap().clear();
                    } else {
                        thinking_buf.lock().unwrap().push_str(text);
                    }
                }
                crate::core::r#loop::LoopEvent::TextDelta(_) => {
                    flush_worker_thinking(&thinking_buf, &worker_id);
                }
                _ => {}
            }
        });

        let loop_config = crate::core::r#loop::SimpleLoopConfig {
            model: self.config.model.clone(),
            system_prompt: system_prompt.to_string(),
            max_turns: self.config.max_turns,
            max_tool_calls: self.config.max_tool_calls,
            token_budget: 128_000,
            agent_id: self.config.id.clone(),
            session_id: uuid::Uuid::new_v4().to_string(),
            model_caps: crate::core::r#loop::ModelCaps::default(),
        };
        let result = crate::core::r#loop::run_simple_loop(
            &*self.provider,
            &self.tools,
            &self.cache,
            &loop_config,
            user_input,
            crate::core::r#loop::SimpleLoopContext {
                event_callback: Some(event_cb),
                registry: self.registry.clone(),
                ..Default::default()
            },
        )
        .await;

        match result {
            crate::core::r#loop::LoopOutcome::Completed { message, .. } => Ok(message),
            crate::core::r#loop::LoopOutcome::MaxTurnsReached { message, .. } => Ok(message),
            crate::core::r#loop::LoopOutcome::Error { message } => Err(crate::Error::Agent(message)),
            _ => Err(crate::Error::Agent("Worker did not complete".into())),
        }
    }
}

fn flush_worker_thinking(buf: &std::sync::Mutex<String>, worker_id: &str) {
    let content = buf.lock().unwrap().clone();
    if !content.is_empty() {
        tracing::debug!(worker = worker_id, thinking = %content, "Worker thinking");
    }
}
