//! A2A 双 Agent 互相沟通测试
//!
//! 创建两个 Agent:
//!   - Alice: 数学专家，负责计算
//!   - Bob: 文学专家，负责写作
//!
//! 它们通过 A2A 协议互相沟通，协作完成任务
//!
//! 运行: LLM_API_KEY=xxx cargo run --example a2a_chat

use orion_agent::agent::registry::AgentRegistry;
use orion_agent::agent::protocol::A2AMessage;
use orion_agent::core::cache::GlobalCache;
use orion_agent::core::provider::Provider;
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;
use orion_agent::core::r#loop::SimpleLoopConfig;
use orion_agent::tools::registry::ToolRegistry;
use orion_agent::tools::{ReadTool, WriteTool, BashTool};
use orion_agent::tools::a2a_message::{SendMessageTool, ListPeersTool, A2APeerConfig, init_a2a_peers};
use std::sync::Arc;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    // 1. 初始化
    // telemetry removed
    let workspace_root = std::env::current_dir().unwrap_or_default();
    orion_agent::core::workspace::init_workspace_guard(workspace_root).await;

    // 2. 创建 Provider
    let provider: Arc<dyn Provider> = Arc::new(OpenAICompatProvider::from_env());
    let cache = GlobalCache::new(500, 300, 5000);

    // 5. 创建 Agent Registry
    let registry = AgentRegistry::new();

    // 6. 配置 A2A 权限
    init_a2a_peers(vec![
        A2APeerConfig { id: "alice".into(), a2a_peers: vec!["bob".into()] },
        A2APeerConfig { id: "bob".into(), a2a_peers: vec!["alice".into()] },
    ]);

    // 7. 注册 Agent (register 返回接收端)
    let _alice_rx = registry.register("alice".into()).await;
    let mut bob_rx = registry.register("bob".into()).await;

    // 7. 创建工具集
    let mut alice_tools = ToolRegistry::new();
    alice_tools.register(ReadTool);
    alice_tools.register(BashTool);
    alice_tools.register(SendMessageTool);
    alice_tools.register(ListPeersTool);

    let mut bob_tools = ToolRegistry::new();
    bob_tools.register(ReadTool);
    bob_tools.register(WriteTool);
    bob_tools.register(SendMessageTool);
    bob_tools.register(ListPeersTool);

    // 8. 打印标题
    println!("╔══════════════════════════════════════════╗");
    println!("║  A2A 双 Agent 沟通测试                    ║");
    println!("║  Alice: 数学专家 (计算)                   ║");
    println!("║  Bob: 文学专家 (写作)                     ║");
    println!("╚══════════════════════════════════════════╝");
    println!();

    // 9. Alice 先发起对话
    println!("═══ Alice 开始工作 ═══");
    let alice_system = r#"你是 Alice，一个数学专家。你的职责是进行数学计算和分析。
你有一个同事 Bob，他是文学专家。你可以通过 send_message 工具和他沟通。
请先完成你的数学任务，然后把结果告诉 Bob。"#;

    let alice_task = "请计算斐波那契数列前 10 项的和，然后通过 send_message 把结果发给 Bob，让他写一首包含这个数字的诗。";

    let alice_config = SimpleLoopConfig {
        model: "deepseek-chat".to_string(),
        system_prompt: alice_system.to_string(),
        max_turns: 10,
        max_tool_calls: 15,
        token_budget: 128_000,
        agent_id: "alice".to_string(),
        ..SimpleLoopConfig::default()
    };
    let alice_result = orion_agent::core::r#loop::run_simple_loop(
        &*provider,
        &alice_tools,
        &cache,
        &alice_config,
        alice_task,
        orion_agent::core::r#loop::SimpleLoopContext {
            registry: Some(registry.clone()),
            ..Default::default()
        },
    ).await;

    match &alice_result {
        orion_agent::core::r#loop::LoopOutcome::Completed { message, usage } => {
            println!("\n  Alice 完成: {} tokens", usage.input_tokens + usage.output_tokens);
            let preview: String = message.chars().take(100).collect();
            println!("  Alice 回复: {}", preview);
        }
        other => println!("\n  Alice 结果: {:?}", other),
    }

    // 10. Bob 检查消息并回复
    println!("\n═══ Bob 开始工作 ═══");

    // Bob 检查 inbox
    let mut bob_inbox = Vec::new();
    while let Ok(msg) = bob_rx.try_recv() {
        bob_inbox.push(msg);
    }

    let bob_a2a_ctx = if bob_inbox.is_empty() {
        String::new()
    } else {
        let mut parts = Vec::new();
        for msg in &bob_inbox {
            if let Some(a2a) = A2AMessage::from_json(&msg.content) {
                match a2a {
                    A2AMessage::ShareResult { from, task_id, content, .. } => {
                        parts.push(format!("[A2A from {}] Task '{}' result: {}", from, task_id, content));
                    }
                    A2AMessage::RequestInfo { from, query, .. } => {
                        parts.push(format!("[A2A from {}] Question: {}", from, query));
                    }
                    _ => {}
                }
            }
        }
        parts.join("\n")
    };

    let bob_system = format!(
        "你是 Bob，一个文学专家。你的职责是写作和创意。\n你有一个同事 Alice，她是数学专家。\n\n{}\n\n请根据 Alice 发给你的信息，写一首包含那个数字的诗。",
        if bob_a2a_ctx.is_empty() { "Alice 还没有发消息给你。".into() } else { bob_a2a_ctx }
    );

    let bob_task = "请写一首包含 Alice 发给你的数字的诗。如果 Alice 还没有发消息，请先用 list_peers 查看可通信的 Agent。";

    let bob_config = SimpleLoopConfig {
        model: "deepseek-chat".to_string(),
        system_prompt: bob_system,
        max_turns: 10,
        max_tool_calls: 15,
        token_budget: 128_000,
        agent_id: "bob".to_string(),
        ..SimpleLoopConfig::default()
    };
    let bob_result = orion_agent::core::r#loop::run_simple_loop(
        &*provider,
        &bob_tools,
        &cache,
        &bob_config,
        bob_task,
        orion_agent::core::r#loop::SimpleLoopContext {
            registry: Some(registry.clone()),
            ..Default::default()
        },
    ).await;

    match &bob_result {
        orion_agent::core::r#loop::LoopOutcome::Completed { message, usage } => {
            println!("\n  Bob 完成: {} tokens", usage.input_tokens + usage.output_tokens);
            let preview: String = message.chars().take(150).collect();
            println!("  Bob 回复: {}", preview);
        }
        other => println!("\n  Bob 结果: {:?}", other),
    }

    println!("\n═══ A2A 测试完成 ═══");
    Ok(())
}
