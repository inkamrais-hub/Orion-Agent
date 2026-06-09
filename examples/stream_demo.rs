//! 流式 CLI 示例 — 实时 token 输出 + thinking 推理显示
//!
//! 运行: cargo run --example stream_demo "你的问题"
//! 效果: token 逐个打印, thinking 灰色显示

use std::io::Write;
use std::time::Instant;
use tokio::sync::mpsc;

use orion_agent::core::provider::{
    ContentBlock, Message, Provider, ProviderRequest, Role, StreamEvent,
};
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    let _ = dotenvy::dotenv();

    let provider = Box::new(OpenAICompatProvider::from_env());
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into());

    let user_input = std::env::args().nth(1).unwrap_or_else(|| {
        "介绍一下你自己".to_string()
    });

    println!("──────────────────────────────────────────────");
    println!("  🚀 流式模式 — Model: {}", model);
    println!("──────────────────────────────────────────────");
    println!("📝 User: {}", user_input);
    print!("🤖 ");

    let req = ProviderRequest {
        model,
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_input }],
            reasoning_content: None,
            cache_breakpoint: false,
        }],
        system_prompt: Some("You are a helpful assistant.".into()),
        max_tokens: Some(4096),
        temperature: Some(0.7),
        stream: false,
        tools: None,
        thinking: None,
        reasoning_effort: None,
            enable_prompt_cache: None,
            cache_key: None,
    };

    let (tx, mut rx) = mpsc::unbounded_channel();
    let start = Instant::now();
    let mut thinking = 0u64;

    provider.stream(req, tx).await?;

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Text { delta } => {
                print!("{}", delta);
            }
            StreamEvent::Thinking { delta } => {
                print!("\x1b[90m{}", delta); // 灰色
                thinking += 1;
            }
            StreamEvent::Done { usage } => {
                println!("\x1b[0m"); // 重置颜色
                println!();
                println!("──────────────────────────────────────────────");
                println!("  ✅ {:?}", start.elapsed());
                println!("  📊 {} in / {} out tokens", usage.input_tokens, usage.output_tokens);
                if thinking > 0 {
                    println!("  💭 Thinking tokens: {}", thinking);
                }
                break;
            }
            StreamEvent::Error { message } => {
                eprintln!("\x1b[0m\n❌ Error: {}", message);
                break;
            }
            _ => {}
        }
        std::io::stdout().flush().ok();
    }

    Ok(())
}
