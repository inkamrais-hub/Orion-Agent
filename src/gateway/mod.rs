//! Orion Gateway - 元系统层
//!
//! 负责 UI 调度、日志审计、消息聚合、系统调度

pub mod commands;
pub mod config;

use std::sync::Arc;
use tokio::sync::broadcast;
use crate::log_info;

/// 简易事件总线 (基于 tokio broadcast channel)
pub struct EventBus {
    tx: broadcast::Sender<String>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub async fn start(&self) {
        tracing::info!("EventBus started");
    }

    /// 发布事件
    pub fn publish(&self, event: &str) {
        let _ = self.tx.send(event.to_string());
    }

    /// 订阅事件
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Gateway 上下文
pub struct GatewayContext {
    /// 事件总线
    pub event_bus: Arc<EventBus>,
    /// 统一配置
    pub config: crate::config::OrionConfig,
    /// 工作目录
    pub working_dir: String,
}

impl GatewayContext {
    /// 创建新的 Gateway 上下文
    pub fn new(config: crate::config::OrionConfig) -> Self {
        let working_dir = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into());

        Self {
            event_bus: Arc::new(EventBus::new()),
            config,
            working_dir,
        }
    }
}

/// 启动上层系统 — 显示菜单选择 CLI/WebUI
pub async fn run_gateway() -> crate::Result<()> {
    crate::logging::init_logging();
    log_info!("gateway", "Orion Agent 启动中...");

    let config = crate::config::OrionConfig::load();

    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    crate::core::workspace::init_workspace_guard(workspace_root.clone()).await;

    // 显示选择菜单
    println!("╔══════════════════════════════════════╗");
    println!("║        Orion Agent v{}             ║", env!("CARGO_PKG_VERSION"));
    println!("╠══════════════════════════════════════╣");
    println!("║                                      ║");
    println!("║  [1] CLI   终端交互模式 (默认)        ║");
    println!("║  [2] WebUI 浏览器界面                 ║");
    println!("║                                      ║");
    println!("╚══════════════════════════════════════╝");
    print!("> ");
    use std::io::Write;
    std::io::stdout().flush().ok();

    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let choice = input.trim();

    match choice {
        "2" => {
            // WebUI 模式 → 启动 WebSocket API
            log_info!("gateway", "启动 WebUI 模式...");
            commands::route_command("api", vec![], GatewayContext::new(config)).await
        }
        _ => {
            // CLI 模式 → 启动交互式对话
            log_info!("gateway", "启动 CLI 模式...");
            commands::route_command("chat", vec![], GatewayContext::new(config)).await
        }
    }
}

/// 公共执行函数：执行单次任务
///
/// 封装：加载配置 → 获取 API Key → 创建 Provider → 注册工具 →
/// 创建 Session → 初始化组件 → 运行 SimpleLoop → 返回结果
pub async fn run_task_once(
    task: &str,
    config: &crate::config::OrionConfig,
    images: Option<Vec<crate::core::provider::ContentBlock>>,
) -> crate::Result<String> {
    let model_config = config.active_model();

    // API Key: 配置 > 环境变量
    let api_key = model_config.api_key.clone()
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("LLM_API_KEY").ok())
        .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
        .unwrap_or_default();
    if api_key.is_empty() {
        return Err(crate::Error::Config(
            "未设置 API Key。请在 ~/.orion/config.yaml 中配置 models[].api_key，或设置 LLM_API_KEY".into()
        ));
    }

    // 创建 Provider
    let provider: Box<dyn crate::core::provider::Provider> =
        Box::new(crate::core::providers::openai_compat::OpenAICompatProvider::new(
            &model_config.endpoint, &api_key, &model_config.name,
        ));
    let provider = std::sync::Arc::from(provider);

    // 注册工具
    let mut tools = crate::tools::registry::ToolRegistry::new();
    crate::tools::register_default_tools(&mut tools);
    tools.register(crate::tools::multi_shell::MultiShellTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotHistoryTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRollbackTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRiskyTool);
    let search_proxy = model_config.proxy.clone()
        .unwrap_or_else(|| std::env::var("HTTP_PROXY").unwrap_or_default());
    if search_proxy.is_empty() {
        tools.register(crate::tools::web_search::WebSearchTool::new());
    } else {
        tools.register(crate::tools::web_search::WebSearchTool::with_proxy(&search_proxy));
    }

    // 连接配置中的 MCP server 并注入工具
    crate::tools::mcp_init::init_mcp_tools(config, &mut tools).await;

    let cache = crate::core::cache::GlobalCache::new(1000, 300, 10000);
    let system_prompt = crate::cli::execute::build_system_prompt(&tools);

    // 创建 Session
    let session_id = crate::session::store::generate_session_id();
    let store = crate::session::UnifiedStore::open().await?;
    let _ = store.create_session(&crate::session::unified::SessionMeta {
        session_id: session_id.clone(),
        agent_name: "main".into(),
        model: model_config.name.clone(),
        working_dir: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into()),
        status: crate::session::unified::SessionStatus::Active,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    }).await;

    let hook_engine = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::core::hooks::HookEngine::load_default(),
    ));
    let exec_policy = crate::core::execpolicy::ExecPolicy::load_default();
    let mut rollout = crate::session::rollout::RolloutRecorder::new(&session_id).ok();
    let mut goal_manager = crate::core::goal::GoalManager::new();

    // 执行
    let token_budget = model_config.max_input_tokens.unwrap_or(128_000);
    let loop_config = crate::core::r#loop::SimpleLoopConfig {
        model: model_config.name.clone(),
        system_prompt,
        max_turns: 50,
        max_tool_calls: 100,
        token_budget,
        agent_id: "main".into(),
        session_id: session_id.clone(),
        model_caps: crate::core::r#loop::ModelCaps {
            thinking: model_config.thinking,
            prompt_cache: model_config.prompt_cache,
            max_output_tokens: model_config.max_tokens.unwrap_or(4096),
        },
        exec_mode: crate::core::exec_mode::ExecMode::default(),
    };

    let outcome = crate::core::r#loop::run_simple_loop(
        &*provider, &tools, &cache, &loop_config, task,
        crate::core::r#loop::SimpleLoopContext {
            hook_engine: Some(hook_engine),
            exec_policy: Some(&exec_policy),
            rollout: rollout.as_mut(),
            goal_manager: Some(&mut goal_manager),
            images,
            ..Default::default()
        },
    ).await;

    match outcome {
        crate::core::r#loop::LoopOutcome::Completed { message, usage } => {
            tracing::info!("任务完成 tokens={}", usage.input_tokens + usage.output_tokens);
            Ok(message)
        }
        other => Ok(format!("{:?}", other)),
    }
}

/// 一次性执行模式 (--onlyrun)
///
/// 直接执行任务，不进入交互模式。
pub async fn run_onlyrun(task: String) -> crate::Result<()> {
    crate::logging::init_logging();

    let config = crate::config::OrionConfig::load();
    let message = run_task_once(&task, &config, None).await?;
    println!("{}", message);
    Ok(())
}
