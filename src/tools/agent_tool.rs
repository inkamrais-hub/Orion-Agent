//! Sub-Agent 工具 — 创建子 Agent 执行子任务

use crate::tools::{Tool, ToolResult, ToolContext};
use serde_json::Value;
use crate::gateway::config::SubAgentModelPolicy;

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
    fn description(&self) -> &str { "Create a sub-agent to handle a specific task" }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "The task for the sub-agent to perform" },
                "max_turns": { "type": "integer", "description": "Max turns for the sub-agent", "default": 10 },
                "max_tool_calls": { "type": "integer", "description": "Max tool calls per turn", "default": 10 }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let task = input["task"].as_str().unwrap_or("").to_string();
        let max_turns = input["max_turns"].as_u64().unwrap_or(10);
        let max_tool_calls = input["max_tool_calls"].as_u64().unwrap_or(10);

        // Git sandbox isolation: create a temporary branch for the sub-agent
        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".into());
        let sandbox = crate::session::sandbox::GitSandbox::new(&working_dir);
        let sandbox_branch = if sandbox.is_git_repo().await {
            let session_id = uuid::Uuid::new_v4().to_string();
            match sandbox.create_branch(&session_id, "sub_agent").await {
                Ok(branch) => Some(branch),
                Err(e) => {
                    tracing::warn!("Failed to create sandbox branch: {}, running without isolation", e);
                    None
                }
            }
        } else {
            None
        };

        // Create provider based on model policy
        let (model, provider) = match &self.model_policy {
            SubAgentModelPolicy::Inherit => {
                // 从统一配置获取模型信息 (使用缓存避免重复加载)
                let app_config = crate::config::OrionConfig::load_cached();
                let model_config = app_config.active_model();

                let provider: Box<dyn crate::core::provider::Provider> =
                    crate::core::providers::create_provider(&model_config);
                (model_config.name.clone(), provider)
            }
            SubAgentModelPolicy::Custom { model, endpoint, api_key } => {
                // 构建临时 ModelConfig 走统一工厂
                let custom_config = crate::model::ModelConfig {
                    name: model.clone(),
                    api_model: None,
                    provider: "openai".into(), // Custom 策略默认 OpenAI 兼容
                    endpoint: endpoint.clone(),
                    api_key: api_key.clone(),
                    max_tokens: None,
                    modalities: vec!["text".into()],
                    thinking: false,
                    prompt_cache: false,
                    max_input_tokens: None,
                    proxy: None,
                    timeout_secs: 120,
                };
                let provider: Box<dyn crate::core::provider::Provider> =
                    crate::core::providers::create_provider(&custom_config);
                (model.clone(), provider)
            }
        };

        // Register tools
        let mut tools = crate::tools::registry::ToolRegistry::new();
        tools.register(crate::tools::ReadTool);
        tools.register(crate::tools::WriteTool);
        tools.register(crate::tools::BashTool);
        tools.register(crate::tools::edit::EditTool);
        tools.register(crate::tools::glob_tool::GlobTool);
        tools.register(crate::tools::grep_tool::GrepTool);

        let cache = crate::core::cache::GlobalCache::new(512, 300, 32);

        let env_info = format!("OS: {}", std::env::consts::OS);
        let sys = format!(
            "You are a sub-agent. \
            Explore the codebase and report findings with file paths and line numbers. \
            Do NOT modify any files. \
            Be concise - return structured findings, not verbose analysis. {}", env_info
        );

        let loop_config = crate::core::r#loop::SimpleLoopConfig {
            model, system_prompt: sys, max_turns, max_tool_calls,
            token_budget: 64_000, agent_id: "sub_agent".into(),
            session_id: uuid::Uuid::new_v4().to_string(),
            model_caps: crate::core::r#loop::ModelCaps::default(),
            exec_mode: crate::core::exec_mode::ExecMode::default(),
            working_dir: None,
        };
        let result = crate::core::r#loop::run_simple_loop(
            &*provider, &tools, &cache, &loop_config, &task,
            Default::default(),
        ).await;

        let tool_result = match result {
            crate::core::r#loop::LoopOutcome::Completed { message, .. } => {
                Ok(ToolResult { content: message, is_error: false, metadata: None })
            }
            crate::core::r#loop::LoopOutcome::MaxTurnsReached { message, .. } => {
                Ok(ToolResult { content: format!("[Sub-agent max turns] {}", message), is_error: false, metadata: None })
            }
            crate::core::r#loop::LoopOutcome::Error { message } => {
                Ok(ToolResult { content: format!("[Sub-agent error] {}", message), is_error: true, metadata: None })
            }
            _ => Ok(ToolResult { content: "Sub-agent did not complete".into(), is_error: true, metadata: None }),
        };

        // Merge sandbox branch back and clean up
        if let Some(branch) = sandbox_branch {
            match sandbox.merge_branch(&branch).await {
                Ok(crate::session::sandbox::MergeResult::Success { .. }) => {
                    tracing::info!("Sandbox branch merged successfully");
                }
                Ok(crate::session::sandbox::MergeResult::Conflict { files, .. }) => {
                    tracing::warn!("Sandbox merge conflicts in: {:?}", files);
                }
                Err(e) => {
                    tracing::warn!("Sandbox merge failed: {}", e);
                }
            }
            let _ = sandbox.cleanup_branch(&branch).await;
        }

        tool_result
    }
}
