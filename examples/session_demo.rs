//! Session Demo — 创建/恢复对话
//!
//! 运行: cargo run --example session_demo
//!
//! 第一次: 创建新 session
//! 第二次: 列出已有 session, 选择继续

use orion_agent::session::manager::SessionManager;
use orion_agent::session::TranscriptEntry;
use chrono::Utc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    let _ = dotenvy::dotenv();

    // 初始化日志
    // telemetry removed

    let mgr = SessionManager::open().await?;

    // 列出已有 session
    let sessions = mgr.list().await?;
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
    let session_id = mgr.create(model).await?;
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
        mgr.append_transcript(&session_id, &entry).await?;
    }

    // 更新元数据
    mgr.update(&session_id, |e| {
        e.turn_count = messages.len() as u64;
        e.total_tokens = 500;
        e.status = orion_agent::session::SessionStatus::Completed;
    }).await?;

    println!("Saved {} messages to session {}", messages.len(), &session_id[..8]);

    // 恢复
    let restored = mgr.restore(&session_id).await?;
    println!("\nRestored {} messages:", restored.len());
    for entry in &restored {
        println!("  [{}] {}", entry.role, &entry.content[..entry.content.len().min(60)]);
    }

    // 搜索
    let results = mgr.search("Rust").await?;
    println!("\nSearch 'Rust': {} results", results.len());

    Ok(())
}
