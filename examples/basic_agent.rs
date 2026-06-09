//! 积木式 Agent 框架 - 使用示例
//!
//! 展示如何组装各积木模块, 构建一个可工作的 Agent
//!
//! 运行: cargo run --example basic_agent

use orion_agent::core::r#loop::{run_simple_loop, SimpleLoopConfig, SimpleLoopContext};
use orion_agent::core::cache::GlobalCache;
use orion_agent::tools::{ReadTool, WriteTool, BashTool};
use orion_agent::tools::registry::ToolRegistry;

/// 一个简单的 Mock Provider (用于演示, 不依赖真实 API)
struct MockProvider {
    name: String,
}

#[async_trait::async_trait]
impl orion_agent::core::provider::Provider for MockProvider {
    fn name(&self) -> &str { &self.name }

    fn supported_models(&self) -> Vec<&str> { vec!["mock-model"] }

    async fn complete(&self, _req: orion_agent::core::provider::ProviderRequest)
        -> orion_agent::Result<orion_agent::core::provider::ProviderResponse>
    {
        // Mock: 返回一个简单的文本响应
        Ok(orion_agent::core::provider::ProviderResponse {
            message: orion_agent::core::provider::Message {
                role: orion_agent::core::provider::Role::Assistant,
                content: vec![
                    orion_agent::core::provider::ContentBlock::Text {
                        text: "Hello from mock provider! This is a test.".into(),
                    }
                ],
                reasoning_content: None,
                cache_breakpoint: false,
            },
            usage: orion_agent::core::provider::UsageInfo::default(),
        })
    }

    async fn stream(
        &self,
        _req: orion_agent::core::provider::ProviderRequest,
        _tx: tokio::sync::mpsc::UnboundedSender<orion_agent::core::provider::StreamEvent>,
    ) -> orion_agent::Result<()> {
        Err(orion_agent::Error::Provider("stream not supported in mock".into()))
    }
}

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    // ============================================================
    // Step 1: 注册工具 (积木式添加)
    // ============================================================
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(ReadTool);
    tool_registry.register(WriteTool);
    tool_registry.register(BashTool);
    println!("📦 Registered {} tools", tool_registry.len());
    for def in tool_registry.definitions() {
        println!("  - {}", def["name"]);
    }

    // ============================================================
    // Step 2: 创建 Provider & Cache
    // ============================================================
    let provider = MockProvider {
        name: "mock".to_string(),
    };
    let cache = GlobalCache::new(500, 300, 5000);

    // ============================================================
    // Step 3: 配置循环
    // ============================================================
    let loop_config = SimpleLoopConfig {
        model: "mock-model".to_string(),
        system_prompt: "You are a helpful coding assistant.".to_string(),
        max_turns: 10,
        max_tool_calls: 30,
        token_budget: 128_000,
        agent_id: "basic-agent".to_string(),
        ..SimpleLoopConfig::default()
    };

    // ============================================================
    // Step 4: 运行查询循环
    // ============================================================
    let outcome = run_simple_loop(
        &provider,
        &tool_registry,
        &cache,
        &loop_config,
        "Hello! What can you do?",
        SimpleLoopContext::default(),
    ).await;

    match outcome {
        orion_agent::core::r#loop::LoopOutcome::Completed { message, usage } => {
            println!("✅ Agent response:");
            println!("{}", message);
            println!("📊 Usage: {} in / {} out", usage.input_tokens, usage.output_tokens);
        }
        orion_agent::core::r#loop::LoopOutcome::MaxTurnsReached { message, .. } => {
            println!("⚠️  Max turns reached: {}", message);
        }
        orion_agent::core::r#loop::LoopOutcome::BudgetExceeded { .. } => {
            println!("⚠️  Budget exceeded");
        }
        orion_agent::core::r#loop::LoopOutcome::GuardrailDenied { reason } => {
            println!("🚫 Guardrail denied: {}", reason);
        }
        orion_agent::core::r#loop::LoopOutcome::Error { message } => {
            println!("❌ Error: {}", message);
        }
    }

    Ok(())
}
