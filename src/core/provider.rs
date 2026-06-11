use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::core::TokenCount;

// ============================================================
//  积木: Provider (LLM 提供商抽象层)
//  职责: 统一所有 LLM 提供商接口
//  替换: 实现此 trait 可接入任何 LLM
// ============================================================

/// 消息角色
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

/// 消息内容块
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Text { text: String },
    /// 工具调用 (含真实的 tool_call_id)
    ToolUse { tool_name: String, input: serde_json::Value, tool_call_id: String },
    /// 工具结果
    ToolResult { tool_name: String, content: String, is_error: bool, tool_call_id: String },
    /// 图片: data 存储 Base64 编码的字符串，media_type 存储 MIME 类型
    Image { data: String, media_type: String },
    /// 推理/思考内容
    Thinking { text: String },
}

/// 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    /// 推理内容 (DeepSeek thinking mode, 需回传)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// 标记此消息为缓存断点 (从此处开始可以缓存)
    #[serde(default)]
    pub cache_breakpoint: bool,
}

impl Message {
    pub fn new(role: Role, content: Vec<ContentBlock>) -> Self {
        Self { role, content, reasoning_content: None, cache_breakpoint: false }
    }
}

/// 标记消息为缓存断点
pub fn mark_cache_breakpoint(messages: &mut [Message]) {
    if let Some(last) = messages.last_mut() {
        last.cache_breakpoint = true;
    }
}

/// 检查是否有缓存断点
pub fn has_cache_breakpoint(messages: &[Message]) -> bool {
    messages.iter().any(|m| m.cache_breakpoint)
}

/// Provider 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
    pub max_tokens: Option<TokenCount>,
    pub temperature: Option<f64>,
    pub stream: bool,
    /// 工具定义 (JSON Schema 格式)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    /// DeepSeek thinking mode (默认 enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>,
    /// DeepSeek reasoning effort (high/medium/low)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// 启用 prompt caching (Anthropic cache_control)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_prompt_cache: Option<bool>,
    /// 缓存 key (相同 key 复用缓存)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
}

/// Provider 响应
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub message: Message,
    pub usage: UsageInfo,
}

/// 用量信息
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: TokenCount,
    pub output_tokens: TokenCount,
    pub cache_creation_tokens: TokenCount,
    pub cache_read_tokens: TokenCount,
}

/// 流式事件
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// 文本增量
    Text { delta: String },
    /// 推理/思考内容 (DeepSeek R1 特有)
    Thinking { delta: String },
    /// 工具调用
    ToolUse { tool_name: String, input: serde_json::Value, tool_call_id: String },
    /// 工具结果
    ToolResult { tool_name: String, content: String, is_error: bool },
    /// 完成 (含用量信息)
    Done { usage: UsageInfo },
    /// 错误
    Error { message: String },
}

/// LLM Provider trait — 实现此 trait 即可接入任意 LLM
#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn supported_models(&self) -> Vec<&str>;
    async fn complete(&self, req: ProviderRequest) -> crate::Result<ProviderResponse>;

    /// 流式调用 — 通过 channel 发送事件, 调用方 recv
    async fn stream(
        &self,
        req: ProviderRequest,
        tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
    ) -> crate::Result<()>;
}

// ============================================================
//  内置: Token 预算管理
// ============================================================

/// Token 预算跟踪器
#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub max_input_tokens: TokenCount,
    pub max_output_tokens: TokenCount,
    pub used_input: TokenCount,
    pub used_output: TokenCount,
    pub warning_threshold: f64,   // 默认 0.80
    pub critical_threshold: f64,  // 默认 0.95
}

impl TokenBudget {
    pub fn new(max_input: TokenCount, max_output: TokenCount) -> Self {
        if max_input == 0 {
            tracing::warn!("TokenBudget: max_input_tokens is 0, budget will report full usage for input");
        }
        if max_output == 0 {
            tracing::warn!("TokenBudget: max_output_tokens is 0, budget will report full usage for output");
        }
        Self {
            max_input_tokens: max_input,
            max_output_tokens: max_output,
            used_input: 0,
            used_output: 0,
            warning_threshold: 0.80,
            critical_threshold: 0.95,
        }
    }

    pub fn input_usage(&self) -> f64 {
        if self.max_input_tokens == 0 { return 1.0; } // 零预算视为已满
        self.used_input as f64 / self.max_input_tokens as f64
    }

    pub fn output_usage(&self) -> f64 {
        if self.max_output_tokens == 0 { return 1.0; } // 零预算视为已满
        self.used_output as f64 / self.max_output_tokens as f64
    }

    pub fn record_usage(&mut self, usage: &UsageInfo) {
        // input_tokens = 当前请求的完整 prompt 大小 (已包含所有历史消息)
        // 用 = 覆盖而非 += 累加，避免跨轮次重复计算导致 budget 误判
        self.used_input = usage.input_tokens;
        // output_tokens 是每次 API 调用的新产出，累加正确
        self.used_output += usage.output_tokens;
    }

    /// 重置使用量为指定值 (压缩后使用，反映当前实际上下文大小)
    pub fn reset_to(&mut self, current_input: TokenCount, current_output: TokenCount) {
        self.used_input = current_input;
        self.used_output = current_output;
    }

    /// 返回当前状态: Ok / Warning / Critical
    pub fn status(&self) -> BudgetStatus {
        let usage = self.input_usage().max(self.output_usage());
        if usage >= self.critical_threshold {
            BudgetStatus::Critical
        } else if usage >= self.warning_threshold {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Ok
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BudgetStatus {
    Ok,
    Warning,
    Critical,
}

// ============================================================
//  Tests: TokenBudget 预算管理
// ============================================================

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn test_zero_budget_returns_full() {
        let budget = TokenBudget::new(0, 0);
        // Zero budget is treated as fully used
        assert_eq!(budget.input_usage(), 1.0);
        assert_eq!(budget.output_usage(), 1.0);
    }

    #[test]
    fn test_normal_usage_calculation() {
        let mut budget = TokenBudget::new(1000, 500);
        budget.used_input = 250;
        budget.used_output = 100;
        assert!((budget.input_usage() - 0.25).abs() < f64::EPSILON);
        assert!((budget.output_usage() - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn test_status_ok_under_thresholds() {
        let mut budget = TokenBudget::new(1000, 1000);
        budget.used_input = 500;
        budget.used_output = 500;
        assert_eq!(budget.status(), BudgetStatus::Ok);
    }

    #[test]
    fn test_status_warning_at_threshold() {
        let mut budget = TokenBudget::new(1000, 1000);
        budget.used_input = 800;
        budget.used_output = 0;
        assert_eq!(budget.status(), BudgetStatus::Warning);
    }

    #[test]
    fn test_status_critical_at_threshold() {
        let mut budget = TokenBudget::new(1000, 1000);
        budget.used_input = 950;
        budget.used_output = 0;
        assert_eq!(budget.status(), BudgetStatus::Critical);
    }

    #[test]
    fn test_record_usage_input_overwrites_output_accumulates() {
        let mut budget = TokenBudget::new(1000, 500);
        let usage1 = UsageInfo {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        };
        let usage2 = UsageInfo {
            input_tokens: 200,
            output_tokens: 100,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        };
        budget.record_usage(&usage1);
        budget.record_usage(&usage2);
        // input: 覆盖为最新值 200 (不是累加 300)
        assert_eq!(budget.used_input, 200);
        // output: 累加 50 + 100 = 150
        assert_eq!(budget.used_output, 150);
    }

    #[test]
    fn test_multi_turn_budget_no_false_critical() {
        // 模拟真实场景: 128K context window, 10 轮工具调用
        let mut budget = TokenBudget::new(128_000, 128_000);

        // 每轮 API 返回的 input_tokens 是当前完整 prompt 大小 (递增)
        let per_turn_inputs = [5000, 8000, 12000, 18000, 25000, 32000, 40000, 48000, 55000, 60000];
        let per_turn_outputs = [500u64; 10];

        for (i, (&input, &output)) in per_turn_inputs.iter().zip(per_turn_outputs.iter()).enumerate() {
            budget.record_usage(&UsageInfo {
                input_tokens: input,
                output_tokens: output,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            });
            // 每轮的 input_usage 应该 = 当前轮的 prompt 大小 / context window
            let expected = input as f64 / 128_000.0;
            assert!(
                (budget.input_usage() - expected).abs() < 0.001,
                "Turn {}: expected input_usage={}, got={}",
                i, expected, budget.input_usage()
            );
        }

        // 最终: input = 60000 / 128000 = 46.9% → Ok
        assert_eq!(budget.status(), BudgetStatus::Ok);
        assert!((budget.input_usage() - 60000.0 / 128000.0).abs() < 0.001);
        // output 累积: 500 * 10 = 5000
        assert_eq!(budget.used_output, 5000);
    }
}
