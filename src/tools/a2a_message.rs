//! A2A 权限通信工具
//!
//! send_message: 向其他 Agent 发消息 (需权限)
//! list_peers: 查看可通信的 Agent 列表

use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};
use crate::agent::protocol::A2AMessage;

/// A2A 权限配置
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct A2APeerConfig {
    pub id: String,
    pub a2a_peers: Vec<String>,
}

/// 全局权限表
static A2A_PEERS: std::sync::LazyLock<std::sync::RwLock<Vec<A2APeerConfig>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(Vec::new()));

/// 初始化权限表
pub fn init_a2a_peers(configs: Vec<A2APeerConfig>) {
    let mut peers = A2A_PEERS.write().unwrap();
    *peers = configs;
}

/// 检查 from 是否有权给 to 发消息
pub fn has_permission(from: &str, to: &str) -> bool {
    let peers = A2A_PEERS.read().unwrap();
    // 1. 精确匹配
    if let Some(cfg) = peers.iter().find(|p| p.id == from) {
        if cfg.a2a_peers.contains(&to.to_string()) { return true; }
    }
    // 2. 通配符
    if let Some(cfg) = peers.iter().find(|p| p.id == from) {
        if cfg.a2a_peers.contains(&"*".to_string()) { return true; }
    }
    // 3. 默认允许 (没有配置 = 开放)
    if peers.is_empty() { return true; }
    false
}

/// 获取 from 的 peers 列表
pub fn get_peers(from: &str) -> Vec<String> {
    let peers = A2A_PEERS.read().unwrap();
    if peers.is_empty() {
        return vec!["*".to_string()]; // 没配置 = 全部可通信
    }
    peers.iter().find(|p| p.id == from).map(|p| p.a2a_peers.clone()).unwrap_or_default()
}

// ============================================================
//  send_message 工具
// ============================================================

pub struct SendMessageTool;

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str { "send_message" }
    fn description(&self) -> &str {
        "Send a message to another agent. The agent will process your request and respond.          Only works if the target has granted you permission (a2a_peers in config).          Use list_peers to see who you can talk to. Use for: asking other agents for help,          sharing results, requesting information."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": {"type": "string", "description": "Target agent ID"},
                "message": {"type": "string", "description": "Message content"}
            },
            "required": ["to", "message"]
        })
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let to = input["to"].as_str().ok_or_else(||crate::Error::Tool("missing 'to'".into()))?;
        let message = input["message"].as_str().ok_or_else(||crate::Error::Tool("missing 'message'".into()))?;

        // 权限检查
        if !has_permission(&ctx.agent_id, to) {
            return Ok(ToolResult {
                content: format!("Agent '{}' has not granted you communication permission. Use list_peers to see available agents.", to),
                is_error: true,
                metadata: None,
            });
        }

        // 通过 registry 发送 (如果有)
        if let Some(ref registry) = ctx.registry {
            let a2a = A2AMessage::RequestInfo {
                from: ctx.agent_id.clone(),
                query: message.to_string(),
            };
            match registry.send_a2a(&to.to_string(), a2a).await {
                Ok(_) => Ok(ToolResult {
                    content: format!("Message sent to '{}'. The agent will process it when available.", to),
                    is_error: false,
                    metadata: None,
                }),
                Err(e) => Ok(ToolResult {
                    content: format!("Failed to send to '{}': {}", to, e),
                    is_error: true,
                    metadata: None,
                }),
            }
        } else {
            Ok(ToolResult {
                content: "A2A registry not available in this context".into(),
                is_error: true,
                metadata: None,
            })
        }
    }
}

// ============================================================
//  list_peers 工具
// ============================================================

pub struct ListPeersTool;

#[async_trait]
impl Tool for ListPeersTool {
    fn name(&self) -> &str { "list_peers" }
    fn description(&self) -> &str {
        "List agents you can send messages to. Use before send_message to check availability."
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let peers = get_peers(&ctx.agent_id);
        if peers.is_empty() {
            Ok(ToolResult { content: "No peers configured. You cannot send messages to other agents.".into(), is_error: false, metadata: None })
        } else if peers.contains(&"*".to_string()) {
            Ok(ToolResult { content: "All agents are available (open mode).".into(), is_error: false, metadata: None })
        } else {
            Ok(ToolResult { content: serde_json::to_string_pretty(&peers).unwrap_or_default(), is_error: false, metadata: None })
        }
    }
}