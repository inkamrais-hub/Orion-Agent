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

/// 编排模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
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
