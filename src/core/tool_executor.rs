//! 工具执行器 — 统一管理工具执行逻辑
//!
//! 消除并行/串行执行路径的重复代码
//!
//! 设计: 构造时持有所有依赖 (不可变引用/Arc), 一次调用完成完整流程
//!   - 护栏检查
//!   - Hook Before/After
//!   - 缓存查询与写入
//!   - 工具执行
//!   - StepObserver 评估
//!   - 审计日志
//!   - Rollout 记录
//!   - Goal token 累计
//!   - 事件发射
//!
//! 提供两种风格:
//!   - 静态方法 `execute_single_tool`: 简单包装 (cache + 工具执行), 供当前 loop.rs 使用
//!   - 实例方法 `execute_full` / `execute_serial` / `execute_parallel`: 完整流程, 供未来重构

use std::sync::Arc;
use std::time::Instant;

use crate::tools::registry::ToolRegistry;
use crate::tools::{ToolContext, ToolResult};
use crate::core::cache::{GlobalCache, ToolCacheKey, compute_input_hash};
use crate::core::guardrail::{GuardrailChain, GuardResult, TurnContext};
use crate::core::hooks::{HookEngine, HookResult};
use crate::core::execpolicy::{ExecPolicy, Decision as PolicyDecision};
use crate::core::r#loop::{LoopEvent, EventCallback, StepObserver, ObserveAction, truncate_str};
use crate::session::rollout::RolloutRecorder;
use crate::audit::{AuditEvent as GlobalAuditEvent, AUDIT_LOGGER};

// ============================================================
//  公共数据结构
// ============================================================

/// 单个工具调用信息
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub tool_call_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

impl PendingToolCall {
    /// 创建新的工具调用
    pub fn new(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
        }
    }

    /// 从 (id, name, input) 元组构造
    pub fn from_tuple(t: (String, String, serde_json::Value)) -> Self {
        Self {
            tool_call_id: t.0,
            tool_name: t.1,
            input: t.2,
        }
    }

    /// 转换为元组
    pub fn into_tuple(self) -> (String, String, serde_json::Value) {
        (self.tool_call_id, self.tool_name, self.input)
    }
}

/// 工具执行最终结果 (与 ToolResult 区分: 包含调用上下文)
#[derive(Debug, Clone)]
pub struct ToolExecutionOutcome {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: ToolResult,
    pub duration_ms: u64,
    /// 是否命中缓存
    pub cache_hit: bool,
}

/// 工具执行器配置
///
/// 兼容旧用法: 至少需要 `session_id`, `agent_id`, `registry`
/// 扩展字段: 启用完整流程时, 需要设置 `guardrails`/`hook_engine`/`exec_policy`/`event_callback`/`rollout`
pub struct ToolExecutorConfig {
    /// 会话 ID (用于审计与上下文)
    pub session_id: String,
    /// Agent ID
    pub agent_id: String,
    /// Agent 注册表 (用于跨 Agent 通信)
    pub registry: Option<Arc<crate::agent::registry::AgentRegistry>>,

    // ── 扩展字段 (实例方法使用) ──
    /// 工作目录 (默认 `std::env::current_dir()`)
    pub working_dir: Option<String>,
    /// 当前轮次号 (用于 ToolContext)
    pub turn_number: Option<u64>,
    /// 护栏链
    pub guardrails: Option<GuardrailChain>,
    /// Hook 引擎
    pub hook_engine: Option<Arc<tokio::sync::Mutex<HookEngine>>>,
    /// ExecPolicy (仅 bash 工具检查)
    pub exec_policy: Option<ExecPolicy>,
    /// 事件回调
    pub event_callback: Option<EventCallback>,
    /// Rollout 记录器 (Arc 包装以支持并行场景)
    pub rollout: Option<Arc<tokio::sync::Mutex<RolloutRecorder>>>,
}

impl ToolExecutorConfig {
    /// 基础配置 (兼容旧 API)
    pub fn basic(
        session_id: impl Into<String>,
        agent_id: impl Into<String>,
        registry: Option<Arc<crate::agent::registry::AgentRegistry>>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_id: agent_id.into(),
            registry,
            working_dir: None,
            turn_number: None,
            guardrails: None,
            hook_engine: None,
            exec_policy: None,
            event_callback: None,
            rollout: None,
        }
    }
}

/// 执行状态 (由调用方持有, 工具执行过程中累加计数)
#[derive(Debug, Clone, Default)]
pub struct ExecutionState {
    pub tool_call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

/// 工具执行错误
///
/// - `GuardrailDenied` / `HookBlocked` / `PolicyForbidden`: 非致命, 调用方应继续
/// - `Replan`: 非致命, 调用方应注入 hint 系统消息后继续
/// - `EarlyStop` / `MaxToolCallsExceeded`: 致命, 调用方应终止循环
#[derive(Debug, Clone)]
pub enum ToolExecutorError {
    /// 护栏拒绝 (非致命)
    GuardrailDenied {
        tool_call_id: String,
        tool_name: String,
        reason: String,
    },
    /// 钩子拒绝 (非致命)
    HookBlocked {
        tool_call_id: String,
        tool_name: String,
        reason: String,
    },
    /// ExecPolicy 拒绝 (非致命)
    PolicyForbidden {
        tool_call_id: String,
        tool_name: String,
        command: String,
    },
    /// StepObserver 建议 Replan (非致命, 调用方注入 hint 后继续)
    Replan(String),
    /// StepObserver 建议 EarlyStop (致命)
    EarlyStop(String),
    /// 超过最大工具调用数 (致命)
    MaxToolCallsExceeded,
}

// ============================================================
//  工具执行输出 (向后兼容)
// ============================================================

/// 工具执行输出 — 封装结果和耗时
///
/// 与 `ToolExecutionOutcome` 的区别: 仅包含基础字段, 不含 `cache_hit` 标志
/// 供旧 API (`execute_single_tool`) 使用
pub struct ToolExecuteOutput {
    pub result: ToolResult,
    pub duration_ms: u64,
}

// ============================================================
//  工具执行器主结构
// ============================================================

/// 工具执行器
///
/// 持有所有执行工具所需的依赖, 通过方法封装完整流程
pub struct ToolExecutor {
    tools: Arc<ToolRegistry>,
    cache: Arc<GlobalCache>,
    config: ToolExecutorConfig,
    observer: Arc<tokio::sync::Mutex<StepObserver>>,
}

impl ToolExecutor {
    /// 创建执行器 (持有所有依赖)
    ///
    /// # 参数
    /// - `tools`: 工具注册表
    /// - `cache`: 全局缓存
    /// - `config`: 执行器配置
    /// - `observer`: 步骤观察器 (Arc 包装以便跨任务共享)
    pub fn new(
        tools: Arc<ToolRegistry>,
        cache: Arc<GlobalCache>,
        config: ToolExecutorConfig,
        observer: Arc<tokio::sync::Mutex<StepObserver>>,
    ) -> Self {
        Self {
            tools,
            cache,
            config,
            observer,
        }
    }

    // ── 静态方法 (向后兼容) ────────────────────────────────────

    /// 执行单个工具 (含缓存) — 静态方法
    ///
    /// 仅做: 缓存查询 → 工具执行 → 缓存写入, 不做护栏/Hook/Observer 等检查
    /// 供 `run_simple_loop` 的并行/串行路径统一调用
    pub async fn execute_single_tool(
        tools: &ToolRegistry,
        cache: &GlobalCache,
        config: &ToolExecutorConfig,
        tool_name: &str,
        input: &serde_json::Value,
        turn_number: u64,
    ) -> ToolExecuteOutput {
        let start_time = Instant::now();
        let cache_key = ToolCacheKey {
            tool_name: tool_name.to_string(),
            input_hash: compute_input_hash(tool_name, input),
        };

        let result = if let Some(cached_val) = cache.get_tool_result(&cache_key).await {
            ToolResult {
                content: cached_val,
                is_error: false,
                metadata: None,
            }
        } else {
            let tool_ctx = ToolContext {
                session_id: config.session_id.clone(),
                working_dir: config
                    .working_dir
                    .clone()
                    .unwrap_or_else(|| {
                        std::env::current_dir()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| ".".into())
                    }),
                turn_number,
                agent_id: config.agent_id.clone(),
                registry: config.registry.clone(),
                node_id: None,
                upstream_data: None,
                execution_mode: Default::default(),
            };
            match tools.execute(tool_name, input.clone(), &tool_ctx).await {
                Ok(res) => {
                    if !res.is_error {
                        cache
                            .set_tool_result(cache_key, res.content.clone(), input)
                            .await;
                    }
                    res
                }
                Err(e) => ToolResult {
                    content: format!("Tool error: {}", e),
                    is_error: true,
                    metadata: None,
                },
            }
        };
        let elapsed = start_time.elapsed().as_millis() as u64;
        ToolExecuteOutput {
            result,
            duration_ms: elapsed,
        }
    }

    // ── 实例方法: 完整流程 ────────────────────────────────────

    /// 完整执行单个工具: 护栏 → Hook Before → 事件 → 缓存 → 执行 → Observer → 事件 → Hook After
    ///
    /// 与静态 `execute_single_tool` 的区别: 这个方法会做完整的拦截链检查
    ///
    /// # 返回
    /// - `Ok(outcome)`: 执行成功
    /// - `Err(GuardrailDenied|HookBlocked|PolicyForbidden)`: 被拦截 (非致命)
    /// - `Err(EarlyStop|MaxToolCallsExceeded)`: 致命错误
    pub async fn execute_full(
        &self,
        call: &PendingToolCall,
        turn_ctx: &TurnContext,
    ) -> Result<ToolExecutionOutcome, ToolExecutorError> {
        // 1. 护栏检查
        if let Some(ref gc) = self.config.guardrails {
            match gc.check_tool(turn_ctx, &call.tool_name, &call.input).await {
                GuardResult::Allow => {}
                GuardResult::Deny(reason) => {
                    return Err(ToolExecutorError::GuardrailDenied {
                        tool_call_id: call.tool_call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        reason,
                    });
                }
                GuardResult::Skip => {
                    // Skip 也作为非致命错误返回
                    return Err(ToolExecutorError::GuardrailDenied {
                        tool_call_id: call.tool_call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        reason: "skipped".into(),
                    });
                }
            }
        }

        // 2. ExecPolicy 检查 (仅 bash)
        if let Some(ref policy) = self.config.exec_policy {
            if call.tool_name == "bash" {
                if let Some(cmd) = call.input.get("command").and_then(|v| v.as_str()) {
                    match policy.check(cmd) {
                        PolicyDecision::Allow => {}
                        PolicyDecision::Forbid => {
                            return Err(ToolExecutorError::PolicyForbidden {
                                tool_call_id: call.tool_call_id.clone(),
                                tool_name: call.tool_name.clone(),
                                command: cmd.to_string(),
                            });
                        }
                    }
                }
            }
        }

        // 3. Rollout: 工具开始
        if let Some(ref rollout) = self.config.rollout {
            let mut guard = rollout.lock().await;
            let _ = guard.tool_start(&call.tool_name, &call.input);
        }

        // 4. Hook BeforeTool
        if let Some(ref hook) = self.config.hook_engine {
            let mut guard = hook.lock().await;
            match guard.run_before(&call.tool_name, &call.input.to_string()) {
                HookResult::Continue => {}
                HookResult::Block(reason) => {
                    return Err(ToolExecutorError::HookBlocked {
                        tool_call_id: call.tool_call_id.clone(),
                        tool_name: call.tool_name.clone(),
                        reason,
                    });
                }
                HookResult::Retry => { /* 后续处理 */ }
            }
        }

        // 5. 发射 ToolStart 事件
        self.emit_tool_start(call);

        // 6. 执行 (含缓存)
        let start_time = Instant::now();
        let (mut result, cache_hit) = self.execute_with_cache(&call.tool_name, &call.input).await;
        let elapsed_ms = start_time.elapsed().as_millis() as u64;

        // 7. StepObserver (含 Retry 循环)
        let observer_action = self.run_observer_with_retry(call, &mut result).await;

        // 8. 审计
        {
            let mut logger = AUDIT_LOGGER.lock().await;
            logger.log(
                GlobalAuditEvent::ToolCall {
                    tool_name: call.tool_name.clone(),
                    input_summary: truncate_str(&call.input.to_string(), 200),
                    success: !result.is_error,
                    duration_ms: elapsed_ms,
                },
                "loop",
            );
        }

        // 9. 发射 ToolEnd 事件
        self.emit_tool_end(call, &result, elapsed_ms);

        // 10. Hook AfterTool
        if let Some(ref hook) = self.config.hook_engine {
            let mut guard = hook.lock().await;
            let _ = guard.run_after(&call.tool_name, &result.content);
        }

        // 11. Rollout: 工具结束
        if let Some(ref rollout) = self.config.rollout {
            let mut guard = rollout.lock().await;
            let _ = guard.tool_end(
                &call.tool_name,
                !result.is_error,
                &result.content,
                elapsed_ms,
            );
        }

        // 12. StepObserver Replan / EarlyStop 处理
        match observer_action {
            Some(ObserveAction::Replan { hint }) => {
                return Err(ToolExecutorError::Replan(hint));
            }
            Some(ObserveAction::EarlyStop { reason }) => {
                return Err(ToolExecutorError::EarlyStop(reason));
            }
            _ => {}
        }

        Ok(ToolExecutionOutcome {
            tool_call_id: call.tool_call_id.clone(),
            tool_name: call.tool_name.clone(),
            result,
            duration_ms: elapsed_ms,
            cache_hit,
        })
    }

    /// 串行执行一批工具
    ///
    /// 按顺序逐个执行, 遇到非致命错误时记录后继续
    pub async fn execute_serial(
        &self,
        calls: Vec<PendingToolCall>,
        turn_ctx: &TurnContext,
    ) -> Vec<Result<ToolExecutionOutcome, ToolExecutorError>> {
        let mut outcomes = Vec::new();
        for call in &calls {
            outcomes.push(self.execute_full(call, turn_ctx).await);
        }
        outcomes
    }

    /// 并行执行一批工具 (仅当全部只读时使用)
    ///
    /// 使用 `join_all` 并行执行, 遇到非致命错误时记录后继续
    pub async fn execute_parallel(
        self: Arc<Self>,
        calls: Vec<PendingToolCall>,
        turn_ctx: &TurnContext,
    ) -> Vec<Result<ToolExecutionOutcome, ToolExecutorError>> {
        use futures::future::join_all;

        let turn_number = turn_ctx.turn_number;
        let tool_call_count = turn_ctx.tool_call_count;
        let total_input_tokens = turn_ctx.total_input_tokens;
        let total_output_tokens = turn_ctx.total_output_tokens;

        let mut handles = Vec::new();
        for call in calls {
            let executor = Arc::clone(&self);
            handles.push(async move {
                let tc = TurnContext {
                    turn_number,
                    tool_call_count,
                    total_input_tokens,
                    total_output_tokens,
                };
                executor.execute_full(&call, &tc).await
            });
        }
        join_all(handles).await
    }

    // ── 私有方法 ──────────────────────────────────────────────

    /// 缓存查询 + 工具执行
    /// 返回 (result, cache_hit)
    async fn execute_with_cache(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> (ToolResult, bool) {
        let cache_key = ToolCacheKey {
            tool_name: tool_name.to_string(),
            input_hash: compute_input_hash(tool_name, input),
        };

        if let Some(cached_val) = self.cache.get_tool_result(&cache_key).await {
            return (
                ToolResult {
                    content: cached_val,
                    is_error: false,
                    metadata: None,
                },
                true,
            );
        }

        let tool_ctx = ToolContext {
            session_id: self.config.session_id.clone(),
            working_dir: self.config.working_dir.clone().unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| ".".into())
            }),
            turn_number: self.config.turn_number.unwrap_or(0),
            agent_id: self.config.agent_id.clone(),
            registry: self.config.registry.clone(),
            node_id: None,
            upstream_data: None,
            execution_mode: Default::default(),
        };

        let result = match self
            .tools
            .execute(tool_name, input.clone(), &tool_ctx)
            .await
        {
            Ok(res) => {
                if !res.is_error {
                    self.cache
                        .set_tool_result(cache_key, res.content.clone(), input)
                        .await;
                }
                res
            }
            Err(e) => ToolResult {
                content: format!("Tool error: {}", e),
                is_error: true,
                metadata: None,
            },
        };
        (result, false)
    }

    /// StepObserver 循环 (含 Retry 重试)
    ///
    /// 观察结果 → 如果 Retry 则重新执行 → 再次观察, 直到遇到非 Retry 的 action
    /// 返回 `Some(action)` 需要调用方处理 (Replan / EarlyStop / Continue)
    async fn run_observer_with_retry(
        &self,
        call: &PendingToolCall,
        result: &mut ToolResult,
    ) -> Option<ObserveAction> {
        loop {
            let action = {
                let mut observer = self.observer.lock().await;
                observer.observe(&call.tool_name, result)
            };
            match action {
                ObserveAction::Continue => return None,
                ObserveAction::Retry => {
                    tracing::info!("StepObserver retry: tool '{}'", call.tool_name);
                    let (retry_result, _) = self.execute_with_cache(&call.tool_name, &call.input).await;
                    *result = retry_result;
                    // 继续循环, 再次观察
                }
                ObserveAction::Replan { hint: _ } => return Some(action),
                ObserveAction::EarlyStop { reason: _ } => return Some(action),
            }
        }
    }

    /// 发射 ToolStart 事件
    fn emit_tool_start(&self, call: &PendingToolCall) {
        if let Some(ref cb) = self.config.event_callback {
            cb(&LoopEvent::ToolStart {
                tool_name: call.tool_name.clone(),
                tool_id: call.tool_call_id.clone(),
                input: call.input.clone(),
            });
        }
    }

    /// 发射 ToolEnd 事件
    fn emit_tool_end(&self, call: &PendingToolCall, result: &ToolResult, duration_ms: u64) {
        if let Some(ref cb) = self.config.event_callback {
            cb(&LoopEvent::ToolEnd {
                tool_name: call.tool_name.clone(),
                tool_id: call.tool_call_id.clone(),
                result: result.content.clone(),
                is_error: result.is_error,
                duration_ms,
            });
        }
    }
}

// ============================================================
//  工具只读性判断
// ============================================================

/// 判断工具调用列表是否全部为只读工具 (可并行执行)
///
/// 与 loop.rs 内同名函数等价 (集中放置以便复用)
pub fn are_tools_readonly(tools: &[(String, String, serde_json::Value)]) -> bool {
    tools.iter().all(|(_, name, _)| {
        matches!(
            name.as_str(),
            "read"
                | "symbol_search"
                | "find_callers"
                | "project_map"
                | "glob"
                | "grep"
                | "ask_user"
                | "list_peers"
                | "snapshot_history"
                | "web_search"
        )
    })
}

// ============================================================
//  单元测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── are_tools_readonly ──────────────────────────────────

    #[test]
    fn readonly_tools_detected() {
        let tools: Vec<(String, String, serde_json::Value)> = vec![
            ("id1".into(), "read".into(), serde_json::json!({})),
            ("id2".into(), "grep".into(), serde_json::json!({})),
            ("id3".into(), "glob".into(), serde_json::json!({})),
        ];
        assert!(are_tools_readonly(&tools));
    }

    #[test]
    fn write_tools_not_readonly() {
        let tools: Vec<(String, String, serde_json::Value)> = vec![
            ("id1".into(), "read".into(), serde_json::json!({})),
            ("id2".into(), "write".into(), serde_json::json!({})),
        ];
        assert!(!are_tools_readonly(&tools));
    }

    #[test]
    fn bash_not_readonly() {
        let tools: Vec<(String, String, serde_json::Value)> = vec![
            ("id1".into(), "read".into(), serde_json::json!({})),
            ("id2".into(), "bash".into(), serde_json::json!({})),
        ];
        assert!(!are_tools_readonly(&tools));
    }

    #[test]
    fn empty_list_is_readonly() {
        let tools: Vec<(String, String, serde_json::Value)> = vec![];
        assert!(are_tools_readonly(&tools));
    }

    // ── PendingToolCall ─────────────────────────────────────

    #[test]
    fn pending_tool_call_new() {
        let call = PendingToolCall::new("id1", "read", serde_json::json!({"path": "/a"}));
        assert_eq!(call.tool_call_id, "id1");
        assert_eq!(call.tool_name, "read");
        assert_eq!(call.input["path"], "/a");
    }

    #[test]
    fn pending_tool_call_from_tuple() {
        let call = PendingToolCall::from_tuple((
            "id1".into(),
            "read".into(),
            serde_json::json!({"path": "/a"}),
        ));
        let (id, name, _) = call.into_tuple();
        assert_eq!(id, "id1");
        assert_eq!(name, "read");
    }

    // ── ToolExecutorConfig ──────────────────────────────────

    #[test]
    fn config_basic() {
        let cfg = ToolExecutorConfig::basic("session_1", "agent_1", None);
        assert_eq!(cfg.session_id, "session_1");
        assert_eq!(cfg.agent_id, "agent_1");
        assert!(cfg.registry.is_none());
        assert!(cfg.guardrails.is_none());
        assert!(cfg.hook_engine.is_none());
        assert!(cfg.exec_policy.is_none());
        assert!(cfg.event_callback.is_none());
        assert!(cfg.rollout.is_none());
    }

    // ── ToolExecutorError ───────────────────────────────────

    #[test]
    fn error_variants_debug() {
        let e1 = ToolExecutorError::GuardrailDenied {
            tool_call_id: "id".into(),
            tool_name: "bash".into(),
            reason: "forbidden".into(),
        };
        let e2 = ToolExecutorError::EarlyStop("stop".into());
        let e3 = ToolExecutorError::MaxToolCallsExceeded;
        // 验证 Debug 实现
        assert!(format!("{:?}", e1).contains("GuardrailDenied"));
        assert!(format!("{:?}", e2).contains("EarlyStop"));
        assert!(format!("{:?}", e3).contains("MaxToolCallsExceeded"));
    }
}
