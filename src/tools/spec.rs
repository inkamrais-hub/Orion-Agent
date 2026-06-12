//! ToolSpec — 工具规范数据（从 Tool trait 派生）
//!
//! ToolSpec 是 Tool trait 的数据投影，不包含任何硬编码 schema。
//! 唯一真相源是 Tool trait 本身。ToolSpec 仅用于序列化/API 响应等场景。

use serde::{Deserialize, Serialize};
use super::ToolExposure;

/// 工具规范 (纯数据，可序列化)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub exposure: ToolExposure,
    pub input_schema: serde_json::Value,
    pub supports_parallel: bool,
    pub search_hint: Option<String>,
}
