//! MCP 工具初始化 — 从配置连接 MCP server 并注入工具

/// 从配置初始化 MCP 工具
///
/// 遍历 `config.mcp_servers`，逐个连接并注册工具。
/// 单个 server 连接失败不会阻止 Agent 启动（仅 warn 日志）。
pub async fn init_mcp_tools(
    config: &crate::config::OrionConfig,
    registry: &mut crate::tools::registry::ToolRegistry,
) {
    for server_config in &config.mcp_servers {
        match crate::tools::mcp::connect_mcp_server(server_config, registry).await {
            Ok(()) => {
                tracing::info!(server = %server_config.name, "MCP server connected");
            }
            Err(e) => {
                tracing::warn!(server = %server_config.name, error = %e, "MCP server failed to connect");
            }
        }
    }
}
