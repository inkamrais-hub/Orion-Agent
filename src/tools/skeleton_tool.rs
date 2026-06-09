use async_trait::async_trait;
use serde_json::Value;

use super::{Tool, ToolContext, ToolResult};

/// 代码骨架提取工具
///
/// 从源码文件中提取函数/结构体/枚举等定义的签名，折叠函数体。
/// 输出精简的骨架视图，Token 消耗仅为原始文件的 10-20%。
pub struct SkeletonTool;

#[async_trait]
impl Tool for SkeletonTool {
    fn name(&self) -> &str {
        "skeleton"
    }

    fn description(&self) -> &str {
        "Extract code skeleton from a source file. Shows only function signatures, struct/enum definitions, \
         impl blocks, and trait definitions with line numbers. Function bodies are collapsed to '{ ... }'. \
         Token usage is only 10-20% of the full file. \
         Supports Rust, Python, JS/TS, Go, and generic languages. \
         Use this to quickly understand a large file's structure without reading the entire content."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the source file"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let path_str = input["path"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'path' field".into())
        })?;

        let path = std::path::Path::new(path_str);
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| crate::Error::Tool(format!("Cannot read '{}': {}", path_str, e)))?;

        let entries = crate::index::skeleton::extract_skeleton(&content, path);
        let skeleton = crate::index::skeleton::format_skeleton(&entries, path);
        let entry_count = entries.len();
        let original_lines = content.lines().count();

        Ok(ToolResult {
            content: skeleton,
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": path_str,
                "definitions": entry_count,
                "original_lines": original_lines,
            })),
        })
    }
}
