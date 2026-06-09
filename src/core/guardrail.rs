use async_trait::async_trait;

// ============================================================
//  积木: Guardrail (护栏系统)
//  职责: 在模型提出意图时拦截/放行
//  组合: 多个 Guardrail 可链式组合
//        权限护栏 + 预算护栏 + Hook 护栏 + 策略护栏
// ============================================================

/// 模型意图
#[derive(Debug, Clone)]
pub enum Intent {
    /// 执行工具调用
    ToolUse { tool_name: String, input: serde_json::Value },
    /// 生成文本
    Text { content: String },
    /// 终止
    EndTurn,
}

/// 护栏检查结果
#[derive(Debug, Clone)]
pub enum GuardResult {
    /// 放行
    Allow,
    /// 拒绝 + 理由
    Deny(String),
    /// 跳过本次操作
    Skip,
}

/// Guardrail trait — 实现此 trait 可添加任意检查规则
#[async_trait]
pub trait Guardrail: Send + Sync {
    fn name(&self) -> &str;

    /// 在工具执行前检查
    async fn check_pre_tool(
        &self,
        ctx: &TurnContext,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> GuardResult;

    /// 在模型输出前检查 (文本拦截)
    async fn check_pre_output(
        &self,
        ctx: &TurnContext,
        content: &str,
    ) -> GuardResult;
}

/// Turn 上下文 — 当前轮次的信息
#[derive(Debug, Clone)]
pub struct TurnContext {
    pub turn_number: u64,
    pub tool_call_count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

// ============================================================
//  内置护栏积木
// ============================================================

/// 权限护栏 — 基于工具名/权限级的简单 ACL
pub struct PermissionGuardrail {
    /// (全局通配) 或 具体工具名 → 所需等级
    rules: Vec<PermissionRule>,
}

pub enum PermissionLevel {
    /// 免确认
    None,
    /// 只读操作
    Read,
    /// 写操作 (需确认)
    Write,
    /// 危险操作 (需明确确认)
    Dangerous,
    /// 禁止
    Forbidden,
}

struct PermissionRule {
    tool_pattern: String,       // 支持 glob: "bash*", "file_*"
    required_level: PermissionLevel,
}

impl PermissionGuardrail {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn add_rule(mut self, tool_pattern: &str, level: PermissionLevel) -> Self {
        self.rules.push(PermissionRule {
            tool_pattern: tool_pattern.to_string(),
            required_level: level,
        });
        self
    }
}

#[async_trait]
impl Guardrail for PermissionGuardrail {
    fn name(&self) -> &str { "permission" }

    async fn check_pre_tool(
        &self,
        _ctx: &TurnContext,
        tool_name: &str,
        _input: &serde_json::Value,
    ) -> GuardResult {
        for rule in &self.rules {
            let pattern = &rule.tool_pattern;
            let matched = if pattern.ends_with('*') {
                // 前缀匹配: "bash*" matches "bash", "bash_exec"
                tool_name.starts_with(pattern.trim_end_matches('*'))
            } else {
                // 精确匹配
                tool_name == pattern
            };
            if matched {
                return match rule.required_level {
                    PermissionLevel::Forbidden => GuardResult::Deny("tool is forbidden".into()),
                    PermissionLevel::Dangerous => GuardResult::Deny("requires explicit approval".into()),
                    PermissionLevel::Write => GuardResult::Deny("requires confirmation".into()),
                    _ => GuardResult::Allow,
                };
            }
        }
        GuardResult::Allow
    }

    async fn check_pre_output(&self, _ctx: &TurnContext, _content: &str) -> GuardResult {
        GuardResult::Allow
    }
}

/// 预算护栏 — 超限时阻断
pub struct BudgetGuardrail {
    max_tool_calls_per_turn: u64,
    max_total_tokens: u64,
}

impl BudgetGuardrail {
    pub fn new(max_tool_calls: u64, max_tokens: u64) -> Self {
        Self {
            max_tool_calls_per_turn: max_tool_calls,
            max_total_tokens: max_tokens,
        }
    }
}

#[async_trait]
impl Guardrail for BudgetGuardrail {
    fn name(&self) -> &str { "budget" }

    async fn check_pre_tool(
        &self,
        ctx: &TurnContext,
        _tool_name: &str,
        _input: &serde_json::Value,
    ) -> GuardResult {
        if ctx.tool_call_count >= self.max_tool_calls_per_turn {
            return GuardResult::Deny(format!(
                "max tool calls per turn ({}) exceeded", self.max_tool_calls_per_turn
            ));
        }
        if ctx.total_input_tokens + ctx.total_output_tokens >= self.max_total_tokens {
            return GuardResult::Deny("total token budget exceeded".into());
        }
        GuardResult::Allow
    }

    async fn check_pre_output(&self, ctx: &TurnContext, _content: &str) -> GuardResult {
        if ctx.total_input_tokens + ctx.total_output_tokens >= self.max_total_tokens {
            return GuardResult::Deny("total token budget exceeded".into());
        }
        GuardResult::Allow
    }
}

// ============================================================
//  护栏链 — 组合多个护栏，依次检查
// ============================================================

/// 护栏链: 积木式组合多个护栏
pub struct GuardrailChain {
    guardrails: Vec<Box<dyn Guardrail>>,
}

impl GuardrailChain {
    pub fn new() -> Self {
        Self { guardrails: Vec::new() }
    }

    pub fn add(mut self, guardrail: Box<dyn Guardrail>) -> Self {
        self.guardrails.push(guardrail);
        self
    }

    pub fn add_all(mut self, guardrails: Vec<Box<dyn Guardrail>>) -> Self {
        self.guardrails.extend(guardrails);
        self
    }

    pub async fn check_tool(
        &self,
        ctx: &TurnContext,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> GuardResult {
        for g in &self.guardrails {
            match g.check_pre_tool(ctx, tool_name, input).await {
                GuardResult::Allow => continue,
                other => return other,
            }
        }
        GuardResult::Allow
    }

    pub async fn check_output(
        &self,
        ctx: &TurnContext,
        content: &str,
    ) -> GuardResult {
        for g in &self.guardrails {
            match g.check_pre_output(ctx, content).await {
                GuardResult::Allow => continue,
                other => return other,
            }
        }
        GuardResult::Allow
    }
}
