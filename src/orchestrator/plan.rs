//! 任务规划与拆解
//!
//! 将用户请求分解为可执行的子任务

use serde::{Deserialize, Serialize};

/// 子任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: String,
    pub description: String,
    pub task_type: TaskType,
    pub dependencies: Vec<String>,
    pub status: TaskStatus,
    pub result: Option<String>,
}

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskType {
    /// 代码搜索/阅读
    CodeSearch,
    /// 代码编写/修改
    CodeWrite,
    /// 代码审查
    CodeReview,
    /// 测试
    Test,
    /// 通用任务
    General,
}

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 任务计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub goal: String,
    pub tasks: Vec<SubTask>,
}

impl TaskPlan {
    /// 从 LLM JSON 响应解析任务计划
    pub fn from_json(json_str: &str) -> crate::Result<Self> {
        let raw: serde_json::Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Plan JSON parse failed: {}, creating fallback single task", e);
                return Ok(Self {
                    goal: json_str.chars().take(100).collect(),
                    tasks: vec![SubTask {
                        id: "task_0".to_string(),
                        description: json_str.to_string(),
                        task_type: TaskType::General,
                        dependencies: vec![],
                        status: TaskStatus::Pending,
                        result: None,
                    }],
                });
            }
        };

        let goal = raw["goal"].as_str().unwrap_or("Unknown goal").to_string();
        let tasks_raw = raw["tasks"].as_array().cloned().unwrap_or_default();

        let tasks: Vec<SubTask> = tasks_raw.iter().enumerate().map(|(i, t)| {
            let task_type = match t["type"].as_str().unwrap_or("general") {
                "search" => TaskType::CodeSearch,
                "write" => TaskType::CodeWrite,
                "review" => TaskType::CodeReview,
                "test" => TaskType::Test,
                _ => TaskType::General,
            };
            let deps: Vec<String> = t["dependencies"].as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            SubTask {
                id: t["id"].as_str().unwrap_or(&format!("task_{}", i)).to_string(),
                description: t["description"].as_str().unwrap_or("").to_string(),
                task_type,
                dependencies: deps,
                status: TaskStatus::Pending,
                result: None,
            }
        }).collect();

        Ok(TaskPlan { goal, tasks })
    }

    /// 获取下一个可执行的任务 (所有依赖已完成)
    pub fn next_executable(&self) -> Option<&SubTask> {
        self.tasks.iter().find(|t| {
            t.status == TaskStatus::Pending
                && t.dependencies.iter().all(|dep| {
                    self.tasks.iter().any(|d| d.id == *dep && d.status == TaskStatus::Completed)
                })
        })
    }

    /// 获取所有可执行的任务 (无依赖或依赖全部完成) — 用于并行执行
    pub fn next_executable_batch(&self) -> Vec<&SubTask> {
        self.tasks.iter().filter(|t| {
            t.status == TaskStatus::Pending
                && t.dependencies.iter().all(|dep| {
                    self.tasks.iter().any(|d| d.id == *dep && d.status == TaskStatus::Completed)
                })
        }).collect()
    }

    /// 是否所有任务都已完成
    pub fn is_complete(&self) -> bool {
        self.tasks.iter().all(|t| t.status == TaskStatus::Completed || t.status == TaskStatus::Failed)
    }

    /// 标记任务完成
    pub fn mark_completed(&mut self, task_id: &str, result: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Completed;
            task.result = Some(result);
        }
    }

    /// 标记任务失败
    pub fn mark_failed(&mut self, task_id: &str, error: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Failed;
            task.result = Some(format!("ERROR: {}", error));
        }
    }

    /// 获取已完成任务的结果摘要
    pub fn completed_summary(&self) -> String {
        self.tasks.iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .filter_map(|t| {
                t.result.as_ref().map(|r| format!("[{}] {}: {}", t.id, t.description, r))
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    }
}

/// 任务规划的系统提示
pub const PLANNING_SYSTEM_PROMPT: &str = r#"You are a task planning agent. Break a coding task into subtasks.

IMPORTANT: Respond with ONLY a valid JSON object. No explanation, no markdown.

Format:
{"goal":"description","tasks":[{"id":"task_0","description":"what to do","type":"general","dependencies":[]}]}

Rules:
- type: search|write|review|test|general
- For SIMPLE tasks (call a tool, read a file, answer a question): use 1 task only, type "general"
- For COMPLEX tasks (build a feature, refactor code): use 2-4 tasks
- NEVER create more than 4 tasks
- Do NOT create a "search" task unless you need to explore many files
- Set dependencies as task IDs that must complete first
- Output ONLY the JSON object"#;
