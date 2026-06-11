//! 持久化聊天循环
//!
//! 启动 → 自动恢复 session → 循环对话 → 退出时保存

use super::commands::SlashRegistry;
use crate::audit::{AuditEvent as GlobalAuditEvent, AUDIT_LOGGER};
use crate::cli::execute::execute_turn;
use crate::config::OrionConfig;
use crate::core::cache::GlobalCache;
use crate::core::provider::Provider;
use crate::core::providers;
use crate::session::memory::{extract_memories, SessionMemory};
use crate::session::UnifiedStore;
use crate::tools::a2a_message::{ListPeersTool, SendMessageTool};
use crate::tools::agent_tool::SubAgentTool;
use crate::tools::registry::ToolRegistry;
use std::sync::Arc;

pub struct ChatState {
    pub store: Arc<UnifiedStore>,
    pub current_session: String,
    pub provider: Box<dyn Provider>,
    pub model: String,
    pub tools: ToolRegistry,
    pub cache: GlobalCache,
    pub memory: SessionMemory,
    /// 待发送的图片列表 (Message Block 方案)
    pub pending_images: Vec<crate::core::provider::ContentBlock>,
    /// 思考模式开关
    pub thinking: bool,
    /// 思考深度: low/medium/high/max/xhigh
    pub reasoning_effort: String,
    /// 当前工作目录
    pub working_dir: String,
}

/// 注册全部工具
fn register_all_tools(config: &OrionConfig) -> (ToolRegistry, GlobalCache) {
    let mut tools = ToolRegistry::new();
    crate::tools::register_default_tools(&mut tools);
    // chat 专用工具
    tools.register(SubAgentTool::new());
    tools.register(SendMessageTool);
    tools.register(ListPeersTool);

    let cache = GlobalCache::from_config(&config.cache);
    (tools, cache)
}

/// 创建新 session 的辅助函数
async fn create_session_helper(store: &Arc<UnifiedStore>, model: &str) -> crate::Result<String> {
    let session_id = crate::session::store::generate_session_id();
    let now = chrono::Utc::now().to_rfc3339();
    store
        .create_session(&crate::session::unified::SessionMeta {
            session_id: session_id.clone(),
            agent_name: String::new(),
            model: model.to_string(),
            working_dir: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            status: crate::session::unified::SessionStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        })
        .await?;
    tracing::info!(session_id = %session_id, model = %model, "Session created");
    Ok(session_id)
}

/// 启动对话
pub async fn run(config: OrionConfig, working_dir: String) -> crate::Result<()> {
    let model_config = config.active_model();
    let store = UnifiedStore::open().await?;

    // 恢复或创建 session
    let sessions = store.list_sessions(50).await?;
    let session_id = if config.cli.auto_resume {
        if let Some(latest) = sessions.iter().max_by_key(|s| s.updated_at.clone()) {
            tracing::info!(session = %latest.session_id, "Resuming session");
            eprintln!("⚡ Resuming session {}", &latest.session_id[..8]);
            latest.session_id.clone()
        } else {
            let id = create_session_helper(&store, &model_config.name).await?;
            eprintln!("⚡ New session {}", &id[..8]);
            id
        }
    } else {
        let id = create_session_helper(&store, &model_config.name).await?;
        eprintln!("⚡ New session {}", &id[..8]);
        id
    };

    let provider: Box<dyn Provider> = providers::create_provider(&model_config);

    let (mut tools, cache) = register_all_tools(&config);

    // 连接配置中的 MCP server 并注入工具
    crate::tools::mcp_init::init_mcp_tools(&config, &mut tools).await;

    let memory = SessionMemory::for_project(&working_dir);

    let mut state = ChatState {
        store,
        current_session: session_id,
        provider,
        model: model_config.name.clone(),
        tools,
        cache,
        memory,
        pending_images: Vec::new(),
        thinking: model_config.thinking,
        reasoning_effort: "medium".into(),
        working_dir,
    };

    // 构建 system prompt，注入 memory 上下文
    let mut system_prompt = crate::cli::execute::build_system_prompt(&state.tools);
    let memory_ctx = state.memory.as_context();
    if !memory_ctx.is_empty() {
        system_prompt.push('\n');
        system_prompt.push_str(&memory_ctx);
        system_prompt.push('\n');
    }

    eprintln!("⚡ Orion Agent ready | model: {} | tools: {} | session: {} | memories: {}",
        state.model, state.tools.len(), &state.current_session[..8], state.memory.len());
    eprintln!("  Type /help for commands, /exit to quit\n");

    // 审计: 会话开始
    {
        let mut logger = AUDIT_LOGGER.lock().await;
        logger.log_with_session(
            GlobalAuditEvent::SessionStart {
                session_id: state.current_session.clone(),
                model: state.model.clone(),
            },
            "chat",
            &state.current_session,
        );
    }
    let session_start_time = std::time::Instant::now();

    // 共享的 slash 命令注册表
    let slash_registry = SlashRegistry::default();

    // chat 循环
    loop {
        eprint!("{}", config.cli.prompt);
        use std::io::Write;
        std::io::stderr().flush().ok();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() || input.is_empty() {
            break;
        }
        let input = input.trim();
        if input.is_empty() { continue; }

        // 命令路由
        if input.starts_with('/') {
            match handle_command(input, &mut state, &config, &slash_registry).await {
                CmdResult::Continue => continue,
                CmdResult::Exit => break,
                CmdResult::Error(msg) => { eprintln!("❌ {}", msg); continue; }
            }
        }

        // 执行对话 (含工具循环, 流式输出)
        // 取出待发送图片 (一次性消费)
        let images = if state.pending_images.is_empty() {
            None
        } else {
            let imgs = std::mem::take(&mut state.pending_images);
            eprintln!("📎 附带 {} 张图片", imgs.len());
            Some(imgs)
        };
        match execute_turn(
            &*state.provider, &state.tools, &state.cache,
            &state.store, &state.current_session, input, &state.model,
            &system_prompt, None, images,
            state.thinking, &state.reasoning_effort,
            Some(&state.working_dir),
        ).await {
            Ok(response) => {
                // 从本轮对话中提取记忆
                let new_memories = extract_memories(input, &response, &state.current_session);
                for (cat, content) in new_memories {
                    state.memory.add(cat, content, 0.8, &state.current_session);
                }
                // 文本已在流式输出中显示，这里只换行
                eprintln!();
            }
            Err(e) => eprintln!("❌ Error: {}", e),
        }
    }

    // 退出时保存记忆
    if let Err(e) = state.memory.save() {
        tracing::warn!("Failed to save memories: {}", e);
    }

    // 审计: 会话结束
    {
        let duration_ms = session_start_time.elapsed().as_millis() as u64;
        let mut logger = AUDIT_LOGGER.lock().await;
        logger.log_with_session(
            GlobalAuditEvent::SessionEnd {
                session_id: state.current_session.clone(),
                duration_ms,
            },
            "chat",
            &state.current_session,
        );
        logger.flush();
    }

    eprintln!("👋 Session saved. Goodbye!");
    Ok(())
}

enum CmdResult {
    Continue,
    Exit,
    Error(String),
}

async fn handle_command(
    input: &str,
    state: &mut ChatState,
    _config: &OrionConfig,
    slash_registry: &SlashRegistry,
) -> CmdResult {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim());

    match cmd {
        "/help" => {
            eprintln!("命令:");
            eprintln!("  /help              显示帮助");
            eprintln!("  /new [name]        创建新会话");
            eprintln!("  /list              列出所有会话");
            eprintln!("  /resume <id>       切换会话");
            eprintln!("  /drop <id>         删除会话");
            eprintln!("  /clear             清屏");
            eprintln!("  /cd [path]         查看/切换工作目录");
            eprintln!("  /model [name]      查看/切换模型");
            eprintln!("  /think             开关思考模式");
            eprintln!("  /think-level [lvl] 设置思考深度 (low/medium/high/max/xhigh)");
            eprintln!("  /history [n]       查看对话历史");
            eprintln!("  /memory            查看记忆");
            eprintln!("  /image <path>      添加图片");
            eprintln!("  /images            查看待发送图片");
            eprintln!("  /clear-images      清空图片");
            eprintln!("  /status            查看状态");
            eprintln!("  /exit              退出");
        }
        "/image" => {
            match arg {
                Some(path) => {
                    let img_path = std::path::Path::new(path);
                    if !img_path.exists() {
                        return CmdResult::Error(format!("图片文件不存在: {}", path));
                    }
                    let data = match std::fs::read(img_path) {
                        Ok(d) => d,
                        Err(e) => return CmdResult::Error(format!("读取图片失败: {}", e)),
                    };
                    if data.len() > 20 * 1024 * 1024 {
                        return CmdResult::Error(format!("图片文件过大 ({}MB > 20MB)", data.len() / 1024 / 1024));
                    }
                    let media_type = match img_path.extension().and_then(|e| e.to_str()) {
                        Some("png") => "image/png",
                        Some("jpg") | Some("jpeg") => "image/jpeg",
                        Some("gif") => "image/gif",
                        Some("webp") => "image/webp",
                        Some("bmp") => "image/bmp",
                        _ => "image/png",
                    };
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    state.pending_images.push(crate::core::provider::ContentBlock::Image {
                        data: b64,
                        media_type: media_type.to_string(),
                    });
                    eprintln!("📎 Image added: {} ({}KB, {})", path, data.len() / 1024, media_type);
                    eprintln!("   已添加 {} 张图片，下次消息将自动附带", state.pending_images.len());
                }
                None => return CmdResult::Error("用法: /image <图片路径>".into()),
            }
        }
        "/images" => {
            if state.pending_images.is_empty() {
                eprintln!("没有待发送的图片");
            } else {
                eprintln!("待发送图片: {} 张", state.pending_images.len());
                eprintln!("下次消息将自动附带，发送后自动清空");
            }
        }
        "/clear-images" => {
            let count = state.pending_images.len();
            state.pending_images.clear();
            eprintln!("已清空 {} 张待发送图片", count);
        }
        "/cd" => {
            match arg {
                Some(path) if !path.is_empty() => {
                    let new_dir = std::path::Path::new(path);
                    if !new_dir.exists() {
                        return CmdResult::Error(format!("目录不存在: {}", path));
                    }
                    if !new_dir.is_dir() {
                        return CmdResult::Error(format!("不是目录: {}", path));
                    }
                    match std::env::set_current_dir(new_dir) {
                        Ok(()) => {
                            let new_cwd = std::env::current_dir()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|_| path.to_string());
                            state.working_dir = new_cwd.clone();
                            // 重新加载项目级记忆
                            state.memory = SessionMemory::for_project(&new_cwd);
                            eprintln!("工作目录: {}", new_cwd);
                        }
                        Err(e) => return CmdResult::Error(format!("无法切换目录: {}", e)),
                    }
                }
                _ => {
                    eprintln!("工作目录: {}", state.working_dir);
                }
            }
        }
        "/new" => {
            match create_session_helper(&state.store, &state.model).await {
                Ok(id) => {
                    state.current_session = id.clone();
                    eprintln!("⚡ New session: {}", &id[..8]);
                }
                Err(e) => return CmdResult::Error(format!("Create session: {}", e)),
            }
        }
        "/list" => {
            match state.store.list_sessions(50).await {
                Ok(sessions) => {
                    if sessions.is_empty() {
                        eprintln!("No sessions.");
                    } else {
                        for (i, s) in sessions.iter().enumerate() {
                            let marker = if s.session_id == state.current_session { " ← current" } else { "" };
                            eprintln!("  [{}] {} ({} turns, {}){}", i, &s.session_id[..8], s.turn_count, s.status.as_str(), marker);
                        }
                    }
                }
                Err(e) => return CmdResult::Error(format!("List sessions: {}", e)),
            }
        }
        "/resume" => {
            match arg {
                Some(id_prefix) => {
                    let sessions = state.store.list_sessions(50).await.unwrap_or_default();
                    if let Some(s) = sessions.iter().find(|s| s.session_id.starts_with(id_prefix)) {
                        state.current_session = s.session_id.clone();
                        eprintln!("⚡ Resumed: {}", &s.session_id[..8]);
                    } else {
                        return CmdResult::Error(format!("Session '{}' not found", id_prefix));
                    }
                }
                None => return CmdResult::Error("Usage: /resume <id>".into()),
            }
        }
        "/drop" => {
            match arg {
                Some(id_prefix) => {
                    let sessions = state.store.list_sessions(50).await.unwrap_or_default();
                    if let Some(s) = sessions.iter().find(|s| s.session_id.starts_with(id_prefix)) {
                        let sid = s.session_id.clone();
                        state.store.delete_session(&sid).await.ok();
                        eprintln!("🗑 Dropped: {}", &sid[..8]);
                        if sid == state.current_session {
                            match create_session_helper(&state.store, &state.model).await {
                                Ok(new_id) => {
                                    state.current_session = new_id.clone();
                                    eprintln!("⚡ Auto-created new session: {}", &new_id[..8]);
                                }
                                Err(e) => return CmdResult::Error(format!("Create fallback: {}", e)),
                            }
                        }
                    } else {
                        return CmdResult::Error(format!("Session '{}' not found", id_prefix));
                    }
                }
                None => return CmdResult::Error("Usage: /drop <id>".into()),
            }
        }
        "/think" => {
            state.thinking = !state.thinking;
            if state.thinking {
                eprintln!("✓ 思考模式: 开启");
                eprintln!("  模型将进行深度推理后再回答");
                eprintln!("  使用 /think-level <level> 调整深度");
            } else {
                eprintln!("✗ 思考模式: 关闭");
                eprintln!("  模型将直接回答，不进行深度推理");
            }
        }
        "/think-level" => {
            match arg {
                Some(level) => {
                    let valid_levels = ["low", "medium", "high", "max", "xhigh"];
                    if valid_levels.contains(&level) {
                        state.reasoning_effort = level.to_string();
                        eprintln!("✓ 思考深度: {}", level);
                        if !state.thinking {
                            eprintln!("  提示: 思考模式当前关闭，使用 /think 开启");
                        }
                    } else {
                        return CmdResult::Error(format!("无效深度 '{}'。可选: {}", level, valid_levels.join(", ")));
                    }
                }
                None => {
                    eprintln!("当前思考深度: {}", state.reasoning_effort);
                    eprintln!();
                    eprintln!("可用级别:");
                    eprintln!("  low    - 快速思考，节省 token");
                    eprintln!("  medium - 平衡模式 (默认)");
                    eprintln!("  high   - 深度思考");
                    eprintln!("  max    - 最大深度");
                    eprintln!("  xhigh  - 极限深度，消耗大量 token");
                    eprintln!();
                    eprintln!("用法: /think-level <level>");
                }
            }
        }
        "/model" => {
            match arg {
                Some(model_name) => {
                    // 切换模型
                    let config = OrionConfig::load();
                    if let Some(model_config) = config.models.iter().find(|m| m.name == model_name) {
                        // 创建新的 Provider (根据 config.provider 字段路由)
                        let new_provider: Box<dyn crate::core::provider::Provider> =
                            crate::core::providers::create_provider(model_config);
                        state.provider = new_provider;
                        state.model = model_config.name.clone();
                        eprintln!("✓ 已切换到: {}", model_name);
                        eprintln!("  endpoint: {}", model_config.endpoint);
                        eprintln!("  thinking: {}", model_config.thinking);
                        eprintln!("  vision:   {}", model_config.supports_vision());
                    } else {
                        return CmdResult::Error(format!("模型 '{}' 不存在。使用 /model 查看可用模型", model_name));
                    }
                }
                None => {
                    // 显示当前模型和可用模型列表
                    let config = OrionConfig::load();
                    eprintln!("当前模型: {}", state.model);
                    eprintln!();
                    eprintln!("可用模型:");
                    for m in &config.models {
                        let marker = if m.name == state.model { " ← current" } else { "" };
                        let mut flags = Vec::new();
                        if m.thinking { flags.push("thinking"); }
                        if m.supports_vision() { flags.push("vision"); }
                        let flags_str = if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) };
                        eprintln!("  {}{}{}", m.name, flags_str, marker);
                    }
                    eprintln!();
                    eprintln!("用法: /model <name> 切换模型");
                }
            }
        }
        "/exit" | "/quit" => return CmdResult::Exit,
        _ => {
            // 通用斜杠命令注册表 (chat/commands.rs 处理 /clear, /status, /history, /memory, /sessions, /delete, /restore, /trash)
            match slash_registry.handle(input).await {
                Some(response) => {
                    if !response.is_empty() {
                        eprintln!("{}", response);
                    }
                }
                None => return CmdResult::Error(format!("Unknown command: {}", cmd)),
            }
        }
    }
    CmdResult::Continue
}
