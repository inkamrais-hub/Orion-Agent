// ============================================================================
// Orion Agent — 主入口
//
// 用法:
//   orion-agent                          → 启动交互式终端 (chat)
//   orion-agent run <任务> [--sandbox]   → 执行单次任务
//   orion-agent --onlyrun "任务"         → 一次性执行 (兼容旧用法)
//   orion-agent --help                   → 显示帮助
//   orion-agent --version                → 显示版本
// ============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    #[cfg(feature = "dotenv")]
    { let _ = dotenvy::dotenv(); }

    orion_agent::logging::init_logging();

    let args: Vec<String> = std::env::args().collect();
    let config = orion_agent::config::OrionConfig::load();
    let ctx = orion_agent::gateway::GatewayContext::new(config);

    let cmd = args.get(1).map(|s| s.as_str());

    match cmd {
        // ── 帮助 ──
        Some("--help") | Some("-h") | Some("help") => {
            print_help();
        }
        // ── 版本 ──
        Some("--version") | Some("-v") | Some("version") => {
            println!("orion-agent {}", VERSION);
        }
        // ── --onlyrun 兼容旧用法 → 路由到 run 命令 ──
        Some("--onlyrun") => {
            let sub_args = args[2..].to_vec();
            if let Err(e) = orion_agent::gateway::commands::route_command("run", sub_args, ctx).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        // ── 内置子命令 ──
        Some(subcommand) if subcommand == "chat" || subcommand == "api" || subcommand == "run" || subcommand == "config" || subcommand == "index" => {
            let sub_args = args[2..].to_vec();
            if let Err(e) = orion_agent::gateway::commands::route_command(subcommand, sub_args, ctx).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        // ── 未知参数 ──
        Some(unknown) if unknown.starts_with('-') => {
            eprintln!("未知参数: {}", unknown);
            eprintln!("运行 --help 查看用法");
            std::process::exit(1);
        }
        // ── 自由输入文本作为单次任务 ──
        Some(_) => {
            let sub_args = args[1..].to_vec();
            if let Err(e) = orion_agent::gateway::commands::route_command("run", sub_args, ctx).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        // ── 无参数 → 默认启动 chat 交互模式 ──
        None => {
            if let Err(e) = orion_agent::gateway::commands::route_command("chat", vec![], ctx).await {
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
    println!("  orion-agent                          启动交互式终端 (chat)");
    println!("  orion-agent chat                     启动交互式终端");
    println!("  orion-api [port]                     启动 Web API 服务");
    println!("  orion-agent run <任务> [--sandbox]    执行单次任务");
    println!("  orion-agent --onlyrun \"任务\"          一次性执行 (兼容旧用法)");
    println!("  orion-agent config                   查看/修改配置");
    println!("  orion-agent index                    索引项目代码");
    println!("  orion-agent --help                   显示此帮助");
    println!("  orion-agent --version                显示版本");
    println!();
    println!("安全选项:");
    println!("  --sandbox                            启用无网络沙箱 (禁止 git push/网络命令)");
    println!();
    println!("配置文件: ~/.orion/config.yaml");
}

