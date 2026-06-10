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

/// 启动上层系统 — 默认直接启动 CLI 交互式对话
pub async fn run_gateway() -> crate::Result<()> {
    crate::logging::init_logging();
    log_info!("gateway", "Orion Agent 启动中...");

    let config = crate::config::OrionConfig::load();
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    crate::core::workspace::init_workspace_guard(workspace_root.clone()).await;

    let ctx = GatewayContext::new(config);
    commands::route_command("chat", vec![], ctx).await
}

/// 构建主 Agent 实例，包含默认工具、缓存、Hook 引擎和安全策略
///
/// `sandbox` 为 true 时启用无网络沙箱模式，禁止 git push/fetch/clone 及所有网络命令
pub async fn build_main_agent(
    config: &crate::config::OrionConfig,
    sandbox: bool,
) -> crate::Result<crate::core::agent::Agent> {
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
    let provider: Arc<dyn crate::core::provider::Provider> =
        Arc::from(Box::new(crate::core::providers::openai_compat::OpenAICompatProvider::new(
            &model_config.endpoint, &api_key, &model_config.name,
        )) as Box<dyn crate::core::provider::Provider>);

    // 注册工具
    let mut tools = crate::tools::registry::ToolRegistry::new();
    crate::tools::register_default_tools(&mut tools);
    tools.register(crate::tools::multi_shell::MultiShellTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotHistoryTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRollbackTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRiskyTool);
    tools.register(crate::tools::agent_tool::SubAgentTool::new());
    tools.register(crate::tools::a2a_message::SendMessageTool);
    tools.register(crate::tools::a2a_message::ListPeersTool);

    
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

    let hook_engine = crate::core::hooks::HookEngine::load_default();
    let exec_policy = if sandbox {
        tracing::info!("Sandbox mode enabled: network operations and VCS writes blocked");
        crate::core::execpolicy::ExecPolicy::sandbox_policy()
    } else {
        crate::core::execpolicy::ExecPolicy::load_default()
    };

    let agent = crate::core::agent::Agent::builder()
        .name("main")
        .model(&model_config.name)
        .system_prompt(system_prompt)
        .provider(provider)
        .tools(tools)
        .cache(cache)
        .hook_engine(hook_engine)
        .exec_policy(exec_policy)
        .max_turns(50)
        .max_tool_calls(100)
        .token_budget(model_config.max_input_tokens.unwrap_or(128_000))
        .thinking(model_config.thinking)
        .reasoning_effort("medium")
        .build()?;

    Ok(agent)
}

/// 公共执行函数：执行单次任务
pub async fn run_task_once(
    task: &str,
    config: &crate::config::OrionConfig,
    images: Option<Vec<crate::core::provider::ContentBlock>>,
    sandbox: bool,
) -> crate::Result<String> {
    // 初始化工作区守卫 (必须在工具执行前完成)
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    crate::core::workspace::init_workspace_guard(workspace_root).await;

    let agent = build_main_agent(config, sandbox).await?;

    // 创建 Session
    let session_id = crate::session::store::generate_session_id();
    let store = crate::session::UnifiedStore::open().await?;
    let _ = store.create_session(&crate::session::store::SessionMeta {
        session_id: session_id.clone(),
        agent_name: agent.config().name.clone(),
        model: agent.config().model.clone(),
        working_dir: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".into()),
        status: crate::session::store::SessionStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    }).await;

    let rollout = crate::session::rollout::RolloutRecorder::new(&session_id).ok()
        .map(|r| std::sync::Arc::new(tokio::sync::Mutex::new(r)));
    let goal_manager = std::sync::Arc::new(tokio::sync::Mutex::new(crate::core::goal::GoalManager::new()));

    let model_config = config.active_model();

    // 运行 SimpleLoop
    let loop_config = crate::core::r#loop::SimpleLoopConfig {
        model: agent.config().model.clone(),
        system_prompt: agent.config().system_prompt.clone(),
        max_turns: agent.config().max_turns,
        max_tool_calls: agent.config().max_tool_calls,
        token_budget: agent.config().token_budget,
        agent_id: agent.config().name.clone(),
        session_id: session_id.clone(),
        model_caps: crate::core::r#loop::ModelCaps {
            thinking: agent.config().thinking,
            prompt_cache: model_config.prompt_cache,
            max_output_tokens: model_config.max_tokens.unwrap_or(4096),
        },
    };

    let outcome = crate::core::r#loop::run_simple_loop(
        agent.provider(),
        agent.tools(),
        agent.cache(),
        &loop_config,
        task,
        crate::core::r#loop::SimpleLoopContext {
            hook_engine: agent.hook_engine(),
            exec_policy: agent.exec_policy(),
            rollout,
            goal_manager: Some(goal_manager),
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
pub async fn run_onlyrun(task: String, sandbox: bool) -> crate::Result<()> {
    crate::logging::init_logging();

    let config = crate::config::OrionConfig::load();
    if sandbox {
        log_info!("gateway", "🔒 沙箱模式: 网络操作和 VCS 写操作已禁止");
    }
    let message = run_task_once(&task, &config, None, sandbox).await?;
    println!("{}", message);
    Ok(())
}
