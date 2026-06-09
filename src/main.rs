// ============================================================================
// Orion Agent — 主入口
//
// 用法:
//   orion-agent                    → 启动上层系统（选择 CLI/WebUI）
//   orion-agent --onlyrun "任务"    → 一次性执行任务
//   orion-agent --help             → 显示帮助
//   orion-agent --version          → 显示版本
// ============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    #[cfg(feature = "dotenv")]
    { let _ = dotenvy::dotenv(); }

    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        // ── 帮助 ──
        Some("--help") | Some("-h") => {
            print_help();
            return;
        }
        // ── 版本 ──
        Some("--version") | Some("-v") => {
            println!("orion-agent {}", VERSION);
            return;
        }
        // ── 一次性执行 ──
        Some("--onlyrun") => {
            let task = args.get(2).cloned().unwrap_or_default();
            if task.is_empty() {
                eprintln!("错误: --onlyrun 需要任务描述");
                eprintln!("用法: orion-agent --onlyrun \"任务描述\"");
                std::process::exit(1);
            }
            if let Err(e) = orion_agent::gateway::run_onlyrun(task).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        // ── 未知参数 ──
        Some(unknown) => {
            eprintln!("未知参数: {}", unknown);
            eprintln!("运行 --help 查看用法");
            std::process::exit(1);
        }
        // ── 无参数 → 启动上层系统 ──
        None => {
            if let Err(e) = orion_agent::gateway::run_gateway().await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn print_help() {
    println!("Orion Agent v{} — 积木化 Rust Agent 框架", VERSION);
    println!();
    println!("用法:");
    println!("  orion-agent                    启动上层系统（选择 CLI/WebUI）");
    println!("  orion-agent --onlyrun \"任务\"    一次性执行任务");
    println!("  orion-agent --help             显示此帮助");
    println!("  orion-agent --version          显示版本");
    println!();
    println!("配置文件: ~/.orion/config.yaml");
}
