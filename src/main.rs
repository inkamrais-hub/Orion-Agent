// ============================================================================
// Orion Agent — 主入口
//
// 用法:
//   orion-agent                              → 启动上层系统（选择 CLI/WebUI）
//   orion-agent --workspace /path/to/project → 以指定目录为工作目录启动
//   orion-agent -w /path/to/project          → 同上（短参数）
//   orion-agent --onlyrun "任务"              → 一次性执行任务
//   orion-agent --help                       → 显示帮助
//   orion-agent --version                    → 显示版本
// ============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 从命令行参数中提取 --workspace / -w 的值，返回 (workspace, remaining_args)
fn parse_workspace_flag(args: &[String]) -> (Option<String>, Vec<String>) {
    let mut workspace: Option<String> = None;
    let mut remaining: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "--workspace" || args[i] == "-w") && i + 1 < args.len() {
            workspace = Some(args[i + 1].clone());
            i += 2;
        } else {
            remaining.push(args[i].clone());
            i += 1;
        }
    }
    (workspace, remaining)
}

#[tokio::main]
async fn main() {
    #[cfg(feature = "dotenv")]
    { let _ = dotenvy::dotenv(); }

    let all_args: Vec<String> = std::env::args().collect();
    let (workspace, args) = parse_workspace_flag(&all_args);

    // 如果指定了 workspace，切换当前工作目录
    if let Some(ref ws) = workspace {
        let ws_path = std::path::Path::new(ws);
        if !ws_path.exists() {
            eprintln!("错误: 工作目录不存在: {}", ws);
            std::process::exit(1);
        }
        if let Err(e) = std::env::set_current_dir(ws_path) {
            eprintln!("错误: 无法切换到工作目录 {}: {}", ws, e);
            std::process::exit(1);
        }
    }

    // args[0] 是程序名，从 args[1] 开始是实际参数
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
        // ── 旧版 CLI 模式 ──
        Some("--legacy") => {
            let config = orion_agent::config::OrionConfig::load();
            if let Err(e) = orion_agent::cli::chat::run(config, workspace.unwrap_or_default()).await {
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
            if let Err(e) = orion_agent::gateway::run_gateway(workspace).await {
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
    println!("  orion-agent                            启动 TUI 交互模式");
    println!("  orion-agent --workspace <path>         以指定目录为工作目录启动");
    println!("  orion-agent -w <path>                  同上（短参数）");
    println!("  orion-agent --legacy                   启动旧版文本 CLI 模式");
    println!("  orion-agent --onlyrun \"任务\"            一次性执行任务");
    println!("  orion-agent --help                     显示此帮助");
    println!("  orion-agent --version                  显示版本");
    println!();
    println!("配置文件: ~/.orion/config.yaml");
}
