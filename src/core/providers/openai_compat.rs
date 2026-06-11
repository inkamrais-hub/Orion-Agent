//! OpenAI 兼容 Provider (支持函数调用 + SSE 流式 + Thinking)
//!
//! 支持: DeepSeek, OpenAI, vLLM, Ollama
//!
//! 环境变量:
//!   LLM_API_BASE   - API 地址 (默认: https://api.deepseek.com)
//!   LLM_API_KEY    - API Key
//!   LLM_MODEL      - 模型名 (默认: deepseek-v4-flash)

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::core::provider::{
    ContentBlock, Message, Provider, ProviderRequest, ProviderResponse, Role, StreamEvent, UsageInfo,
};
use crate::Error;

pub struct OpenAICompatProvider {
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl OpenAICompatProvider {
    pub fn from_env() -> crate::Result<Self> {
        let api_key = std::env::var("LLM_API_KEY")
            .map_err(|_| crate::Error::Config("LLM_API_KEY must be set in .env or environment".into()))?;
        let api_base = std::env::var("LLM_API_BASE")
            .unwrap_or_else(|_| "https://api.deepseek.com".into());
        let model = std::env::var("LLM_MODEL")
            .unwrap_or_else(|_| "deepseek-v4-flash".into());
        Ok(Self::new(&api_base, &api_key, &model))
    }

    pub fn new(api_base: &str, api_key: &str, model: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client, api_base: api_base.trim_end_matches('/').to_string(), api_key: api_key.to_string(), model: model.to_string() }
    }

    fn to_openai_messages(messages: &[Message]) -> Vec<OpenAIMessage> {
        messages.iter().map(|msg| {
            let role = match msg.role {
                Role::User => "user", Role::Assistant => "assistant",
                Role::Tool => "tool", Role::System => "system",
            };
            let mut out = OpenAIMessage {
                role: role.to_string(), content: None,
                tool_calls: None, tool_call_id: None, name: None,
                reasoning_content: None,
            };
            // 回传 reasoning_content (DeepSeek/Qwen/Kimi 思考模式必须)
            if msg.reasoning_content.is_some() {
                out.reasoning_content = msg.reasoning_content.clone();
            }

            // 检测是否包含图片 (多模态消息)
            let has_image = msg.content.iter().any(|b| matches!(b, ContentBlock::Image { .. }));

            if has_image {
                // 多模态消息: content 使用数组格式
                let mut parts: Vec<serde_json::Value> = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            parts.push(serde_json::json!({"type": "text", "text": text}));
                        }
                        ContentBlock::Image { data, media_type } => {
                            // data 已经是 Base64 字符串
                            let url = format!("data:{};base64,{}", media_type, data);
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {"url": url}
                            }));
                        }
                        _ => {}
                    }
                }
                out.content = Some(serde_json::Value::Array(parts));
            } else {
                // 普通消息: 逐块处理
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => out.content = Some(serde_json::Value::String(text.clone())),
                        ContentBlock::ToolUse { tool_name, input, tool_call_id } => {
                            out.role = "assistant".to_string();
                            out.tool_calls.get_or_insert_with(Vec::new).push(OpenAIToolCall {
                                id: tool_call_id.clone(),
                                type_field: "function".to_string(),
                                function: OpenAIFunctionCall {
                                    name: tool_name.clone(),
                                    arguments: serde_json::to_string(input).unwrap_or_default(),
                                },
                            });
                        }
                        ContentBlock::ToolResult { tool_name, content, tool_call_id, .. } => {
                            out.role = "tool".to_string();
                            out.content = Some(serde_json::Value::String(content.clone()));
                            out.tool_call_id = Some(tool_call_id.clone());
                            out.name = Some(tool_name.clone());
                        }
                        ContentBlock::Image { .. } => {
                            // 已在 has_image 分支处理
                        }
                        ContentBlock::Thinking { text } => {
                            if out.reasoning_content.is_none() {
                                out.reasoning_content = Some(text.clone());
                            }
                        }
                    }
                }
            }
            out
        }).collect()
    }

    fn to_openai_tools(tools: &[serde_json::Value]) -> Vec<OpenAITool> {
        tools.iter().map(|t| OpenAITool {
            type_field: "function".to_string(),
            function: OpenAIToolFunction {
                name: t["name"].as_str().unwrap_or("unknown").to_string(),
                description: t["description"].as_str().unwrap_or("").to_string(),
                parameters: t["input_schema"].clone(),
            },
        }).collect()
    }

    fn gen_tool_id() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        format!("call_{}", COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

#[async_trait]
impl Provider for OpenAICompatProvider {
    fn name(&self) -> &str { "openai-compat" }
    fn supported_models(&self) -> Vec<&str> { vec![&self.model] }

    async fn complete(&self, req: ProviderRequest) -> crate::Result<ProviderResponse> {
        let mut messages = Self::to_openai_messages(&req.messages);

        // 注入 system prompt 为 system 消息
        if let Some(ref system) = req.system_prompt {
            if !system.is_empty() {
                messages.insert(0, OpenAIMessage {
                    role: "system".to_string(),
                    content: Some(serde_json::Value::String(system.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                });
            }
        }

        let tools = req.tools.as_ref().map(|t| Self::to_openai_tools(t));

        // reasoning_effort only in thinking mode
        let thinking = req.thinking.clone();
        let reasoning_effort = if thinking.is_some() { req.reasoning_effort.clone() } else { None };

        // prompt caching (DeepSeek: automatic prefix cache; explicit hint for clarity)
        let cache = if req.enable_prompt_cache.unwrap_or(false) {
            Some(serde_json::json!({"type": "prompt_cache", "value": true}))
        } else { None };

        let body = ChatCompletionRequest {
            model: self.model.clone(), messages, tools,
            max_tokens: req.max_tokens, temperature: req.temperature,
            stream: false, thinking, reasoning_effort, cache,
        };

        let url = format!("{}/v1/chat/completions", self.api_base);
        let resp = self.client.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body).send().await
            .map_err(|e| Error::Provider(format!("Request: {}", e)))?;
        if !resp.status().is_success() {
            return Err(Error::Provider(format!("API error ({}): {}", resp.status(), resp.text().await.unwrap_or_default())));
        }

        let chat: ChatCompletionResponse = resp.json().await.map_err(|e| Error::Provider(format!("Parse: {}", e)))?;
        let choice = chat.choices.into_iter().next().ok_or_else(|| Error::Provider("No choices".into()))?;

        let mut blocks = Vec::new();
        if let Some(text) = &choice.message.content {
            if !text.is_empty() {
                blocks.push(ContentBlock::Text { text: text.clone() });
            }
        }
        if let Some(tcs) = choice.message.tool_calls {
            for tc in tcs {
                let input = serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| serde_json::json!({}));
                blocks.push(ContentBlock::ToolUse {
                    tool_name: tc.function.name,
                    input,
                    tool_call_id: tc.id,
                });
            }
        }

        // 构建 Message，保留 reasoning_content 供后续回传
        let mut msg = Message::new(Role::Assistant, blocks);
        msg.reasoning_content = choice.message.reasoning_content;

        Ok(ProviderResponse {
            message: msg,
            usage: UsageInfo {
                input_tokens: chat.usage.prompt_tokens as u64,
                output_tokens: chat.usage.completion_tokens as u64,
                cache_creation_tokens: chat.usage.prompt_cache_hit_tokens.unwrap_or(0) as u64,
                cache_read_tokens: 0,
            },
        })
    }

    async fn stream(&self, req: ProviderRequest, tx: mpsc::UnboundedSender<StreamEvent>) -> crate::Result<()> {
        let mut messages = Self::to_openai_messages(&req.messages);

        // 注入 system prompt
        if let Some(ref system) = req.system_prompt {
            if !system.is_empty() {
                messages.insert(0, OpenAIMessage {
                    role: "system".to_string(),
                    content: Some(serde_json::Value::String(system.clone())),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_content: None,
                });
            }
        }

        let tools = req.tools.as_ref().map(|t| Self::to_openai_tools(t));

        let thinking = req.thinking.clone();
        let reasoning_effort = if thinking.is_some() { req.reasoning_effort.clone() } else { None };

        let cache = if req.enable_prompt_cache.unwrap_or(false) {
            Some(serde_json::json!({"type": "prompt_cache", "value": true}))
        } else { None };

        let body = ChatCompletionRequest {
            model: self.model.clone(), messages, tools,
            max_tokens: req.max_tokens, temperature: req.temperature,
            stream: true, thinking, reasoning_effort, cache,
        };

        let url = format!("{}/v1/chat/completions", self.api_base);
        let resp = self.client.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&body).send().await
            .map_err(|e| {
                let detail = format!("{:?}", e); // 使用 Debug 格式显示完整错误链
                Error::Provider(format!("Stream req: {}", detail))
            })?;
        if !resp.status().is_success() {
            let _ = tx.send(StreamEvent::Error { message: format!("API error ({})", resp.status()) });
            return Ok(());
        }

        let mut byte_stream = resp.bytes_stream();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut buf = String::new();
            let mut acc_tools: Option<Vec<AccumToolCall>> = None;
            let mut last_usage: Option<UsageInfo> = None;
            while let Some(chunk) = byte_stream.next().await {
                let Ok(bytes) = chunk else {
                    let _ = tx2.send(StreamEvent::Error { message: "Stream read error".into() });
                    return;
                };
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(nl) = buf.find('\n') {
                    let line = buf[..nl].trim().to_string();
                    buf = buf[nl + 1..].to_string();
                    if line.is_empty() || line.starts_with(':') { continue; }
                    let data = match line.strip_prefix("data: ") { Some(d) => d, _ => continue };
                    if data == "[DONE]" { let _ = tx2.send(StreamEvent::Done { usage: last_usage.take().unwrap_or_default() }); return; }
                    let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) else { continue; };
                    if let Some(u) = chunk.usage {
                        last_usage = Some(UsageInfo {
                            input_tokens: u.prompt_tokens as u64,
                            output_tokens: u.completion_tokens as u64,
                            cache_creation_tokens: u.prompt_cache_hit_tokens.unwrap_or(0) as u64,
                            cache_read_tokens: 0,
                        });
                    }
                    let Some(choice) = chunk.choices.into_iter().next() else { continue; };
                    let d = choice.delta;
                    if let Some(ref t) = d.reasoning_content { if !t.is_empty() { let _ = tx2.send(StreamEvent::Thinking { delta: t.clone() }); } }
                    if let Some(ref t) = d.content { if !t.is_empty() { let _ = tx2.send(StreamEvent::Text { delta: t.clone() }); } }
                    if let Some(tcs) = d.tool_calls {
                        for tc in tcs {
                            let acc = acc_tools.get_or_insert_with(Vec::new);
                            if let Some(idx) = tc.index {
                                while acc.len() <= idx as usize { acc.push(AccumToolCall::default()); }
                                if let Some(id) = tc.id { acc[idx as usize].id = id; }
                                if let Some(name) = tc.function.as_ref().and_then(|f| f.name.clone()) { acc[idx as usize].name = name; }
                                if let Some(args) = tc.function.and_then(|f| f.arguments) { acc[idx as usize].arguments.push_str(&args); }
                            }
                        }
                    }
                    if let Some(ref reason) = choice.finish_reason {
                        if reason == "tool_calls" {
                            if let Some(calls) = acc_tools.take() {
                                for tc in calls {
                                    let input = serde_json::from_str(&tc.arguments).unwrap_or_default();
                                    let id = if tc.id.is_empty() { Self::gen_tool_id() } else { tc.id.clone() };
                                    let _ = tx2.send(StreamEvent::ToolUse { tool_name: tc.name, input, tool_call_id: id });
                                }
                            }
                        }
                        let _ = tx2.send(StreamEvent::Done { usage: last_usage.take().unwrap_or_default() });
                        return;
                    }
                }
            }
            let _ = tx2.send(StreamEvent::Done { usage: last_usage.take().unwrap_or_default() });
        });
        Ok(())
    }
}

// ============================================================
//  类型定义
// ============================================================

#[derive(Debug, Default)]
struct AccumToolCall { id: String, name: String, arguments: String }

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String, messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")] tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")] max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] temperature: Option<f64>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")] thinking: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")] reasoning_effort: Option<String>,
    /// DeepSeek prompt caching hint
    #[serde(skip_serializing_if = "Option::is_none")]
    cache: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse { choices: Vec<Choice>, usage: Usage }
#[derive(Debug, Deserialize)]
struct Choice { message: ResponseMessage, #[allow(dead_code)] finish_reason: Option<String> }
#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    #[serde(default)] tool_calls: Option<Vec<ResponseToolCall>>,
    #[serde(default)] reasoning_content: Option<String>,
}
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseToolCall { id: String, #[serde(rename = "type")] type_field: String, function: ResponseFunction }
#[derive(Debug, Deserialize)]
struct ResponseFunction { name: String, arguments: String }
#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    /// 支持 String (普通消息) 或 Array (多模态消息含图片)
    #[serde(skip_serializing_if = "Option::is_none")] content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")] tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")] tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] reasoning_content: Option<String>,
}
#[derive(Debug, Serialize)]
struct OpenAIToolCall { id: String, #[serde(rename = "type")] type_field: String, function: OpenAIFunctionCall }
#[derive(Debug, Serialize)]
struct OpenAIFunctionCall { name: String, arguments: String }
#[derive(Debug, Serialize)]
struct OpenAITool { #[serde(rename = "type")] type_field: String, function: OpenAIToolFunction }
#[derive(Debug, Serialize)]
struct OpenAIToolFunction { name: String, description: String, parameters: serde_json::Value }
#[derive(Debug, Deserialize)]
struct Usage { prompt_tokens: u32, completion_tokens: u32, #[serde(default)] prompt_cache_hit_tokens: Option<u32> }

// 流式 SSE 类型
#[derive(Debug, Deserialize)]
struct StreamChunk { choices: Vec<StreamChoice>, usage: Option<Usage> }
#[derive(Debug, Deserialize)]
struct StreamChoice { delta: StreamDelta, #[allow(dead_code)] finish_reason: Option<String>, #[allow(dead_code)] index: u32 }
#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)] content: Option<String>,
    #[serde(default)] reasoning_content: Option<String>,
    #[serde(default)] tool_calls: Option<Vec<DeltaToolCall>>,
}
#[derive(Debug, Deserialize)]
struct DeltaToolCall {
    index: Option<u32>, #[serde(default)] id: Option<String>,
    #[serde(rename = "type")] #[allow(dead_code)] type_field: Option<String>,
    #[serde(default)] function: Option<DeltaFunction>,
}
#[derive(Debug, Deserialize)]
struct DeltaFunction { #[serde(default)] name: Option<String>, #[serde(default)] arguments: Option<String> }
