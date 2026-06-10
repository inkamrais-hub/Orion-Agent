//! Goal 状态机 — 目标追踪 + Steering 自动续命
//!
//! Codex 设计:
//!   get_goal    → 查看当前目标状态
//!   create_goal → 创建新目标 (带 token 预算)
//!   update_goal → 更新目标状态
//!
//! Steering 自动续命:
//!   - 目标被 block 超过 3 轮 → 自动注入提示让 Agent 换思路
//!   - 预算快用完 → 自动注入提示让 Agent 加速完成

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// 目标状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Blocked,
    Completed,
    Failed,
}

/// 目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub goal_id: String,
    pub description: String,
    pub status: GoalStatus,
    pub token_budget: u64,
    pub tokens_used: u64,
    /// Tracks the last cumulative token value passed to `record_tokens`
    /// so that only the delta is added to `tokens_used`.
    #[serde(default)]
    pub last_recorded_tokens: u64,
    pub block_count: u32,
    pub last_blocked_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Steering 注入类型
#[derive(Debug, Clone)]
pub enum SteeringType {
    /// 目标被 block 太久，建议换思路
    ObjectiveUpdated,
    /// 预算快用完，建议加速
    Continuation,
}

/// Goal 管理器
pub struct GoalManager {
    goals: Vec<Goal>,
    /// 最大 block 次数 (超过则注入 steering)
    max_block_count: u32,
    /// 预算警告阈值 (百分比)
    budget_warning_threshold: f64,
}

impl GoalManager {
    pub fn new() -> Self {
        Self {
            goals: Vec::new(),
            max_block_count: 3,
            budget_warning_threshold: 0.8, // 80%
        }
    }

    /// 创建新目标
    pub fn create(&mut self, description: impl Into<String>, token_budget: u64) -> String {
        let goal_id = format!("goal_{}", uuid::Uuid::new_v4().to_string()[..8].to_string());
        let now = Utc::now();

        let goal = Goal {
            goal_id: goal_id.clone(),
            description: description.into(),
            status: GoalStatus::Active,
            token_budget,
            tokens_used: 0,
            last_recorded_tokens: 0,
            block_count: 0,
            last_blocked_reason: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        self.goals.push(goal);
        goal_id
    }

    /// 更新目标状态
    pub fn update(&mut self, goal_id: &str, status: GoalStatus) -> Option<&Goal> {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.goal_id == goal_id) {
            goal.status = status.clone();
            goal.updated_at = Utc::now();
            if status == GoalStatus::Completed || status == GoalStatus::Failed {
                goal.completed_at = Some(Utc::now());
            }
        }
        self.get(goal_id)
    }

    /// 记录 token 使用
    ///
    /// `cumulative_tokens` is the running total from the caller (e.g.
    /// `state.total_usage.input_tokens + state.total_usage.output_tokens`).
    /// Only the delta since the last call is added to `tokens_used`.
    pub fn record_tokens(&mut self, goal_id: &str, cumulative_tokens: u64) -> Option<&Goal> {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.goal_id == goal_id) {
            let delta = cumulative_tokens.saturating_sub(goal.last_recorded_tokens);
            goal.tokens_used += delta;
            goal.last_recorded_tokens = cumulative_tokens;
            goal.updated_at = Utc::now();
        }
        self.get(goal_id)
    }

    /// 记录 block
    pub fn record_block(&mut self, goal_id: &str, reason: impl Into<String>) -> Option<&Goal> {
        if let Some(goal) = self.goals.iter_mut().find(|g| g.goal_id == goal_id) {
            goal.block_count += 1;
            goal.last_blocked_reason = Some(reason.into());
            goal.status = GoalStatus::Blocked;
            goal.updated_at = Utc::now();
        }
        self.get(goal_id)
    }

    /// 获取目标
    pub fn get(&self, goal_id: &str) -> Option<&Goal> {
        self.goals.iter().find(|g| g.goal_id == goal_id)
    }

    /// 获取当前活跃目标
    pub fn active_goal(&self) -> Option<&Goal> {
        self.goals.iter().find(|g| g.status == GoalStatus::Active || g.status == GoalStatus::Blocked)
    }

    /// 检查是否需要注入 steering
    pub fn check_steering(&self, goal_id: &str) -> Option<SteeringType> {
        let goal = self.get(goal_id)?;

        // 检查 block 次数
        if goal.block_count >= self.max_block_count {
            return Some(SteeringType::ObjectiveUpdated);
        }

        // 检查预算使用率
        if goal.token_budget > 0 {
            let usage_ratio = goal.tokens_used as f64 / goal.token_budget as f64;
            if usage_ratio >= self.budget_warning_threshold {
                return Some(SteeringType::Continuation);
            }
        }

        None
    }

    /// 生成 steering 提示
    pub fn generate_steering_prompt(&self, goal_id: &str) -> Option<String> {
        let goal = self.get(goal_id)?;
        let steering_type = self.check_steering(goal_id)?;

        Some(match steering_type {
            SteeringType::ObjectiveUpdated => {
                format!(
                    "[Steering] 目标 '{}' 已被 block {} 次。建议: 换一种方法或简化目标。",
                    goal.description, goal.block_count
                )
            }
            SteeringType::Continuation => {
                format!(
                    "[Steering] 目标 '{}' 预算使用 {:.0}%。建议: 加速完成或请求更多预算。",
                    goal.description,
                    (goal.tokens_used as f64 / goal.token_budget as f64) * 100.0
                )
            }
        })
    }

    /// 获取目标摘要
    pub fn summary(&self, goal_id: &str) -> Option<String> {
        let goal = self.get(goal_id)?;
        Some(format!(
            "Goal: {} | Status: {:?} | Tokens: {}/{} | Blocks: {}",
            goal.description, goal.status, goal.tokens_used, goal.token_budget, goal.block_count
        ))
    }

    pub fn len(&self) -> usize {
        self.goals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }
}

impl Default for GoalManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── 依赖 ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_goal() {
        let mut manager = GoalManager::new();
        let id = manager.create("测试目标", 1000);
        assert!(id.starts_with("goal_"));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_update_goal() {
        let mut manager = GoalManager::new();
        let id = manager.create("测试目标", 1000);
        let goal = manager.update(&id, GoalStatus::Completed).unwrap();
        assert_eq!(goal.status, GoalStatus::Completed);
    }

    #[test]
    fn test_steering_on_block() {
        let mut manager = GoalManager::new();
        let id = manager.create("测试目标", 1000);
        
        // block 3 次
        manager.record_block(&id, "原因1");
        manager.record_block(&id, "原因2");
        manager.record_block(&id, "原因3");
        
        let steering = manager.check_steering(&id);
        assert!(matches!(steering, Some(SteeringType::ObjectiveUpdated)));
    }

    #[test]
    fn test_steering_on_budget() {
        let mut manager = GoalManager::new();
        let id = manager.create("测试目标", 100);
        
        // 使用 85% 预算
        manager.record_tokens(&id, 85);
        
        let steering = manager.check_steering(&id);
        assert!(matches!(steering, Some(SteeringType::Continuation)));
    }

    #[test]
    fn test_record_tokens_delta() {
        let mut manager = GoalManager::new();
        let id = manager.create("token delta test", 1000);

        // Turn 1: cumulative 100 → delta = 100, tokens_used = 100
        manager.record_tokens(&id, 100);
        assert_eq!(manager.get(&id).unwrap().tokens_used, 100);

        // Turn 2: cumulative 250 → delta = 150, tokens_used = 250
        manager.record_tokens(&id, 250);
        assert_eq!(manager.get(&id).unwrap().tokens_used, 250);

        // Turn 3: cumulative 400 → delta = 150, tokens_used = 400
        manager.record_tokens(&id, 400);
        assert_eq!(manager.get(&id).unwrap().tokens_used, 400);
    }

    #[test]
    fn test_record_tokens_same_value_no_double_count() {
        let mut manager = GoalManager::new();
        let id = manager.create("no double count test", 1000);

        // Record the same cumulative value twice — delta should be 0 the second time
        manager.record_tokens(&id, 100);
        manager.record_tokens(&id, 100);
        assert_eq!(manager.get(&id).unwrap().tokens_used, 100);
    }

    #[test]
    fn test_active_goal_returns_active_and_blocked() {
        let mut manager = GoalManager::new();
        let id1 = manager.create("active goal", 1000);
        let id2 = manager.create("blocked goal", 1000);
        let _id3 = manager.create("completed goal", 1000);

        manager.update(&id2, GoalStatus::Blocked);
        manager.update(&_id3, GoalStatus::Completed);

        // active_goal should return the first Active or Blocked goal
        let active = manager.active_goal();
        assert!(active.is_some());
        let active_id = &active.unwrap().goal_id;
        assert!(active_id == &id1 || active_id == &id2);
    }
}
