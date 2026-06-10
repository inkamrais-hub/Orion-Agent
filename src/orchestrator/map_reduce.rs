//! MapReduce 编排器 — 上下文分叉与合并
//!
//! 核心思想:
//!   主 Agent 将任务分叉给多个 Sub-Agent 并行执行，
//!   每个 Sub-Agent 在独立的上下文中工作。
//!   完成后，由 Reducer Agent 汇总所有子结果，生成结构化摘要注入主上下文。

use std::sync::Arc;

use crate::core::agent::Agent;
use crate::core::provider::Provider;
use crate::tools::registry::ToolRegistry;

/// 粗略估算文本 token 数量（约每 4 字节 = 1 token）
fn estimate_text_tokens(text: &str) -> u64 {
    (text.len() as u64) / 4
}

/// MapReduce 编排器
pub struct MapReduceOrchestrator {
    provider: Arc<dyn Provider>,
    /// 工具工厂闭包：为每个子 Agent 创建独立的 ToolRegistry 实例
    tools_factory: Arc<dyn Fn() -> ToolRegistry + Send + Sync>,
    max_parallel: usize,
    model: String,
}

/// 子任务定义
pub struct SubTask {
    pub id: String,
    pub description: String,
    pub system_prompt: Option<String>,
}

impl Clone for SubTask {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            description: self.description.clone(),
            system_prompt: self.system_prompt.clone(),
        }
    }
}

/// 子任务执行结果
#[derive(Debug, Clone)]
pub struct SubTaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub tokens_used: u64,
}

/// 汇总结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwarmSummary {
    pub completed_tasks: Vec<String>,
    pub modified_files: Vec<String>,
    pub failures: Vec<String>,
    pub total_tokens: u64,
    pub summary: String,
}

impl MapReduceOrchestrator {
    /// 创建编排器，使用工具工厂闭包为每个子 Agent 生成独立的 ToolRegistry
    pub fn new(
        provider: Arc<dyn Provider>,
        tools_factory: impl Fn() -> ToolRegistry + Send + Sync + 'static,
    ) -> Self {
        Self {
            provider,
            tools_factory: Arc::new(tools_factory),
            max_parallel: 6,
            model: "deepseek-chat".into(),
        }
    }

    /// 创建编排器，子 Agent 共享同一份工具注册表（通过 Clone）
    pub fn with_registry(provider: Arc<dyn Provider>, registry: ToolRegistry) -> Self {
        Self::new(provider, move || registry.clone())
    }

    /// 创建编排器，子 Agent 使用空工具（纯文本生成任务）
    pub fn without_tools(provider: Arc<dyn Provider>) -> Self {
        Self::new(provider, ToolRegistry::new)
    }

    pub fn max_parallel(mut self, n: usize) -> Self {
        self.max_parallel = n;
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// 执行 MapReduce 编排
    pub async fn execute(
        &self,
        subtasks: Vec<SubTask>,
        reduce_prompt: &str,
    ) -> crate::Result<SwarmSummary> {
        // Map 阶段：并行执行所有子任务
        let results = self.map_phase(&subtasks).await?;

        // Reduce 阶段：汇总结果
        let summary = self.reduce_phase(&results, reduce_prompt).await?;

        Ok(summary)
    }

    /// Map 阶段：并行执行子任务
    async fn map_phase(&self, subtasks: &[SubTask]) -> crate::Result<Vec<SubTaskResult>> {
        use futures::future::join_all;

        let mut handles = Vec::new();

        // 按 max_parallel 分批，避免一次 spawn 过多任务
        for chunk in subtasks.chunks(self.max_parallel) {
            for task in chunk {
                let provider = self.provider.clone();
                let tools_factory = self.tools_factory.clone();
                let task = task.clone();
                let model = self.model.clone();

                let handle = tokio::spawn(async move {
                    let system_prompt = task.system_prompt.clone().unwrap_or_else(|| {
                        "You are a focused sub-agent. Complete your assigned task efficiently. \
                         Output a brief JSON summary of what you did when finished."
                            .to_string()
                    });

                    let tools = tools_factory();

                    let agent = Agent::builder()
                        .name(&task.id)
                        .model(&model)
                        .system_prompt(&system_prompt)
                        .provider(provider)
                        .tools(tools)
                        .max_turns(10)
                        .max_tool_calls(20)
                        .build();

                    match agent {
                        Ok(agent) => match agent.chat(&task.description).await {
                            Ok(output) => {
                                let tokens = estimate_text_tokens(&task.description)
                                    + estimate_text_tokens(&output);
                                SubTaskResult {
                                    task_id: task.id,
                                    success: true,
                                    output,
                                    tokens_used: tokens,
                                }
                            }
                            Err(e) => {
                                let tokens = estimate_text_tokens(&task.description);
                                SubTaskResult {
                                    task_id: task.id,
                                    success: false,
                                    output: format!("Error: {}", e),
                                    tokens_used: tokens,
                                }
                            }
                        },
                        Err(e) => SubTaskResult {
                            task_id: task.id,
                            success: false,
                            output: format!("Build error: {}", e),
                            tokens_used: 0,
                        },
                    }
                });

                handles.push(handle);
            }
        }

        // 等待所有子任务完成
        let all_results = join_all(handles).await;

        let results: Vec<SubTaskResult> = all_results
            .into_iter()
            .filter_map(|r| r.ok())
            .collect();

        tracing::info!(
            completed = results.iter().filter(|r| r.success).count(),
            failed = results.iter().filter(|r| !r.success).count(),
            "Map phase completed"
        );

        Ok(results)
    }

    /// Reduce 阶段：汇总所有子任务结果
    async fn reduce_phase(
        &self,
        results: &[SubTaskResult],
        reduce_prompt: &str,
    ) -> crate::Result<SwarmSummary> {
        // 构造汇总输入
        let results_json = serde_json::to_string_pretty(
            &results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "task_id": r.task_id,
                        "success": r.success,
                        "output": r.output,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();

        let prompt = format!(
            "{}\n\nSub-task results:\n{}\n\n\
             Output a JSON object with: completed_tasks, modified_files, failures, summary",
            reduce_prompt, results_json
        );

        let agent = Agent::builder()
            .name("reducer")
            .model(&self.model)
            .system_prompt(
                "You are a result summarizer. Read all sub-task results and produce a concise JSON summary. \
                 Output ONLY valid JSON.",
            )
            .provider(self.provider.clone())
            .tools(ToolRegistry::new())
            .max_turns(3)
            .build()?;

        let output = agent.chat(&prompt).await?;

        // 统计 Map 阶段所有子任务的 token 用量，加上 Reduce 阶段的估算
        let map_tokens: u64 = results.iter().map(|r| r.tokens_used).sum();
        let reduce_tokens = estimate_text_tokens(&prompt) + estimate_text_tokens(&output);
        let total_tokens = map_tokens + reduce_tokens;

        // 尝试解析 JSON，失败则构造 fallback 摘要
        let mut summary =
            serde_json::from_str::<SwarmSummary>(&output).unwrap_or_else(|_| SwarmSummary {
                completed_tasks: results
                    .iter()
                    .filter(|r| r.success)
                    .map(|r| r.task_id.clone())
                    .collect(),
                modified_files: vec![],
                failures: results
                    .iter()
                    .filter(|r| !r.success)
                    .map(|r| format!("{}: {}", r.task_id, r.output))
                    .collect(),
                total_tokens: 0,
                summary: output,
            });

        summary.total_tokens = total_tokens;

        Ok(summary)
    }
}
