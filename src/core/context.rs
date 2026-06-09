use crate::core::TokenCount;
use crate::core::provider::Message;

/// ToolResult 截断长度 (可通过环境变量 ORION_TRUNCATE_LEN 覆盖)
const TRUNCATE_LEN: usize = 200;

// ============================================================
//  积木: Context (上下文管理 + 压缩策略)
//  职责: 监控上下文窗口, 在阈值触发时执行压缩
//  参考: Claude Code 5 级阶梯压缩
// ============================================================

/// 上下文使用状态
#[derive(Debug, Clone)]
pub struct ContextUsage {
    pub used_tokens: TokenCount,
    pub max_tokens: TokenCount,
}

impl ContextUsage {
    pub fn ratio(&self) -> f64 {
        self.used_tokens as f64 / self.max_tokens as f64
    }

    pub fn is_warning(&self) -> bool {
        self.ratio() >= 0.80
    }

    pub fn is_critical(&self) -> bool {
        self.ratio() >= 0.95
    }

    pub fn is_exhausted(&self) -> bool {
        self.ratio() >= 1.0
    }
}

/// 压缩策略
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompactionStrategy {
    /// 无压缩
    None,
    /// 微压缩 (75% 触发): 裁剪最早 N 条消息
    Micro { keep_recent: usize },
    /// 自动压缩 (90% 触发): 摘要旧消息
    Auto { summary_target_tokens: TokenCount },
    /// 响应式压缩 (90%+): 剥离非文本块 + 摘要
    Reactive { strip_images: bool },
    /// 紧急折叠 (97%): 只保留摘要 + 最新一条
    Collapse { max_summary_words: usize },
    /// 截断: 保留首尾, 删除中间
    Snip { head_count: usize, tail_count: usize },
    /// 阶梯式截断 (Prompt Cache 优化): 一次删除 N 条，保持前缀稳定
    ///
    /// 优势: 接下来的 N-1 轮对话中，Messages 前缀保持完全稳定，Prompt Cache 命中率拉满。
    Chunked { chunk_size: usize, keep_recent: usize },
}

/// 压缩结果
#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub strategy: CompactionStrategy,
    pub messages_before: usize,
    pub messages_after: usize,
    pub tokens_freed: TokenCount,
    pub duration_ms: u64,
}

// ============================================================
//  上下文管理器
// ============================================================

/// 上下文状态
#[derive(Debug, Clone)]
pub enum ContextLevel {
    /// < 75%, 正常运行
    Healthy,
    /// 75%-90%, 可考虑微压缩
    Elevated,
    /// 90%-95%, 需要压缩
    Warning,
    /// 95%-97%, 强烈需要压缩
    Critical,
    /// > 97%, 紧急折叠
    Exhausted,
}

/// 上下文管理器 — 监控窗口 + 推荐压缩策略
pub struct ContextManager {
    pub max_tokens: TokenCount,
    pub compaction_count: u64,      // 总压缩次数
    pub consecutive_failures: u8,   // 断路器
    pub disabled: bool,             // 断路器是否打开
    pub disabled_until: Option<std::time::Instant>, // 断路器恢复时间
}

/// 断路器恢复等待时间
const CIRCUIT_BREAKER_RECOVERY_SECS: u64 = 60;

impl ContextManager {
    pub fn new(max_tokens: TokenCount) -> Self {
        Self {
            max_tokens,
            compaction_count: 0,
            consecutive_failures: 0,
            disabled: false,
            disabled_until: None,
        }
    }

    pub fn level(&self, used: TokenCount) -> ContextLevel {
        let ratio = used as f64 / self.max_tokens as f64;
        if ratio >= 0.97 {
            ContextLevel::Exhausted
        } else if ratio >= 0.95 {
            ContextLevel::Critical
        } else if ratio >= 0.90 {
            ContextLevel::Warning
        } else if ratio >= 0.75 {
            ContextLevel::Elevated
        } else {
            ContextLevel::Healthy
        }
    }

    /// 推荐最合适的压缩策略
    pub fn recommend_strategy(&self, used: TokenCount) -> CompactionStrategy {
        if self.disabled {
            // 检查是否到了恢复时间
            if let Some(recovery_time) = self.disabled_until {
                if std::time::Instant::now() >= recovery_time {
                    // 恢复时间已到，但这里不能 mutate self (immutable ref)
                    // 返回一个轻量策略让调用方知道可以重试
                    return CompactionStrategy::Micro { keep_recent: 10 };
                }
            }
            return CompactionStrategy::None;
        }

        match self.level(used) {
            ContextLevel::Healthy => CompactionStrategy::None,
            ContextLevel::Elevated => {
                CompactionStrategy::Micro { keep_recent: 10 }
            }
            ContextLevel::Warning => {
                CompactionStrategy::Auto { summary_target_tokens: 2048 }
            }
            ContextLevel::Critical => {
                CompactionStrategy::Reactive { strip_images: true }
            }
            ContextLevel::Exhausted => {
                CompactionStrategy::Collapse { max_summary_words: 500 }
            }
        }
    }

    /// 记录压缩结果 (含断路器逻辑)
    pub fn record_compaction(&mut self, success: bool) {
        self.compaction_count += 1;
        if success {
            self.consecutive_failures = 0;
            // 成功压缩后重置断路器
            if self.disabled {
                self.disabled = false;
                self.disabled_until = None;
            }
        } else {
            self.consecutive_failures += 1;
            if self.consecutive_failures >= 3 {
                self.disabled = true;
                // 设置恢复时间 (60秒后重试)
                self.disabled_until = Some(
                    std::time::Instant::now() + std::time::Duration::from_secs(CIRCUIT_BREAKER_RECOVERY_SECS)
                );
            }
        }
    }
}

// ============================================================
//  LLM 摘要压缩 (P3 实现)
// ============================================================

/// 压缩上下文 — 调用 LLM 生成摘要
///
/// 用于 Auto/Reactive/Collapse 策略
/// 将旧消息发送给 LLM 生成摘要，替换原始消息
pub async fn compact_context_with_llm(
    messages: &mut Vec<Message>,
    strategy: &CompactionStrategy,
    provider: &dyn crate::core::provider::Provider,
    model: &str,
) -> Option<CompactionResult> {
    use crate::core::provider::{ContentBlock, Message as Msg, Role};
    use std::time::Instant;

    let start = Instant::now();
    let before = messages.len();
    if before <= 2 { return None; }

    match strategy {
        CompactionStrategy::None => return None,
        CompactionStrategy::Micro { keep_recent } => {
            if before > *keep_recent {
                let drain_count = before - *keep_recent;
                messages.drain(1..=drain_count);
            }
        }
        CompactionStrategy::Chunked { chunk_size, keep_recent } => {
            // 阶梯式截断: 一次删除 chunk_size 条消息，保持前缀稳定
            // 优势: 接下来的 chunk_size-1 轮对话中，Messages 前缀保持完全稳定
            if before > *keep_recent + chunk_size {
                let drain_count = *chunk_size;
                messages.drain(1..=drain_count);
            } else if before > *keep_recent {
                let drain_count = before - *keep_recent;
                messages.drain(1..=drain_count);
            }
        }
        CompactionStrategy::Snip { head_count, tail_count } => {
            if before > head_count + tail_count {
                let mut kept: Vec<Msg> = Vec::new();
                let h = (*head_count).min(before);
                for m in messages.drain(..h) { kept.push(m); }
                let remaining: Vec<Msg> = messages.drain(..).collect();
                let skip = remaining.len().saturating_sub(*tail_count);
                for m in remaining.into_iter().skip(skip) { kept.push(m); }
                *messages = kept;
            }
        }
        CompactionStrategy::Auto { summary_target_tokens } => {
            // 提取消息文本
            let conversation = extract_conversation_text(messages);
            let prompt = format!(
                "Summarize the following conversation in under {} tokens. \
                 Preserve all key decisions, file paths, and technical details:\n\n{}",
                summary_target_tokens, conversation
            );
            if let Some(summary) = call_llm_summary(provider, model, &prompt).await {
                // 压缩后只保留一个 User 消息，确保格式正确
                messages.clear();
                messages.push(Msg {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: format!(
                            "[Previous conversation summary]\n{}\n\nPlease continue the work based on this summary.",
                            summary
                        ),
                    }],
                    reasoning_content: None,
                    cache_breakpoint: false,
                });
            }
        }
        CompactionStrategy::Reactive { strip_images } => {
            if *strip_images {
                for msg in messages.iter_mut() {
                    msg.content.retain(|b| !matches!(b, ContentBlock::Image { .. }));
                }
            }
            let conversation = extract_conversation_text(messages);
            let prompt = format!(
                "Condense this conversation. Keep file paths, code decisions, and errors:\n\n{}",
                conversation
            );
            if let Some(summary) = call_llm_summary(provider, model, &prompt).await {
                messages.clear();
                messages.push(Msg {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: format!("[Previous conversation summary]\n{}\n\nPlease continue.", summary),
                    }],
                    reasoning_content: None,
                    cache_breakpoint: false,
                });
            }
        }
        CompactionStrategy::Collapse { max_summary_words } => {
            let conversation = extract_conversation_text(messages);
            let prompt = format!(
                "Provide an extremely concise summary (max {} words). Only keep: \
                 what was asked, what was done, current state:\n\n{}",
                max_summary_words, conversation
            );
            if let Some(summary) = call_llm_summary(provider, model, &prompt).await {
                messages.clear();
                messages.push(Msg {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: format!("[Collapsed Summary]\n{}\n\nPlease continue.", summary),
                    }],
                    reasoning_content: None,
                    cache_breakpoint: false,
                });
            }
        }
    }

    // 清理消息格式: 确保没有孤立的 Tool 消息，且消息角色交替正确
    sanitize_messages(messages);

    let after = messages.len();
    let tokens_freed = ((before - after) * 100) as u64; // 估算
    Some(CompactionResult {
        strategy: *strategy,
        messages_before: before,
        messages_after: after,
        tokens_freed,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// 清理消息格式: 确保没有孤立的 Tool 消息，且消息角色交替正确
fn sanitize_messages(messages: &mut Vec<Message>) {
    use crate::core::provider::Role;
    
    // 1. 移除孤立的 Tool 消息 (没有对应的 Assistant tool_use)
    let mut has_pending_tool_use = false;
    messages.retain(|m| {
        match m.role {
            Role::Assistant => {
                has_pending_tool_use = m.content.iter().any(|b| matches!(b, crate::core::provider::ContentBlock::ToolUse { .. }));
                true
            }
            Role::Tool => {
                if has_pending_tool_use {
                    has_pending_tool_use = false;
                    true
                } else {
                    false // 孤立的 Tool 消息，移除
                }
            }
            _ => true,
        }
    });
    
    // 2. 确保消息角色交替 (不允许连续的同角色消息)
    let mut cleaned: Vec<Message> = Vec::new();
    for msg in messages.drain(..) {
        if let Some(last) = cleaned.last() {
            if last.role == msg.role && msg.role != Role::Tool {
                // 连续同角色消息，合并内容和 reasoning_content
                if let Some(last_mut) = cleaned.last_mut() {
                    for block in msg.content {
                        last_mut.content.push(block);
                    }
                    // 合并 reasoning_content
                    if let Some(new_reasoning) = msg.reasoning_content {
                        match last_mut.reasoning_content.as_mut() {
                            Some(existing) => {
                                existing.push('\n');
                                existing.push_str(&new_reasoning);
                            }
                            None => { last_mut.reasoning_content = Some(new_reasoning); }
                        }
                    }
                    continue;
                }
            }
        }
        cleaned.push(msg);
    }
    *messages = cleaned;
}

/// 从消息列表中提取对话文本
fn extract_conversation_text(messages: &[Message]) -> String {
    use crate::core::provider::ContentBlock;
    let mut parts = Vec::new();
    for msg in messages {
        let role = match msg.role {
            crate::core::provider::Role::User => "User",
            crate::core::provider::Role::Assistant => "Assistant",
            crate::core::provider::Role::Tool => "Tool",
            crate::core::provider::Role::System => "System",
        };
        // 提取 reasoning_content
        if let Some(ref reasoning) = msg.reasoning_content {
            if !reasoning.is_empty() {
                parts.push(format!("[{} Thinking]: {}", role, &reasoning[..reasoning.len().min(TRUNCATE_LEN)]));
            }
        }
        for block in &msg.content {
            match block {
                ContentBlock::Text { text } => parts.push(format!("[{}]: {}", role, text)),
                ContentBlock::ToolUse { tool_name, input, .. } => {
                    parts.push(format!("[{}]: Tool call: {}({})", role, tool_name, input));
                }
                ContentBlock::ToolResult { tool_name, content, is_error, .. } => {
                    let prefix = if *is_error { "ERROR" } else { "OK" };
                    parts.push(format!("[{}]: Tool result [{}] {}: {}", role, prefix, tool_name, &content[..content.len().min(TRUNCATE_LEN)]));
                }
                ContentBlock::Thinking { text } => {
                    parts.push(format!("[{} Thinking]: {}", role, &text[..text.len().min(TRUNCATE_LEN)]));
                }
                _ => {}
            }
        }
    }
    parts.join("\n")
}

/// 调用 LLM 生成摘要
async fn call_llm_summary(
    provider: &dyn crate::core::provider::Provider,
    model: &str,
    prompt: &str,
) -> Option<String> {
    use crate::core::provider::{ContentBlock, Message as Msg, ProviderRequest, Role};

    let req = ProviderRequest {
        model: model.to_string(),
        messages: vec![Msg {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt.to_string() }],
            reasoning_content: None,
            cache_breakpoint: false,
        }],
        system_prompt: Some("You are a conversation summarizer. Be concise and precise.".to_string()),
        max_tokens: Some(2048),
        temperature: Some(0.3),
        stream: false,
        tools: None,
        thinking: Some(serde_json::json!({"type": "disabled"})),
        reasoning_effort: None,
            enable_prompt_cache: None,
            cache_key: None,
    };

    match provider.complete(req).await {
        Ok(resp) => {
            let text: String = resp.message.content.iter()
                .filter_map(|b| if let ContentBlock::Text { text } = b { Some(text.as_str()) } else { None })
                .collect();
            if text.is_empty() { None } else { Some(text) }
        }
        Err(e) => {
            tracing::warn!("LLM compaction failed: {}", e);
            None
        }
    }
}
