//! Coding Agent — 多 Agent 协作编码
//!
//! 运行: cargo run --example coding_agent "用 Rust 写一个斐波那契数列"
//!
//! 环境变量:
//!   OUTPUT_MODE=json  — JSON 输出 (CI/CD)
//!   RUST_LOG=debug    — 调试日志

use orion_agent::config::OrionConfig;
use orion_agent::core::cache::GlobalCache;
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;
use orion_agent::orchestrator::coordinator::{Coordinator, CoordinatorConfig};
use orion_agent::agent::registry::AgentRegistry;
use orion_agent::session::manager::SessionManager;
use orion_agent::ui::progress::ProgressBar;
use orion_agent::ui::report::{SessionReport, print_session_report};
use std::sync::Arc;

#[tokio::main]
async fn main() -> orion_agent::Result<()> {
    let _ = dotenvy::dotenv();

    // 1. 初始化日志
    // telemetry removed

    // 2. 初始化工作区守卫
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    orion_agent::core::workspace::init_workspace_guard(workspace_root).await;

    // 2. Ctrl+C 优雅退出
    tokio::spawn(async {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n⚠ Interrupted — exiting");
        std::process::exit(130);
    });

    // 3. 加载配置
    let config = OrionConfig::load();
    let json_mode = std::env::var("OUTPUT_MODE").unwrap_or_default() == "json";

    if !json_mode {
        eprintln!("========================================");
        eprintln!("  Orion Coding Agent");
        eprintln!("  Model: {}", config.default_model);
        eprintln!("  Mode:  {}", config.orchestrator.mode);
        eprintln!("  Workers: {}", config.orchestrator.max_workers);
        eprintln!("========================================");
    }

    // 4. 创建 Session
    let session_mgr = SessionManager::open().await?;
    let session_id = session_mgr.create(&config.default_model).await?;
    tracing::info!(session = %session_id, "Session started");

    // 5. 全局缓存
    let cache = GlobalCache::from_config(&config.cache);

    // 6. Provider (使用 Arc 共享，避免重复创建 HTTP 客户端)
    let provider: Arc<dyn orion_agent::core::provider::Provider> =
        Arc::new(OpenAICompatProvider::from_env());

    // 7. Coordinator
    let coord_config = CoordinatorConfig {
        worker_model: config.orchestrator.worker_model.clone(),
        max_rounds: config.orchestrator.max_rounds,
    };
    let report_cache = cache.clone();
    let registry = AgentRegistry::new();
    let coordinator = Coordinator::new(coord_config, provider, cache, registry);

    // 8. 用户输入
    let user_request = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if user_request.is_empty() {
        eprintln!("Usage: cargo run --example coding_agent \"your coding task\"");
        std::process::exit(1);
    }
    if !json_mode {
        eprintln!("\nTask: {}\n", user_request);
    }

    // 9. 执行
    let start = std::time::Instant::now();
    let mut progress = if !json_mode { Some(ProgressBar::new("Orchestrating...")) } else { None };
    let result = coordinator.execute(&user_request).await;
    let duration = start.elapsed();
    if let Some(ref mut pb) = progress {
        pb.set_message("Done");
    }
    drop(progress);

    // 10. 更新 Session
    session_mgr.update(&session_id, |e| {
        e.status = orion_agent::session::SessionStatus::Completed;
    }).await.ok();

    // 11. Session 报告
    if !json_mode {
        let report = SessionReport::from_cache(&report_cache, duration, 1, 0, 0);
        print_session_report(&report);
    }

    // 12. 输出结果
    match result {
        Ok(answer) => {
            if json_mode {
                let output = serde_json::json!({
                    "status": "completed",
                    "session_id": session_id,
                    "result": answer,
                    "cache": report_cache.full_report(),
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                eprintln!("\n========================================");
                eprintln!("Session: {}", &session_id[..8]);
                eprintln!("Cache: {}", report_cache.full_report());
                eprintln!("========================================");
                println!("{}", answer);
                eprintln!("========================================");
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
