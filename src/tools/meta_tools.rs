//! 元工具 — 延迟装载模式下的工具发现与加载
//!
//! 在延迟模式下，LLM 初始只知道两个元工具：
//! - `list_categories` — 列出所有可用工具（名称+描述，不含 schema）
//! - `load_tool` — 按名称加载工具的完整 Schema，使其在后续 Turn 中可用
//!
//! 使用 `Weak<ToolRegistry>` 打破 ToolRegistry ↔ 元工具 的循环引用。

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Weak;

use super::{Tool, ToolContext, ToolResult};
use super::registry::ToolRegistry;

/// 元工具：按需加载工具 Schema
pub struct LoadToolTool {
    registry: Weak<ToolRegistry>,
}

impl LoadToolTool {
    pub fn new(registry: Weak<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for LoadToolTool {
    fn name(&self) -> &str { "load_tool" }

    fn description(&self) -> &str {
        "Load a tool's full schema by name. After loading, the tool becomes available \
         for use in subsequent turns. Use list_categories first to discover available tools."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Name of the tool to load"
                }
            },
            "required": ["tool_name"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let name = input["tool_name"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'tool_name'".into())
        })?;

        let registry = self.registry.upgrade().ok_or_else(|| {
            crate::Error::Tool("ToolRegistry has been dropped".into())
        })?;

        if registry.activate(name) {
            let schema = registry.tool_schema(name).unwrap();
            Ok(ToolResult {
                content: format!(
                    "Tool '{}' loaded successfully. Schema:\n{}",
                    name,
                    serde_json::to_string_pretty(&schema).unwrap_or_default()
                ),
                is_error: false,
                metadata: None,
            })
        } else {
            let available = registry.tool_names().join(", ");
            Ok(ToolResult {
                content: format!(
                    "Tool '{}' not found. Available tools: {}",
                    name, available
                ),
                is_error: true,
                metadata: None,
            })
        }
    }
}

/// 元工具：列出所有可用工具
pub struct ListCategoriesTool {
    registry: Weak<ToolRegistry>,
}

impl ListCategoriesTool {
    pub fn new(registry: Weak<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListCategoriesTool {
    fn name(&self) -> &str { "list_categories" }

    fn description(&self) -> &str {
        "List all available tool categories and tool names. \
         Use this to discover tools before loading them with load_tool."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let registry = self.registry.upgrade().ok_or_else(|| {
            crate::Error::Tool("ToolRegistry has been dropped".into())
        })?;

        let brief = registry.brief_list();
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&brief).unwrap_or_default(),
            is_error: false,
            metadata: None,
        })
    }
}
