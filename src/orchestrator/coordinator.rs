//! Coordinator — 编排器协调者

use super::worker::{Worker, WorkerConfig};
use crate::core::provider::{Provider, ProviderRequest, Message, Role, ContentBlock};
use crate::orchestrator::plan::{TaskPlan, TaskStatus, PLANNING_SYSTEM_PROMPT};
use crate::tools::registry::ToolRegistry;
use crate::core::cache::GlobalCache;
use crate::agent::registry::AgentRegistry;
use std::sync::Arc;

/// 协调者配置
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub worker_model: Option<String>,
    pub max_rounds: usize,
    /// 主执行循环最大迭代次数，防止死循环（默认 20）
    pub max_iterations: usize,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            worker_model: None,
            max_rounds: 5,
            max_iterations: 20,
        }
    }
}

/// Coordinator
pub struct Coordinator {
    pub config: CoordinatorConfig,
    pub provider: Arc<dyn Provider>,
    pub cache: GlobalCache,
    pub registry: Arc<AgentRegistry>,
    /// 共享工具注册表（clone 给每个 Worker）
    pub tools: ToolRegistry,
}

impl Coordinator {
    pub fn new(
        config: CoordinatorConfig,
        provider: Arc<dyn Provider>,
        cache: GlobalCache,
        registry: Arc<AgentRegistry>,
        tools: ToolRegistry,
    ) -> Self {
        Self { config, provider, cache, registry, tools }
    }

    /// 执行任务：先用 LLM 生成 TaskPlan，再按 DAG 依赖逐个执行子任务
    pub async fn execute(&self, task_description: &str) -> crate::Result<String> {
        // 1. 调用 LLM 生成 TaskPlan
        let plan_json = self.call_llm_for_plan(task_description).await?;
        let mut plan = TaskPlan::from_json(&plan_json)?;

        tracing::info!(goal = %plan.goal, tasks = plan.tasks.len(), "Task plan created");

        // 2. 循环执行可执行任务
        let max_iterations = self.config.max_iterations; // 防止死循环
        let mut iterations = 0;
        while !plan.is_complete() && iterations < max_iterations {
            iterations += 1;

            let Some(subtask) = plan.next_executable().cloned() else {
                tracing::warn!("No executable tasks found, plan may be stuck");
                break;
            };

            tracing::info!(task_id = %subtask.id, desc = %subtask.description, "Executing subtask");

            let worker = self.create_worker(&subtask.id).await;
            let context = plan.completed_summary();

            match worker.execute(&subtask.description, &context).await {
                Ok(result) => {
                    tracing::info!(task_id = %subtask.id, "Subtask completed");
                    plan.mark_completed(&subtask.id, result);
                }
                Err(e) => {
                    // 首次失败：重试一次
                    tracing::warn!(task_id = %subtask.id, error = %e, "Subtask failed, retrying once");

                    let retry_worker = self.create_worker(&subtask.id).await;
                    let retry_context = plan.completed_summary();

                    match retry_worker.execute(&subtask.description, &retry_context).await {
                        Ok(result) => {
                            tracing::info!(task_id = %subtask.id, "Subtask succeeded on retry");
                            plan.mark_completed(&subtask.id, result);
                        }
                        Err(retry_err) => {
                            tracing::warn!(task_id = %subtask.id, error = %retry_err, "Subtask failed after retry");
                            plan.mark_failed(&subtask.id, retry_err.to_string());
                        }
                    }
                }
            }
        }

        // 3. 检查是否全部完成（DAG 卡死检测）
        if !plan.is_complete() {
            let failed_tasks: Vec<String> = plan.tasks.iter()
                .filter(|t| t.status != TaskStatus::Completed)
                .map(|t| format!("{}: {:?} - {}", t.id, t.status, t.result.as_deref().unwrap_or("no result")))
                .collect();
            return Err(crate::Error::Agent(format!(
                "DAG execution incomplete. {} tasks not completed: {}",
                failed_tasks.len(),
                failed_tasks.join(", ")
            )));
        }

        // 4. 汇总结果
        let summary = plan.completed_summary();
        if summary.is_empty() {
            Ok("No tasks completed successfully.".to_string())
        } else {
            Ok(summary)
        }
    }

    /// 调用 LLM 生成任务计划 JSON
    async fn call_llm_for_plan(&self, task_description: &str) -> crate::Result<String> {
        let model = self.config.worker_model.clone()
            .unwrap_or_else(|| "deepseek-chat".to_string());

        let req = ProviderRequest {
            model,
            messages: vec![Message::new(Role::User, vec![ContentBlock::Text {
                text: format!("Break this task into subtasks:\n\n{}", task_description),
            }])],
            system_prompt: Some(PLANNING_SYSTEM_PROMPT.to_string()),
            max_tokens: Some(2048),
            temperature: Some(0.3),
            stream: false,
            tools: None,
            thinking: None,
            reasoning_effort: None,
            enable_prompt_cache: None,
            cache_key: None,
        };

        let resp = self.provider.complete(req).await?;
        let text = resp.message.content.iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    /// 创建 Worker，继承主 Agent 的全部工具（包括 MCP 和自定义工具）
    async fn create_worker(&self, id: &str) -> Worker {
        // 从统一配置获取模型信息
        let app_config = crate::config::OrionConfig::load();
        let model_config = app_config.active_model();

        let config = WorkerConfig {
            id: format!("worker_{}", id),
            model: self.config.worker_model.clone().unwrap_or_else(|| model_config.name.clone()),
            max_turns: 10,
            max_tool_calls: 30,
        };

        let api_key = model_config.api_key.clone()
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("LLM_API_KEY").ok())
            .unwrap_or_default();

        let provider: Box<dyn Provider> = Box::new(
            crate::core::providers::openai_compat::OpenAICompatProvider::new(
                &model_config.endpoint,
                &api_key,
                &config.model,
            ),
        );

        // Clone 共享工具注册表，子 Worker 自动继承全部工具
        let tools = self.tools.clone();

        Worker::new(config, provider, tools, self.cache.clone(), Some(self.registry.clone()))
    }
}
