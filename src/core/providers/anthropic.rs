//! Anthropic Provider (Claude API)
//!
//! 支持: Claude 3/4 系列模型, 函数调用, SSE 流式, Thinking
//! 也可用于 DeepSeek 的 Anthropic 兼容端点: https://api.deepseek.com/anthropic
//!
//! 环境变量:
//!   ANTHROPIC_API_KEY  - API Key
//!   ANTHROPIC_API_BASE - API 地址 (默认: https://api.anthropic.com)
//!   ANTHROPIC_MODEL    - 模型名 (默认: claude-sonnet-4-20250514)

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::core::provider::{
    ContentBlock, Message, Provider, ProviderRequest, ProviderResponse, Role, StreamEvent, UsageInfo,
};
use crate::Error;

pub struct AnthropicProvider {
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn from_env() -> crate::Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .or_else(|_| std::env::var("LLM_API_KEY"))
            .map_err(|_| crate::Error::Config("ANTHROPIC_API_KEY or LLM_API_KEY must be set".into()))?;
        let api_base = std::env::var("ANTHROPIC_API_BASE")
            .unwrap_or_else(|_| "https://api.anthropic.com".into());
        let model = std::env::var("ANTHROPIC_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
        Ok(Self::new(&api_base, &api_key, &model))
    }

    pub fn new(api_base: &str, api_key: &str, model: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            api_base: api_base.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    /// 框架 Message → Anthropic 格式
    fn to_anthropic_messages(messages: &[Message]) -> Vec<AnthropicMessage> {
        let mut result: Vec<AnthropicMessage> = Vec::new();
        for msg in messages {
            match msg.role {
                Role::System => continue, // system 单独处理
                _ => {
                    let mut blocks = Vec::new();
                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => {
                                blocks.push(AnthropicContentBlock::Text {
                                    text: text.clone(),
                                    cache_control: None,
                                });
                            }
                            ContentBlock::ToolUse { tool_name, input, tool_call_id } => {
                                blocks.push(AnthropicContentBlock::ToolUse {
                                    type_field: "tool_use".to_string(),
                                    id: tool_call_id.clone(),
                                    name: tool_name.clone(),
                                    input: input.clone(),
                                });
                            }
                            ContentBlock::ToolResult { content, tool_call_id, is_error, .. } => {
                                blocks.push(AnthropicContentBlock::ToolResult {
                                    type_field: "tool_result".to_string(),
                                    tool_use_id: tool_call_id.clone(),
                                    content: vec![AnthropicContentBlock::TextContent {
                                        text: content.clone(),
                                    }],
                                    is_error: *is_error,
                                });
                            }
                            ContentBlock::Image { data, media_type } => {
                                // data 已经是 Base64 字符串
                                blocks.push(AnthropicContentBlock::Image {
                                    type_field: "image".to_string(),
                                    source: AnthropicImageSource {
                                        type_field: "base64".to_string(),
                                        media_type: media_type.clone(),
                                        data: data.clone(),
                                    },
                                });
                            }
                            ContentBlock::Thinking { text } => {
                                blocks.push(AnthropicContentBlock::Text {
                                    text: format!("[Thinking: {}]", text),
                                    cache_control: None,
                                });
                            }
                        }
                    }
                    // 如果消息标记了 cache_breakpoint, 给最后一个 Text 内容块添加 cache_control
                    if msg.cache_breakpoint {
                        if let Some(AnthropicContentBlock::Text { cache_control, .. }) = blocks.last_mut() {
                            *cache_control = Some(CacheControl { type_field: "ephemeral".to_string() });
                        }
                    }

                    let role_str = match msg.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        _ => "user",
                    };
                    result.push(AnthropicMessage {
                        role: role_str.to_string(),
                        content: blocks,
                    });
                }
            }
        }
        result
    }

    /// 框架工具定义 → Anthropic tools 格式
    fn to_anthropic_tools(tools: &[serde_json::Value]) -> Vec<AnthropicTool> {
        tools.iter().map(|t| AnthropicTool {
            name: t["name"].as_str().unwrap_or("unknown").to_string(),
            description: t["description"].as_str().unwrap_or("").to_string(),
            input_schema: t["input_schema"].clone(),
            cache_control: None,
        }).collect()
    }

    /// 解析 Anthropic 响应内容块 → 框架 ContentBlock
    fn parse_content_blocks(blocks: &[AnthropicContentBlock]) -> Vec<ContentBlock> {
        let mut result = Vec::new();
        for block in blocks {
            match block {
                AnthropicContentBlock::Text { text, .. } | AnthropicContentBlock::TextContent { text } => {
                    result.push(ContentBlock::Text { text: text.clone() });
                }
                AnthropicContentBlock::ToolUse { id, name, input, .. } => {
                    result.push(ContentBlock::ToolUse {
                        tool_name: name.clone(),
                        input: input.clone(),
                        tool_call_id: id.clone(),
                    });
                }
                AnthropicContentBlock::ToolResult { .. } => {
                    // ToolResult 只在发送给 API 时使用
                }
                AnthropicContentBlock::Image { .. } => {
                    // 图片在响应中不直接返回文本内容
                }
            }
        }
        result
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str { "anthropic" }
    fn supported_models(&self) -> Vec<&str> { vec![&self.model] }

    async fn complete(&self, req: ProviderRequest) -> crate::Result<ProviderResponse> {
        let system = req.system_prompt.as_deref().unwrap_or("");
        let cache_enabled = req.enable_prompt_cache.unwrap_or(false);

        let an_messages = Self::to_anthropic_messages(&req.messages);
        let tools = req.tools.as_ref().map(|t| Self::to_anthropic_tools(t));

        // Prompt caching: 对 system 和 tools 添加 cache_control
        let system_value = if !system.is_empty() {
            if cache_enabled {
                Some(serde_json::json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": {"type": "ephemeral"}
                }]))
            } else {
                Some(serde_json::json!(system))
            }
        } else { None };

        let tools_cached = if cache_enabled {
            tools.map(|ts| ts.into_iter().enumerate().map(|(i, mut t)| {
                if i == 0 { t.cache_control = Some(CacheControl { type_field: "ephemeral".to_string() }); }
                t
            }).collect())
        } else { tools };

        let body = AnthropicRequest {
            model: self.model.clone(),
            messages: an_messages,
            system: system_value,
            tools: tools_cached,
            max_tokens: req.max_tokens.unwrap_or(4096),
            stream: false,
        };

        let url = format!("{}/v1/messages", self.api_base);
        let resp = self.client.post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body).send().await
            .map_err(|e| Error::Provider(format!("Request: {}", e)))?;

        if !resp.status().is_success() {
            return Err(Error::Provider(format!(
                "API error ({}): {}", resp.status(), resp.text().await.unwrap_or_default()
            )));
        }

        let chat: AnthropicResponse = resp.json().await
            .map_err(|e| Error::Provider(format!("Parse: {}", e)))?;

        let blocks = Self::parse_content_blocks(&chat.content);

        Ok(ProviderResponse {
            message: Message::new(Role::Assistant, blocks),
            usage: UsageInfo {
                input_tokens: chat.usage.input_tokens as u64,
                output_tokens: chat.usage.output_tokens as u64,
                cache_creation_tokens: chat.usage.cache_creation_input_tokens.unwrap_or(0) as u64,
                cache_read_tokens: chat.usage.cache_read_input_tokens.unwrap_or(0) as u64,
            },
        })
    }

    async fn stream(&self, req: ProviderRequest, tx: mpsc::UnboundedSender<StreamEvent>) -> crate::Result<()> {
        let system = req.system_prompt.as_deref().unwrap_or("");
        let cache_enabled = req.enable_prompt_cache.unwrap_or(false);
        let an_messages = Self::to_anthropic_messages(&req.messages);
        let tools = req.tools.as_ref().map(|t| Self::to_anthropic_tools(t));

        // Prompt caching: 对 system 和 tools 添加 cache_control
        let system_value = if !system.is_empty() {
            if cache_enabled {
                Some(serde_json::json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": {"type": "ephemeral"}
                }]))
            } else {
                Some(serde_json::json!(system))
            }
        } else { None };

        let tools_cached = if cache_enabled {
            tools.map(|ts| ts.into_iter().enumerate().map(|(i, mut t)| {
                if i == 0 { t.cache_control = Some(CacheControl { type_field: "ephemeral".to_string() }); }
                t
            }).collect())
        } else { tools };

        let body = AnthropicRequest {
            model: self.model.clone(),
            messages: an_messages,
            system: system_value,
            tools: tools_cached,
            max_tokens: req.max_tokens.unwrap_or(4096),
            stream: true,
        };

        let url = format!("{}/v1/messages", self.api_base);
        let resp = self.client.post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body).send().await
            .map_err(|e| Error::Provider(format!("Stream req: {}", e)))?;

        if !resp.status().is_success() {
            let _ = tx.send(StreamEvent::Error {
                message: format!("API error ({})", resp.status()),
            });
            return Ok(());
        }

        let mut byte_stream = resp.bytes_stream();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut buf = String::new();
            let mut current_tool: Option<(String, String, String)> = None; // (id, name, args)
            let mut last_usage = UsageInfo::default();

            while let Some(chunk) = byte_stream.next().await {
                let Ok(bytes) = chunk else {
                    let _ = tx2.send(StreamEvent::Error { message: "Stream read error".into() });
                    return;
                };
                buf.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(nl) = buf.find('\n') {
                    let line = buf[..nl].trim().to_string();
                    buf = buf[nl + 1..].to_string();
                    if line.is_empty() || !line.starts_with("event:") { continue; }

                    // Anthropic SSE 格式: event: XXX \n data: {...}
                    let event_type = line.strip_prefix("event:").map(|s| s.trim().to_string());
                    // 读取下一行 data
                    let data_line = if let Some(dnl) = buf.find('\n') {
                        let d = buf[..dnl].trim().to_string();
                        buf = buf[dnl + 1..].to_string();
                        d
                    } else { continue; };
                    let data = match data_line.strip_prefix("data: ") {
                        Some(d) => d.to_string(),
                        _ => continue,
                    };

                    match event_type.as_deref() {
                        Some("message_start") => {
                            if let Ok(parsed) = serde_json::from_str::<AnthropicMessageStartData>(&data) {
                                if let Some(u) = parsed.message.usage {
                                    last_usage.input_tokens = u.input_tokens.unwrap_or(0) as u64;
                                    last_usage.cache_creation_tokens = u.cache_creation_input_tokens.unwrap_or(0) as u64;
                                    last_usage.cache_read_tokens = u.cache_read_input_tokens.unwrap_or(0) as u64;
                                }
                            }
                        }
                        Some("content_block_start") | Some("content_block_delta") => {
                            if let Ok(parsed) = serde_json::from_str::<AnthropicStreamDelta>(&data) {
                                if let Some(text) = &parsed.delta.text {
                                    let _ = tx2.send(StreamEvent::Text { delta: text.clone() });
                                }
                                if let Some(thinking) = &parsed.delta.thinking {
                                    let _ = tx2.send(StreamEvent::Thinking { delta: thinking.clone() });
                                }
                                // 工具调用开始
                                if let Some(tool_use) = &parsed.content_block {
                                    if tool_use.type_field == "tool_use" {
                                        current_tool = Some((
                                            tool_use.id.clone().unwrap_or_default(),
                                            tool_use.name.clone().unwrap_or_default(),
                                            String::new(),
                                        ));
                                    }
                                }
                                // 工具参数增量
                                if let Some(partial) = &parsed.delta.partial_json {
                                    if let Some((ref id, ref name, ref args)) = current_tool {
                                        let new_args = format!("{}{}", args, partial);
                                        current_tool = Some((id.clone(), name.clone(), new_args));
                                    }
                                }
                            }
                        }
                        Some("content_block_stop") => {
                            // Flush 当前工具
                            if let Some((id, name, args)) = current_tool.take() {
                                let input = serde_json::from_str(&args).unwrap_or_default();
                                let _ = tx2.send(StreamEvent::ToolUse {
                                    tool_name: name, input, tool_call_id: id,
                                });
                            }
                        }
                        Some("message_delta") => {
                            if let Ok(parsed) = serde_json::from_str::<AnthropicMessageDelta>(&data) {
                                if let Some(u) = parsed.usage {
                                    last_usage.output_tokens = u.output_tokens.unwrap_or(0) as u64;
                                }
                                if parsed.delta.stop_reason.as_deref() == Some("tool_use") {
                                    // Flush
                                    if let Some((id, name, args)) = current_tool.take() {
                                        let input = serde_json::from_str(&args).unwrap_or_default();
                                        let _ = tx2.send(StreamEvent::ToolUse {
                                            tool_name: name, input, tool_call_id: id,
                                        });
                                    }
                                }
                            }
                        }
                        Some("message_stop") => {
                            let _ = tx2.send(StreamEvent::Done { usage: last_usage });
                            return;
                        }
                        _ => {}
                    }
                }
            }
            let _ = tx2.send(StreamEvent::Done { usage: last_usage });
        });
        Ok(())
    }
}

// ============================================================
//  Anthropic API 类型定义
// ============================================================

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    /// system prompt — string or content blocks array
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    max_tokens: u64,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheControl {
    #[serde(rename = "type")]
    type_field: String,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        cache_control: Option<CacheControl>,
    },
    #[allow(dead_code)]
    TextContent {
        text: String,
    },
    ToolUse {
        #[serde(rename = "type")]
        type_field: String,
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        type_field: String,
        tool_use_id: String,
        content: Vec<AnthropicContentBlock>,
        is_error: bool,
    },
    Image {
        #[serde(rename = "type")]
        type_field: String,
        source: AnthropicImageSource,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    type_field: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

// 流式类型
#[derive(Debug, Deserialize)]
struct AnthropicStreamDelta {
    #[serde(default)]
    delta: StreamDeltaContent,
    #[serde(default)]
    content_block: Option<StreamContentBlock>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDeltaContent {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamContentBlock {
    #[serde(rename = "type")]
    type_field: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    delta: MessageDeltaContent,
    #[serde(default)]
    usage: Option<AnthropicStreamUsage>,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaContent {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStartData {
    message: AnthropicMessageStart,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStart {
    #[serde(default)]
    usage: Option<AnthropicStreamUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct AnthropicStreamUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}
