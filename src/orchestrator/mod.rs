//! Agent 编排系统
//!
//! Coordinator/Worker 模式:
//!   用户请求 → Coordinator 拆解任务 → Workers 执行 → 结果汇总
//!
//! 模式:
//!   - Sequential: 顺序执行
//!   - Parallel: 并行执行 (join_all)
//!   - Collaborative: 协商式 (Worker 间可通信)

pub mod coordinator;
pub mod map_reduce;
pub mod plan;
pub mod worker;

use serde::{Deserialize, Serialize};

use self::coordinator::{Coordinator, CoordinatorConfig};
use self::map_reduce::{MapReduceOrchestrator, SubTask as MRSubTask};
use crate::agent::registry::AgentRegistry;
use crate::core::cache::GlobalCache;
use crate::core::provider::{ContentBlock, Message, Provider, ProviderRequest, Role};
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;
use std::time::Instant;

/// 编排模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[derive(Default)]
pub enum OrchestratorMode {
    #[default]
    Sequential,
    Parallel,
    Collaborative,
}

impl std::fmt::Display for OrchestratorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequential => write!(f, "sequential"),
            Self::Parallel => write!(f, "parallel"),
            Self::Collaborative => write!(f, "collaborative"),
        }
    }
}

impl std::str::FromStr for OrchestratorMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" => Ok(Self::Sequential),
            "parallel" => Ok(Self::Parallel),
            "collaborative" => Ok(Self::Collaborative),
            _ => Err(format!("Unknown mode: {}", s)),
        }
    }
}


/// 编排器配置
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub coordinator_model: String,
    pub worker_model: String,
    pub max_workers: usize,
    pub max_rounds_per_task: usize,
    pub mode: OrchestratorMode,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            coordinator_model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
            worker_model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into()),
            max_workers: 3,
            max_rounds_per_task: 10,
            mode: OrchestratorMode::Sequential,
        }
    }
}

/// 编排结果
#[derive(Debug, Clone)]
pub struct OrchestratorResult {
    pub final_answer: String,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_tokens: u64,
    pub duration_ms: u64,
}

// ============================================================
//  Orchestrator — 顶层调度结构
//  根据 OrchestratorMode 分发到 Coordinator 或 MapReduce 编排器
// ============================================================

/// 顶层编排器，根据配置的模式分发任务到不同的编排策略
pub struct Orchestrator {
    pub config: OrchestratorConfig,
    pub provider: Arc<dyn Provider>,
    pub cache: GlobalCache,
    pub registry: Arc<AgentRegistry>,
    pub tools: ToolRegistry,
}

impl Orchestrator {
    /// 创建新的 Orchestrator 实例
    pub fn new(
        config: OrchestratorConfig,
        provider: Arc<dyn Provider>,
        cache: GlobalCache,
        registry: Arc<AgentRegistry>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            config,
            provider,
            cache,
            registry,
            tools,
        }
    }

    /// 执行任务：根据 config.mode 分发到对应的编排策略
    ///
    /// - Sequential / Parallel → Coordinator (DAG-based execution)
    /// - Collaborative → MapReduceOrchestrator (parallel sub-agents + reduce)
    pub async fn run(&self, task: &str) -> crate::Result<OrchestratorResult> {
        let start = Instant::now();

        let result = match self.config.mode {
            OrchestratorMode::Sequential | OrchestratorMode::Parallel => {
                self.run_coordinator(task).await
            }
            OrchestratorMode::Collaborative => {
                self.run_collaborative(task).await
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(mut r) => {
                r.duration_ms = duration_ms;
                Ok(r)
            }
            Err(e) => {
                // Wrap the error with timing info so callers still get a result
                Err(crate::Error::Agent(format!(
                    "Orchestrator failed after {}ms: {}",
                    duration_ms, e
                )))
            }
        }
    }

    // --------------------------------------------------------
    //  Sequential / Parallel — delegate to Coordinator
    // --------------------------------------------------------

    async fn run_coordinator(&self, task: &str) -> crate::Result<OrchestratorResult> {
        let coord_config = CoordinatorConfig {
            worker_model: Some(self.config.coordinator_model.clone()),
            max_rounds: self.config.max_rounds_per_task,
        };

        let coordinator = Coordinator::new(
            coord_config,
            self.provider.clone(),
            self.cache.clone(),
            self.registry.clone(),
            self.tools.clone(),
        );

        match coordinator.execute(task).await {
            Ok(answer) => {
                // Count completed tasks from the summary lines: "[task_id]: description: result"
                let tasks_completed = answer
                    .lines()
                    .filter(|line| line.starts_with('['))
                    .count();

                Ok(OrchestratorResult {
                    final_answer: answer,
                    tasks_completed,
                    tasks_failed: 0,
                    total_tokens: 0,
                    duration_ms: 0,
                })
            }
            Err(e) => {
                // Coordinator returned an error (e.g. DAG stuck) — report as failure
                Ok(OrchestratorResult {
                    final_answer: format!("Coordinator error: {}", e),
                    tasks_completed: 0,
                    tasks_failed: 1,
                    total_tokens: 0,
                    duration_ms: 0,
                })
            }
        }
    }

    // --------------------------------------------------------
    //  Collaborative — MapReduceOrchestrator
    // --------------------------------------------------------

    async fn run_collaborative(&self, task: &str) -> crate::Result<OrchestratorResult> {
        // 1. Use LLM to decompose the task into subtasks for map-reduce
        let subtasks = self.decompose_for_collaborative(task).await?;
        let total_subtasks = subtasks.len();

        tracing::info!(
            subtasks = total_subtasks,
            "Collaborative mode: decomposed task into subtasks"
        );

        // 2. Create MapReduceOrchestrator with a tools factory that clones our registry
        let tools = self.tools.clone();
        let map_reduce = MapReduceOrchestrator::new(self.provider.clone(), move || tools.clone())
            .max_parallel(self.config.max_workers)
            .model(self.config.coordinator_model.clone());

        // 3. Execute via map-reduce
        let reduce_prompt = format!(
            "Synthesize the results from all sub-tasks that were executed to accomplish: \
             '{}'\n\nProvide a comprehensive summary of what was accomplished, any files \
             modified, and any failures encountered.",
            task
        );

        let summary = map_reduce.execute(subtasks, &reduce_prompt).await?;

        // 4. Build OrchestratorResult from SwarmSummary
        let tasks_completed = summary.completed_tasks.len();
        let tasks_failed = summary.failures.len();
        let total_tokens = summary.total_tokens;

        Ok(OrchestratorResult {
            final_answer: summary.summary,
            tasks_completed,
            tasks_failed,
            total_tokens,
            duration_ms: 0, // filled in by run()
        })
    }

    // --------------------------------------------------------
    //  LLM-based task decomposition for Collaborative mode
    // --------------------------------------------------------

    /// Uses the LLM to decompose a high-level task into `map_reduce::SubTask` entries
    /// suitable for parallel execution by the MapReduceOrchestrator.
    async fn decompose_for_collaborative(
        &self,
        task: &str,
    ) -> crate::Result<Vec<MRSubTask>> {
        let system_prompt = "\
            You are a task decomposition agent. Break the given task into independent, \
            parallelizable sub-tasks. Each sub-task should be self-contained and executable \
            by a sub-agent.\n\n\
            Respond with ONLY a valid JSON object in this exact format:\n\
            {\"subtasks\":[{\"id\":\"sub_0\",\"description\":\"...\",\"system_prompt\":\"optional \
            specialized instructions\"}]}\n\n\
            Rules:\n\
            - Create 2-6 sub-tasks depending on complexity\n\
            - Each sub-task must be independently executable\n\
            - Do NOT create sub-tasks that depend on each other\n\
            - Output ONLY the JSON object, no explanation or markdown";

        let req = ProviderRequest {
            model: self.config.coordinator_model.clone(),
            messages: vec![Message::new(
                Role::User,
                vec![ContentBlock::Text {
                    text: format!(
                        "Decompose this task into parallel sub-tasks:\n\n{}",
                        task
                    ),
                }],
            )],
            system_prompt: Some(system_prompt.to_string()),
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

        let text = resp
            .message
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        // Parse the LLM JSON response into SubTask structs
        let parsed: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            crate::Error::Agent(format!(
                "Failed to parse collaborative decomposition JSON: {}. Raw: {}",
                e, text
            ))
        })?;

        let subtasks_arr = parsed["subtasks"]
            .as_array()
            .ok_or_else(|| {
                crate::Error::Agent(
                    "Missing 'subtasks' array in LLM decomposition response".to_string(),
                )
            })?;

        let subtasks: Vec<MRSubTask> = subtasks_arr
            .iter()
            .enumerate()
            .map(|(i, item)| MRSubTask {
                id: item["id"]
                    .as_str()
                    .unwrap_or(&format!("sub_{}", i))
                    .to_string(),
                description: item["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                system_prompt: item["system_prompt"]
                    .as_str()
                    .map(|s| s.to_string()),
            })
            .collect();

        if subtasks.is_empty() {
            // Fallback: treat the entire task as a single subtask
            tracing::warn!(
                "LLM returned empty subtasks list, falling back to single subtask"
            );
            Ok(vec![MRSubTask {
                id: "sub_0".to_string(),
                description: task.to_string(),
                system_prompt: None,
            }])
        } else {
            Ok(subtasks)
        }
    }
}
