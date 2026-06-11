//! REST API + SSE 流式服务
//!
//! 端点:
//! - GET    /api/health         - 健康检查
//! - GET    /api/agents         - 列出所有 Agent 配置
//! - POST   /api/agents         - 创建 Agent 配置
//! - GET    /api/agents/{id}    - 获取单个 Agent 配置
//! - PUT    /api/agents/{id}    - 更新 Agent 配置
//! - DELETE /api/agents/{id}    - 删除 Agent 配置
//! - GET    /api/tools          - 列出可用工具
//! - POST   /api/chat           - SSE 流式对话
//! - POST   /api/sessions/{id}/rollback - Session 回滚

use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::stream::{self, Stream};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;

use crate::session::unified::AgentConfigModel;
use crate::session::UnifiedStore;
use crate::config::OrionConfig;
use crate::core::agent::{Agent, AgentEvent};
use crate::core::provider::Provider;
use crate::core::providers;
use crate::tools::registry::ToolRegistry;

// ============================================================
//  认证配置
// ============================================================

/// API 认证配置
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// 是否启用认证 (默认 false, 开发模式)
    pub enabled: bool,
    /// API Key (环境变量 ORION_API_KEY 或配置文件)
    pub api_key: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: std::env::var("ORION_API_KEY").ok(),
        }
    }
}

// ============================================================
//  限流器
// ============================================================

/// 简易内存限流器 (按 IP)
pub struct RateLimiter {
    requests: dashmap::DashMap<String, (std::time::Instant, std::sync::atomic::AtomicU64)>,
    max_per_minute: u64,
}

impl RateLimiter {
    /// 创建限流器，指定每分钟最大请求数
    pub fn new(max_per_minute: u64) -> Self {
        Self {
            requests: dashmap::DashMap::new(),
            max_per_minute,
        }
    }

    /// 检查是否允许请求通过 (返回 true 表示允许)
    pub fn check(&self, key: &str) -> bool {
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(60);

        let mut entry = self.requests.entry(key.to_string()).or_insert_with(|| {
            (now, std::sync::atomic::AtomicU64::new(0))
        });

        let (window_start, counter) = entry.value();
        if now.duration_since(*window_start) > window {
            // Reset window
            *entry = (now, std::sync::atomic::AtomicU64::new(1));
            true
        } else {
            let count = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            count < self.max_per_minute
        }
    }
}

// ============================================================
//  共享状态
// ============================================================

/// API 共享状态，通过 `Arc` 在所有 handler 间共享
pub struct ApiState {
    pub store: Arc<UnifiedStore>,
    pub config: OrionConfig,
    pub auth_config: AuthConfig,
    pub rate_limiter: Arc<RateLimiter>,
}

// ============================================================
//  Router
// ============================================================

/// 创建 API 路由器（所有端点 + 共享状态 + 中间件）
pub fn create_router(state: Arc<ApiState>) -> Router {
    let ui_path = if std::path::Path::new("../orion-ui").exists() {
        "../orion-ui"
    } else if std::path::Path::new("orion-ui").exists() {
        "orion-ui"
    } else {
        "../orion-ui"
    };

    let auth_config = Arc::new(state.auth_config.clone());
    let rate_limiter = state.rate_limiter.clone();

    Router::new()
        .route("/api/health", get(health))
        .route("/api/agents", get(list_agents).post(create_agent))
        .route("/api/agents/{id}", get(get_agent).put(update_agent).delete(delete_agent))
        .route("/api/tools", get(list_tools))
        .route("/api/chat", post(chat))
        .route("/api/sessions/{id}/rollback", post(rollback_session))
        .fallback_service(tower_http::services::ServeDir::new(ui_path))
        .layer(middleware::from_fn(move |req, next| {
            let auth = Arc::clone(&auth_config);
            async move { auth_middleware(auth, req, next).await }
        }))
        .layer(middleware::from_fn(move |req, next| {
            let limiter = Arc::clone(&rate_limiter);
            async move { rate_limit_middleware(limiter, req, next).await }
        }))
        .with_state(state)
}

// ============================================================
//  请求体类型
// ============================================================

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    agent_id: Option<String>,
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct RollbackRequest {
    turn_index: u32,
}

#[derive(Deserialize)]
struct CreateAgentRequest {
    name: String,
    model: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default = "default_tools_json")]
    tools_json: String,
    #[serde(default = "default_mcp_json")]
    mcp_servers_json: String,
    #[serde(default = "default_max_turns")]
    max_turns: i64,
    #[serde(default = "default_max_tool_calls")]
    max_tool_calls: i64,
    #[serde(default = "default_token_budget")]
    token_budget: i64,
    #[serde(default)]
    thinking: bool,
    #[serde(default = "default_reasoning_effort")]
    reasoning_effort: String,
}

fn default_tools_json() -> String {
    "[]".into()
}
fn default_mcp_json() -> String {
    "[]".into()
}
fn default_max_turns() -> i64 {
    20
}
fn default_max_tool_calls() -> i64 {
    30
}
fn default_token_budget() -> i64 {
    128_000
}
fn default_reasoning_effort() -> String {
    "medium".into()
}

// ============================================================
//  端点实现
// ============================================================

/// GET /api/health
async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/agents — 列出全部 Agent 配置
async fn list_agents(
    State(state): State<Arc<ApiState>>,
) -> Result<impl IntoResponse, StatusCode> {
    state
        .store
        .list_agents()
        .await
        .map(|agents| Json(serde_json::json!(agents)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// POST /api/agents — 创建 Agent 配置
async fn create_agent(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let now = chrono::Utc::now().to_rfc3339();
    let model = AgentConfigModel {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name,
        model: body.model,
        system_prompt: body.system_prompt,
        tools_json: body.tools_json,
        mcp_servers_json: body.mcp_servers_json,
        max_turns: body.max_turns,
        max_tool_calls: body.max_tool_calls,
        token_budget: body.token_budget,
        thinking: body.thinking,
        reasoning_effort: body.reasoning_effort,
        created_at: now.clone(),
        updated_at: now,
    };

    state
        .store
        .create_agent(&model)
        .await
        .map(|()| (StatusCode::CREATED, Json(serde_json::json!(model))))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/agents/{id} — 获取单个 Agent 配置
async fn get_agent(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    match state.store.get_agent(&id).await {
        Ok(Some(agent)) => Ok(Json(serde_json::json!(agent))),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// DELETE /api/agents/{id} — 删除 Agent 配置
async fn delete_agent(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match state.store.delete_agent(&id).await {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// PUT /api/agents/{id} — 更新 Agent 配置
async fn update_agent(
    State(state): State<Arc<ApiState>>,
    Path(id): Path<String>,
    Json(body): Json<CreateAgentRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let existing = state.store.get_agent(&id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if existing.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    let model = AgentConfigModel {
        id: id.clone(),
        name: body.name,
        model: body.model,
        system_prompt: body.system_prompt,
        tools_json: body.tools_json,
        mcp_servers_json: body.mcp_servers_json,
        max_turns: body.max_turns,
        max_tool_calls: body.max_tool_calls,
        token_budget: body.token_budget,
        thinking: body.thinking,
        reasoning_effort: body.reasoning_effort,
        created_at: existing.unwrap().created_at,
        updated_at: String::new(), // store.update 会自动填充
    };

    state
        .store
        .update_agent(&id, &model)
        .await
        .map(|_| Json(serde_json::json!(model)))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// GET /api/tools — 列出全部已注册的内置工具
async fn list_tools() -> impl IntoResponse {
    let mut registry = ToolRegistry::new();
    crate::tools::register_default_tools(&mut registry);
    Json(serde_json::json!({ "tools": registry.definitions() }))
}

/// POST /api/chat — SSE 流式对话（核心端点）
///
/// 请求体:
/// ```json
/// { "agent_id": "xxx", "message": "你好", "session_id": "xxx" }
/// ```
///
/// SSE 事件类型: thinking, text, tool_start, tool_end, turn_complete, done, error
async fn chat(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // 1. 获取 Agent 配置（有 agent_id 从 store 读取，否则用默认配置）
    let agent_config = match &body.agent_id {
        Some(id) => match state.store.get_agent(id).await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Err(StatusCode::NOT_FOUND),
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        },
        None => default_agent_config(&state.config),
    };

    // 2. 动态创建 Agent
    let agent = build_agent_from_config(&agent_config, &state.config)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 3. 启动流式对话，拿到事件接收器
    let rx = agent
        .chat_stream(&body.message, body.session_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 4. 将 UnboundedReceiver<AgentEvent> 转换为 SSE 流
    let sse_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let (event_type, data) = agent_event_to_sse(&event);
                let sse_event = Event::default().event(event_type).data(data);
                Some((Ok::<_, Infallible>(sse_event), rx))
            }
            None => None,
        }
    });

    Ok(Sse::new(sse_stream))
}

/// POST /api/sessions/{id}/rollback — 回滚 Session 到指定轮次
///
/// 请求体:
/// ```json
/// { "turn_index": 3 }
/// ```
///
/// 响应:
/// ```json
/// { "restored_files": ["src/main.rs", "src/lib.rs"] }
/// ```
async fn rollback_session(
    State(state): State<Arc<ApiState>>,
    Path(session_id): Path<String>,
    Json(body): Json<RollbackRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // 1. 获取需要回滚的动作列表
    let actions = state
        .store
        .rollback_to_turn(&session_id, body.turn_index)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut restored_files = Vec::new();

    // 2. 按动作执行文件恢复（后改的先恢复，actions 已按 DESC 排序）
    for action in &actions {
        match &action.content_before {
            Some(content) => {
                // 写回文件原始内容
                if let Err(e) = std::fs::write(&action.target_path, content) {
                    tracing::warn!(
                        path = %action.target_path,
                        error = %e,
                        "Failed to restore file during rollback"
                    );
                    continue;
                }
            }
            None => {
                // 文件是本轮新建的，删除它
                if std::path::Path::new(&action.target_path).exists() {
                    if let Err(e) = std::fs::remove_file(&action.target_path) {
                        tracing::warn!(
                            path = %action.target_path,
                            error = %e,
                            "Failed to delete newly-created file during rollback"
                        );
                        continue;
                    }
                }
            }
        }
        restored_files.push(action.target_path.clone());
    }

    // 3. 清理已被回滚的快照
    state
        .store
        .cleanup_snapshots_after(&session_id, body.turn_index)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(serde_json::json!({
        "restored_files": restored_files,
    })))
}

// ============================================================
//  中间件
// ============================================================

/// API Key 认证中间件
///
/// 检查 `X-API-Key` 或 `Authorization: Bearer` 头中的 API Key。
/// 当 `AuthConfig::enabled` 为 false 或未配置 API Key 时，直接放行。
async fn auth_middleware(
    auth_config: Arc<AuthConfig>,
    req: Request,
    next: Next,
) -> Response {
    if !auth_config.enabled {
        return next.run(req).await;
    }

    let expected_key = match &auth_config.api_key {
        Some(key) if !key.is_empty() => key.clone(),
        _ => return next.run(req).await, // No key configured = open access
    };

    // Check X-API-Key header
    if let Some(key) = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok()) {
        if key == expected_key {
            return next.run(req).await;
        }
    }

    // Check Authorization: Bearer header
    if let Some(auth) = req.headers().get("Authorization").and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            if token == expected_key {
                return next.run(req).await;
            }
        }
    }

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("Unauthorized"))
        .unwrap()
}

/// 简易 IP 限流中间件
///
/// 基于 `X-Forwarded-For` / `X-Real-IP` 头识别客户端 IP，
/// 使用内存滑窗限流。未配置限流器时直接放行。
async fn rate_limit_middleware(
    rate_limiter: Arc<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = req
        .headers()
        .get("X-Forwarded-For")
        .or_else(|| req.headers().get("X-Real-IP"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    if rate_limiter.check(&ip) {
        next.run(req).await
    } else {
        Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(Body::from("Rate limit exceeded"))
            .unwrap()
    }
}

// ============================================================
//  辅助函数
// ============================================================

/// 无 agent_id 时使用的默认配置
fn default_agent_config(config: &OrionConfig) -> AgentConfigModel {
    let default_model = config.active_model();
    AgentConfigModel {
        id: "default".into(),
        name: "default".into(),
        model: default_model.name,
        system_prompt: String::new(),
        tools_json: "[]".into(),
        mcp_servers_json: "[]".into(),
        max_turns: config.agent.max_turns as i64,
        max_tool_calls: config.agent.max_tool_calls as i64,
        token_budget: config.agent.token_budget as i64,
        thinking: false,
        reasoning_effort: "medium".into(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// 根据 AgentConfigModel + OrionConfig 动态构建 Agent 实例
///
/// 解析 tools_json 选择性注册工具，解析 mcp_servers_json 加载 MCP 工具
async fn build_agent_from_config(
    agent_config: &AgentConfigModel,
    app_config: &OrionConfig,
) -> crate::Result<Agent> {
    // 1. 查找模型配置：优先名称匹配，回退到第一个
    let model_config = app_config
        .models
        .iter()
        .find(|m| m.name == agent_config.model)
        .or_else(|| app_config.models.first())
        .ok_or_else(|| crate::Error::Config("No models configured".into()))?;

    // 2. 创建 Provider (根据 config.provider 字段路由)
    let provider: Arc<dyn Provider> = Arc::from(providers::create_provider(model_config));

    // 3. 按 tools_json 选择性注册内置工具
    let mut tools = ToolRegistry::new();
    let requested_tools: Vec<String> = serde_json::from_str(&agent_config.tools_json)
        .unwrap_or_default();

    if requested_tools.is_empty() {
        // 空数组 = 注册全部默认工具
        crate::tools::register_default_tools(&mut tools);
    } else {
        register_tools_by_name(&mut tools, &requested_tools);
    }

    // 4. 加载 MCP 工具（connect_mcp_server 内部使用全局连接池去重，同名 server 只 spawn 一次）
    let mcp_configs: Vec<crate::tools::mcp::McpServerConfig> =
        serde_json::from_str(&agent_config.mcp_servers_json).unwrap_or_default();
    for mcp_cfg in &mcp_configs {
        if let Err(e) = crate::tools::mcp::connect_mcp_server(mcp_cfg, &mut tools).await {
            tracing::warn!(server = %mcp_cfg.name, error = %e, "MCP server failed to connect");
        }
    }

    // 5. 构建 Agent
    Agent::builder()
        .name(&agent_config.name)
        .model(&agent_config.model)
        .system_prompt(&agent_config.system_prompt)
        .provider(provider)
        .tools(tools)
        .max_turns(agent_config.max_turns as u64)
        .max_tool_calls(agent_config.max_tool_calls as u64)
        .token_budget(agent_config.token_budget as u64)
        .thinking(agent_config.thinking)
        .reasoning_effort(&agent_config.reasoning_effort)
        .build()
}

/// 按名称列表选择性注册内置工具
fn register_tools_by_name(registry: &mut ToolRegistry, names: &[String]) {
    use crate::tools::*;
    for name in names {
        match name.as_str() {
            "read" => registry.register(ReadTool),
            "write" => registry.register(WriteTool),
            "bash" => registry.register(BashTool),
            "edit" => registry.register(edit::EditTool),
            "symbol_search" => registry.register(code_intelligence::SymbolSearchTool),
            "find_callers" => registry.register(code_intelligence::FindCallersTool),
            "project_map" => registry.register(code_intelligence::ProjectMapTool),
            "ask_user" => registry.register(ask_user::AskUserTool),
            "glob" => registry.register(glob_tool::GlobTool),
            "grep" => registry.register(grep_tool::GrepTool),
            "skeleton" => registry.register(skeleton_tool::SkeletonTool),
            other => {
                tracing::warn!(tool = other, "Unknown tool name, skipped");
            }
        }
    }
}

/// 将 AgentEvent 转换为 (SSE 事件名, JSON 数据字符串)
fn agent_event_to_sse(event: &AgentEvent) -> (&'static str, String) {
    let (ty, val) = match event {
        AgentEvent::Thinking(text) => ("thinking", serde_json::json!({ "text": text })),
        AgentEvent::Text(text) => ("text", serde_json::json!({ "text": text })),
        AgentEvent::ToolStart { name, input } => {
            ("tool_start", serde_json::json!({ "name": name, "input": input }))
        }
        AgentEvent::ToolEnd {
            name,
            result,
            success,
            duration_ms,
        } => (
            "tool_end",
            serde_json::json!({
                "name": name,
                "result": result,
                "success": success,
                "duration_ms": duration_ms,
            }),
        ),
        AgentEvent::TurnComplete { turn } => {
            ("turn_complete", serde_json::json!({ "turn": turn }))
        }
        AgentEvent::Done {
            message,
            input_tokens,
            output_tokens,
        } => (
            "done",
            serde_json::json!({
                "message": message,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            }),
        ),
        AgentEvent::Error(msg) => ("error", serde_json::json!({ "message": msg })),
    };
    (ty, serde_json::to_string(&val).unwrap_or_default())
}
