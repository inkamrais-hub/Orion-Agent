//! MCP (Model Context Protocol) 客户端
//! 通过 stdio 连接 MCP server, JSON-RPC 2.0 协议

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

/// 池化的 MCP 客户端 — 持有请求通道和缓存的工具定义
pub struct McpPooledClient {
    request_tx: tokio::sync::mpsc::UnboundedSender<McpRequest>,
    tool_defs: Vec<McpToolDef>,
}

/// 全局 MCP 连接池 — 按 server name 去重，相同 name 只 spawn 一个子进程
static MCP_POOL: LazyLock<DashMap<String, Arc<McpPooledClient>>> =
    LazyLock::new(DashMap::new);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

pub struct McpClient {
    server_name: String,
    child: Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    request_id: u64,
}

impl McpClient {
    pub async fn connect(config: &McpServerConfig) -> crate::Result<Self> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| crate::Error::Agent(format!("MCP spawn {}: {}", config.name, e)))?;

        let stdin = child.stdin.take().ok_or_else(|| crate::Error::Agent("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| crate::Error::Agent("no stdout".into()))?;
        let stdout = BufReader::new(stdout);

        let mut client = Self { server_name: config.name.clone(), child, stdin, stdout, request_id: 0 };
        client.send_request("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"orion-agent","version":"0.1.0"}})).await?;
        client.send_notification("initialized", json!({})).await?;
        Ok(client)
    }

    async fn send_request(&mut self, method: &str, params: Value) -> crate::Result<Value> {
        self.request_id += 1;
        let request = json!({"jsonrpc":"2.0","id":self.request_id,"method":method,"params":params});
        let msg = format!("{}\n", serde_json::to_string(&request)?);
        self.stdin.write_all(msg.as_bytes()).await.map_err(|e| crate::Error::Agent(format!("MCP write: {}", e)))?;
        self.stdin.flush().await.ok();

        // 30 秒超时，防止 server 卡住
        let mut line = String::new();
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.stdout.read_line(&mut line),
        ).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Err(crate::Error::Agent(format!("MCP read: {}", e))),
            Err(_) => return Err(crate::Error::Agent(format!("MCP timeout (30s): {} no response", self.server_name))),
        }

        let resp: Value = serde_json::from_str(&line).map_err(|e| crate::Error::Agent(format!("MCP parse: {}", e)))?;
        if let Some(err) = resp.get("error") { return Err(crate::Error::Agent(format!("MCP error: {}", err))); }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> crate::Result<()> {
        let notification = json!({"jsonrpc":"2.0","method":method,"params":params});
        let msg = format!("{}\n", serde_json::to_string(&notification)?);
        self.stdin.write_all(msg.as_bytes()).await.map_err(|e| crate::Error::Agent(format!("MCP write: {}", e)))?;
        self.stdin.flush().await.ok();
        Ok(())
    }

    pub async fn list_tools(&mut self) -> crate::Result<Vec<McpToolDef>> {
        let result = self.send_request("tools/list", json!({})).await?;
        let tools = result.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        Ok(tools.into_iter().map(|t| McpToolDef {
            name: t["name"].as_str().unwrap_or("").to_string(),
            description: t["description"].as_str().unwrap_or("").to_string(),
            input_schema: t.get("inputSchema").cloned().unwrap_or(json!({})),
            server_name: self.server_name.clone(),
        }).collect())
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> crate::Result<String> {
        let result = self.send_request("tools/call", json!({"name":name,"arguments":arguments})).await?;
        let content = result.get("content").and_then(|c| c.as_array());
        match content {
            Some(blocks) => {
                let text: String = blocks.iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(if text.is_empty() { format!("{:?}", result) } else { text })
            }
            None => Ok(format!("{:?}", result)),
        }
    }

    pub async fn shutdown(&mut self) {
        let _ = self.send_request("shutdown", json!({})).await;
        let _ = self.send_notification("exit", json!({})).await;
        let _ = self.child.kill().await;
    }
}

#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub server_name: String,
}

pub struct McpToolProxy {
    def: McpToolDef,
    tx: tokio::sync::mpsc::UnboundedSender<McpRequest>,
}

pub struct McpRequest {
    pub tool_name: String,
    pub arguments: Value,
    pub response_tx: tokio::sync::oneshot::Sender<crate::Result<String>>,
}

impl McpToolProxy {
    pub fn new(def: McpToolDef, tx: tokio::sync::mpsc::UnboundedSender<McpRequest>) -> Self {
        Self { def, tx }
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str { &self.def.name }
    fn description(&self) -> &str { &self.def.description }
    fn input_schema(&self) -> Value { self.def.input_schema.clone() }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        self.tx.send(McpRequest { tool_name: self.def.name.clone(), arguments: input, response_tx: resp_tx })
            .map_err(|_| crate::Error::Agent("MCP channel closed".into()))?;
        match tokio::time::timeout(std::time::Duration::from_secs(60), resp_rx).await {
            Ok(Ok(Ok(content))) => Ok(ToolResult { content, is_error: false, metadata: None }),
            Ok(Ok(Err(e))) => Ok(ToolResult { content: format!("MCP error: {}", e), is_error: true, metadata: None }),
            Ok(Err(_)) => Ok(ToolResult { content: "MCP channel closed".into(), is_error: true, metadata: None }),
            Err(_) => Ok(ToolResult { content: format!("MCP tool '{}' timeout (60s)", self.def.name), is_error: true, metadata: None }),
        }
    }
}

pub async fn connect_mcp_server(
    config: &McpServerConfig,
    registry: &mut crate::tools::registry::ToolRegistry,
) -> crate::Result<()> {
    // 1. 从池中获取已有连接，不存在则首次创建
    let pooled = if let Some(existing) = MCP_POOL.get(&config.name) {
        existing.clone()
    } else {
        let mut client = McpClient::connect(config).await?;
        let tool_defs = client.list_tools().await?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<McpRequest>();

        // 后台任务：将池中所有 proxy 的请求转发到子进程
        tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                let result = client.call_tool(&req.tool_name, req.arguments).await;
                let _ = req.response_tx.send(result);
            }
            client.shutdown().await;
        });

        let p = Arc::new(McpPooledClient { request_tx: tx, tool_defs });
        MCP_POOL.insert(config.name.clone(), p.clone());
        p
    };

    // 2. 注册代理工具（复用池中的 request_tx）
    let tool_names: Vec<String> = pooled.tool_defs.iter().map(|d| d.name.clone()).collect();
    for def in &pooled.tool_defs {
        let proxy = McpToolProxy::new(def.clone(), pooled.request_tx.clone());
        registry.register(proxy);
    }

    // 3. 为该 MCP server 创建动态聚类，使 sub-agent 可按聚类授权
    let sanitized: String = config.name.chars().map(|c| {
        if c.is_alphanumeric() || c == '_' { c } else { '_' }
    }).collect();
    let cluster_name = format!("mcp_{}", sanitized);
    let brief = format!(
        "{} tool(s) from MCP server '{}'",
        tool_names.len(), config.name
    );
    registry.register_dynamic_cluster(&cluster_name, tool_names, &brief);

    tracing::info!(server = %config.name, cluster = %cluster_name, tools = pooled.tool_defs.len(), "MCP tools registered (pooled)");
    Ok(())
}
