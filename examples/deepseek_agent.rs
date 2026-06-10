//! DeepSeek Agent 示例 — orion-agent MVP 体验
//!
//! 前置: .env 已配好 API Key (默认用提供的 DeepSeek key)
//! 运行: cargo run --example deepseek_agent "你的问题"

use orion_agent::core::r#loop::{run_simple_loop, SimpleLoopConfig, SimpleLoopContext};
use orion_agent::core::cache::GlobalCache;
use orion_agent::core::provider::Provider;
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;
use orion_agent::tools::{BashTool, ReadTool, WriteTool};
use orion_agent::tools::registry::ToolRegistry;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    // 加载 .env (如果存在)
    let _ = dotenvy::dotenv();

    // ============================================================
    // Step 1: 工具
    // ============================================================
    let mut tools = ToolRegistry::new();
    tools.register(ReadTool);
    tools.register(WriteTool);
    tools.register(BashTool);

    // ============================================================
    // Step 2: Cache
    // ============================================================
    let cache = GlobalCache::new(500, 300, 5000);

    // ============================================================
    // Step 3: Provider (DeepSeek)
    // ============================================================
    let provider = OpenAICompatProvider::from_env()?;

    // ============================================================
    // Step 4: 循环配置
    // ============================================================
    let loop_config = SimpleLoopConfig {
        model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into()),
        system_prompt: "\
You are a coding assistant. You MUST use the available tools to complete tasks.
**CRITICAL**: When asked to read a file, you MUST call the `read` tool.
When asked to write code, you MUST call the `write` tool.
DO NOT fabricate file contents from your training data.
Always use tools for any file operations."
            .to_string(),
        max_turns: 20,
        max_tool_calls: 30,
        token_budget: 128_000,
        agent_id: "deepseek-agent".to_string(),
        ..SimpleLoopConfig::default()
    };

    println!("========================================");
    println!("  🚀 orion-agent 已启动");
    println!("  Provider: {}", provider.name());
    println!("  Model:    {}", loop_config.model);
    println!("========================================");

    // ============================================================
    // Step 5: 运行查询循环
    // ============================================================
    let user_input = std::env::args().nth(1).unwrap_or_else(|| {
        "Hello! What tools do you have available?".to_string()
    });

    println!("\n📝 User: {}\n", user_input);

    let outcome = run_simple_loop(
        &provider,
        &tools,
        &cache,
        &loop_config,
        &user_input,
        SimpleLoopContext::default(),
    ).await;

    println!("\n========================================");
    match outcome {
        orion_agent::core::r#loop::LoopOutcome::Completed { message, usage } => {
            println!("  ✅ Agent response ({} chars):", message.len());
            println!("{}", message.chars().take(500).collect::<String>());
            if message.chars().count() > 500 {
                println!("  ... (truncated)");
            }
            println!(
                "  📊 {} in / {} out tokens",
                usage.input_tokens, usage.output_tokens
            );
        }
        orion_agent::core::r#loop::LoopOutcome::MaxTurnsReached { message, .. } => {
            println!("  ⚠️  Max turns: {}", message);
        }
        orion_agent::core::r#loop::LoopOutcome::BudgetExceeded { .. } => {
            println!("  ⚠️  Budget exceeded");
        }
        orion_agent::core::r#loop::LoopOutcome::GuardrailDenied { reason } => {
            println!("  🚫 Guardrail: {}", reason);
        }
        orion_agent::core::r#loop::LoopOutcome::Error { message } => {
            println!("  ❌ Error: {}", message);
        }
    }
    println!("========================================");

    Ok(())
}
