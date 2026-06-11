//! Sub-Agent 工具 — 创建子 Agent 执行子任务
//!
//! 改进:
//! - 结果缓存: 相同任务不重复执行
//! - 可配置工具集: 通过 `tool_set` 参数选择
//! - 继承父级 prompt_cache 配置

use crate::tools::{Tool, ToolResult, ToolContext};
use serde_json::Value;
use crate::gateway::config::SubAgentModelPolicy;
use std::sync::Mutex;

/// 子 Agent 结果缓存 (task_hash → result)
static SUB_AGENT_CACHE: std::sync::LazyLock<Mutex<std::collections::HashMap<u64, CachedResult>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

const CACHE_MAX_ENTRIES: usize = 64;

#[derive(Clone)]
struct CachedResult {
    content: String,
    is_error: bool,
    created_at: std::time::Instant,
}

pub struct SubAgentTool {
    model_policy: SubAgentModelPolicy,
}

impl Default for SubAgentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SubAgentTool {
    pub fn new() -> Self {
        Self { model_policy: SubAgentModelPolicy::Inherit }
    }

    pub fn with_policy(policy: SubAgentModelPolicy) -> Self {
        Self { model_policy: policy }
    }
}

#[async_trait::async_trait]
impl Tool for SubAgentTool {
    fn name(&self) -> &str { "create_sub_agent" }

    fn description(&self) -> &str {
        "Create a sub-agent to handle a specific task. \
         Sub-agents are isolated workers with their own context. \
         Results are cached — identical tasks return cached results. \
         Use tool_set to control which tools the sub-agent can access: \
         'readonly' (read/glob/grep only), 'full' (read/write/edit/bash/glob/grep), \
         'search' (glob/grep only). Default: 'readonly'."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "The task for the sub-agent to perform" },
                "tool_set": {
                    "type": "string",
                    "description": "Tool set: 'readonly' (default), 'full', 'search'",
                    "enum": ["readonly", "full", "search"]
                },
                "max_turns": { "type": "integer", "description": "Max turns for the sub-agent", "default": 10 },
                "max_tool_calls": { "type": "integer", "description": "Max tool calls", "default": 15 }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let task = input["task"].as_str().unwrap_or("").to_string();
        let tool_set = input["tool_set"].as_str().unwrap_or("readonly");
        let max_turns = input["max_turns"].as_u64().unwrap_or(10);
        let max_tool_calls = input["max_tool_calls"].as_u64().unwrap_or(15);

        // ── 结果缓存检查 ──
        let cache_key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            task.hash(&mut hasher);
            tool_set.hash(&mut hasher);
            hasher.finish()
        };

        if let Ok(cache) = SUB_AGENT_CACHE.lock() {
            if let Some(cached) = cache.get(&cache_key) {
                // 5 分钟内返回缓存
                if cached.created_at.elapsed().as_secs() < 300 {
                    tracing::info!(task = %&task[..task.len().min(50)], "Sub-agent cache hit");
                    return Ok(ToolResult {
                        content: format!("[cached] {}", cached.content),
                        is_error: cached.is_error,
                        metadata: Some(serde_json::json!({"cached": true})),
                    });
                }
            }
        }

        // ── 创建 Provider ──
        let app_config = crate::config::OrionConfig::load_cached();
        let model_config = app_config.active_model();

        let (model, provider) = match &self.model_policy {
            SubAgentModelPolicy::Inherit => {
                let api_key = model_config.api_key.clone()
                    .filter(|k| !k.is_empty())
                    .or_else(|| std::env::var("LLM_API_KEY").ok())
                    .unwrap_or_default();

                let provider: Box<dyn crate::core::provider::Provider> = Box::new(
                    crate::core::providers::openai_compat::OpenAICompatProvider::new(
                        &model_config.endpoint, &api_key, &model_config.name,
                    ),
                );
                (model_config.name.clone(), provider)
            }
            SubAgentModelPolicy::Custom { model, endpoint, api_key } => {
                let key = api_key.clone()
                    .filter(|k| !k.is_empty())
                    .or_else(|| std::env::var("LLM_API_KEY").ok())
                    .unwrap_or_default();
                let provider: Box<dyn crate::core::provider::Provider> = Box::new(
                    crate::core::providers::openai_compat::OpenAICompatProvider::new(
                        endpoint, &key, model,
                    ),
                );
                (model.clone(), provider)
            }
        };

        // ── 注册工具集 (按 tool_set 参数) ──
        let mut tools = crate::tools::registry::ToolRegistry::new();
        match tool_set {
            "full" => {
                tools.register(crate::tools::ReadTool);
                tools.register(crate::tools::WriteTool);
                tools.register(crate::tools::BashTool);
                tools.register(crate::tools::edit::EditTool);
                tools.register(crate::tools::glob_tool::GlobTool);
                tools.register(crate::tools::grep_tool::GrepTool);
            }
            "search" => {
                tools.register(crate::tools::glob_tool::GlobTool);
                tools.register(crate::tools::grep_tool::GrepTool);
            }
            _ => { // "readonly" default
                tools.register(crate::tools::ReadTool);
                tools.register(crate::tools::glob_tool::GlobTool);
                tools.register(crate::tools::grep_tool::GrepTool);
                tools.register(crate::tools::skeleton_tool::SkeletonTool);
            }
        }

        let cache = crate::core::cache::GlobalCache::new(512, 300, 32);

        // ── System Prompt ──
        let env_info = format!("OS: {} | CWD: {}", std::env::consts::OS,
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
        let read_only_hint = if tool_set == "readonly" || tool_set == "search" {
            "You are READ-ONLY — you cannot write or modify files. Focus on analysis and reporting."
        } else {
            "You can read and modify files. Verify changes with build/test commands."
        };
        let sys = format!(
            "You are a sub-agent working on a specific task. \
             {} \
             {} \
             Be concise — return structured findings with file paths and line numbers. \
             When done, summarize what you found or accomplished.",
            read_only_hint, env_info
        );

        let loop_config = crate::core::r#loop::SimpleLoopConfig {
            model,
            system_prompt: sys,
            max_turns,
            max_tool_calls,
            token_budget: 64_000,
            agent_id: format!("sub_{}", &task[..task.len().min(16)].chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()),
            session_id: uuid::Uuid::new_v4().to_string(),
            model_caps: crate::core::r#loop::ModelCaps {
                prompt_cache: model_config.prompt_cache,
                ..Default::default()
            },
            compaction_ratio: app_config.agent.compaction_ratio,
        };

        let result = crate::core::r#loop::run_simple_loop(
            &*provider, &tools, &cache, &loop_config, &task,
            Default::default(),
        ).await;

        let (content, is_error) = match result {
            crate::core::r#loop::LoopOutcome::Completed { message, .. } => {
                (message, false)
            }
            crate::core::r#loop::LoopOutcome::MaxTurnsReached { message, .. } => {
                (format!("[max turns] {}", message), false)
            }
            crate::core::r#loop::LoopOutcome::BudgetExceeded { usage } => {
                (format!("[budget exceeded] {}K tokens used",
                    (usage.input_tokens + usage.output_tokens) / 1000), true)
            }
            crate::core::r#loop::LoopOutcome::Error { message } => {
                (format!("[error] {}", message), true)
            }
            crate::core::r#loop::LoopOutcome::GuardrailDenied { reason } => {
                (format!("[guardrail] {}", reason), true)
            }
        };

        // ── 写入缓存 ──
        if let Ok(mut cache) = SUB_AGENT_CACHE.lock() {
            // 简易 LRU: 超过容量时清除最旧的
            if cache.len() >= CACHE_MAX_ENTRIES {
                if let Some(oldest_key) = cache.iter()
                    .min_by_key(|(_, v)| v.created_at)
                    .map(|(k, _)| *k)
                {
                    cache.remove(&oldest_key);
                }
            }
            cache.insert(cache_key, CachedResult {
                content: content.clone(),
                is_error,
                created_at: std::time::Instant::now(),
            });
        }

        Ok(ToolResult {
            content,
            is_error,
            metadata: Some(serde_json::json!({
                "tool_set": tool_set,
                "cached": false,
            })),
        })
    }
}
