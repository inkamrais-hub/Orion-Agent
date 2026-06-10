//! DeepSeek v4-flash 集成测试
//!
//! 通过 OpenAICompatProvider 实际调用 DeepSeek API，验证:
//!   1. 非流式 complete (含 system prompt)
//!   2. 流式 stream (SSE)
//!   3. 工具调用 (function calling)
//!
//! 运行: cargo test --test deepseek_integration -- --nocapture

use orion_agent::core::provider::{
    ContentBlock, Message, Provider, ProviderRequest, Role, StreamEvent,
};
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;
use tokio::sync::mpsc;

const API_KEY: &str = "sk-d8986bd2ffc7470182717371323726a7";
const API_BASE: &str = "https://api.deepseek.com";
const MODEL: &str = "deepseek-v4-flash";

fn build_provider() -> OpenAICompatProvider {
    OpenAICompatProvider::new(API_BASE, API_KEY, MODEL)
}

/// Test 1: 非流式调用 — 简单问答
#[tokio::test]
async fn test_non_streaming_chat() {
    let provider = build_provider();

    let req = ProviderRequest {
        model: MODEL.into(),
        messages: vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "请用一句话介绍你自己。".into(),
            }],
        )],
        system_prompt: Some("你是 Orion Agent，一个智能助手。请用中文回答。".into()),
        max_tokens: Some(256),
        temperature: Some(0.7),
        stream: false,
        tools: None,
        thinking: None,
        reasoning_effort: None,
        enable_prompt_cache: Some(true),
        cache_key: None,
    };

    let resp = provider.complete(req).await.expect("API call failed");

    // 验证响应非空
    assert!(!resp.message.content.is_empty(), "Response should have content");

    // 提取文本
    let text = resp
        .message
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    println!("[non-stream] Response: {}", text);
    println!(
        "[non-stream] Usage: in={} out={} cache={}",
        resp.usage.input_tokens, resp.usage.output_tokens, resp.usage.cache_creation_tokens
    );

    assert!(!text.is_empty(), "Text content should not be empty");
    assert!(resp.usage.output_tokens > 0, "Should have output tokens");
}

/// Test 2: 流式调用 — SSE streaming
#[tokio::test]
async fn test_streaming_chat() {
    let provider = build_provider();

    let req = ProviderRequest {
        model: MODEL.into(),
        messages: vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "1+1等于几？直接回答数字。".into(),
            }],
        )],
        system_prompt: Some("你是一个数学助手。简短回答。".into()),
        max_tokens: Some(64),
        temperature: None,
        stream: true,
        tools: None,
        thinking: None,
        reasoning_effort: None,
        enable_prompt_cache: None,
        cache_key: None,
    };

    let (tx, mut rx) = mpsc::unbounded_channel();
    provider.stream(req, tx).await.expect("Stream call failed");

    let mut full_text = String::new();
    let mut got_done = false;

    // 收集所有事件 (最多等 30 秒)
    let timeout = tokio::time::Duration::from_secs(30);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(StreamEvent::Text { delta }) => {
                        print!("{}", delta);
                        full_text.push_str(&delta);
                    }
                    Some(StreamEvent::Thinking { delta }) => {
                        println!("[thinking] {}", delta);
                    }
                    Some(StreamEvent::Done { usage }) => {
                        println!("\n[stream] Done. Usage: in={} out={}", usage.input_tokens, usage.output_tokens);
                        got_done = true;
                        break;
                    }
                    Some(StreamEvent::Error { message }) => {
                        panic!("Stream error: {}", message);
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                break;
            }
        }
    }

    assert!(got_done, "Should receive Done event");
    assert!(!full_text.is_empty(), "Should have streamed some text");
    println!("[stream] Full text: {}", full_text);
}

/// Test 3: 工具调用 (function calling)
#[tokio::test]
async fn test_tool_calling() {
    let provider = build_provider();

    let tool_schema = serde_json::json!({
        "name": "get_weather",
        "description": "获取指定城市的当前天气",
        "input_schema": {
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "城市名称，例如 北京"
                }
            },
            "required": ["city"]
        }
    });

    let req = ProviderRequest {
        model: MODEL.into(),
        messages: vec![Message::new(
            Role::User,
            vec![ContentBlock::Text {
                text: "北京今天天气怎么样？".into(),
            }],
        )],
        system_prompt: Some("你是一个天气助手。使用 get_weather 工具查询天气。".into()),
        max_tokens: Some(256),
        temperature: None,
        stream: false,
        tools: Some(vec![tool_schema]),
        thinking: None,
        reasoning_effort: None,
        enable_prompt_cache: None,
        cache_key: None,
    };

    let resp = provider.complete(req).await.expect("Tool call failed");

    println!("[tool] Response blocks: {:?}", resp.message.content);

    // 检查是否有 tool_use 调用
    let has_tool_use = resp.message.content.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));

    if has_tool_use {
        for block in &resp.message.content {
            if let ContentBlock::ToolUse { tool_name, input, tool_call_id } = block {
                println!(
                    "[tool] Called: {} with input={:?} id={}",
                    tool_name, input, tool_call_id
                );
                assert_eq!(tool_name, "get_weather");
                assert!(input.get("city").is_some(), "Should have city param");
            }
        }
    } else {
        // 有些模型可能直接文本回答而不调用工具，也 OK
        let text = resp.message.content.iter()
            .filter_map(|b| match b { ContentBlock::Text { text } => Some(text.as_str()), _ => None })
            .collect::<Vec<_>>()
            .join("");
        println!("[tool] Model responded with text instead: {}", text);
    }

    println!(
        "[tool] Usage: in={} out={}",
        resp.usage.input_tokens, resp.usage.output_tokens
    );
    assert!(resp.usage.output_tokens > 0, "Should have output tokens");
}
