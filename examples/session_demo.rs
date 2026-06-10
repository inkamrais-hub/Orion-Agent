//! Session Demo — 创建/恢复对话
//!
//! 运行: cargo run --example session_demo
//!
//! 第一次: 创建新 session
//! 第二次: 列出已有 session, 选择继续

use orion_agent::session::UnifiedStore;
use orion_agent::session::unified::TranscriptEntry;
use orion_agent::session::store::{SessionMeta, SessionStatus as StoreSessionStatus};
use chrono::Utc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    let _ = dotenvy::dotenv();

    // 初始化日志
    // telemetry removed

    let store = UnifiedStore::open().await?;

    // 列出已有 session
    let sessions = store.list_sessions(100).await?;
    if sessions.is_empty() {
        println!("No existing sessions.");
    } else {
        println!("Existing sessions:");
        for (i, s) in sessions.iter().enumerate() {
            println!("  [{}] {} ({}) — {} turns, status: {:?}",
                i, &s.session_id[..8], s.model, s.turn_count, s.status);
        }
    }

    // 创建新 session
    let model = "deepseek-v4-flash";
    let session_id = orion_agent::session::store::generate_session_id();
    let now = chrono::Utc::now();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into());
    store.create_session(&SessionMeta {
        session_id: session_id.clone(),
        agent_name: "demo".into(),
        model: model.to_string(),
        working_dir: cwd,
        status: StoreSessionStatus::Active,
        created_at: now,
        updated_at: now,
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    }).await?;
    println!("\nNew session: {}", &session_id[..8]);

    // 模拟对话
    let messages = vec![
        ("user", "Hello! What can you do?"),
        ("assistant", "I'm a coding assistant. I can help you write, read, and test code."),
        ("user", "Write a hello world in Rust."),
        ("assistant", "fn main() { println!(\"Hello, world!\"); }"),
    ];

    for (role, content) in &messages {
        let entry = TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            parent_id: None,
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            timestamp: Utc::now(),
        };
        store.append_transcript(&session_id, &entry).await?;
    }

    // 更新元数据
    store.update_session_stats(
        &session_id,
        messages.len() as u32,
        0,
        500,
    ).await?;
    store.update_session_status(&session_id, StoreSessionStatus::Completed).await?;

    println!("Saved {} messages to session {}", messages.len(), &session_id[..8]);

    // 恢复
    let restored = store.get_transcript(&session_id).await?;
    println!("\nRestored {} messages:", restored.len());
    for entry in &restored {
        println!("  [{}] {}", entry.role, &entry.content[..entry.content.len().min(60)]);
    }

    // 搜索
    println!("\n(Search feature not yet implemented in UnifiedStore)");

    Ok(())
}
