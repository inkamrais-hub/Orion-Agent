//! ToolSpec — 工具规范数据化
//!
//! Codex 设计: 工具分为 4 种 exposure 档位
//!   Direct          → 直接给 LLM 用 (bash, read, write)
//!   Deferred        → LLM 需要时才加载 (代码索引工具)
//!   DirectModelOnly → 只给 LLM 看，用户看不到
//!   Hidden          → 隐藏，内部用 (子 Agent)
//!
//! 好处: LLM 只看到常用的 5-6 个工具，其他的按需搜索

use serde::{Deserialize, Serialize};

/// 工具暴露级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum ToolExposure {
    /// 直接给 LLM (默认)
    Direct,
    /// LLM 需要时才加载 (通过 ToolSearch)
    Deferred,
    /// 只给 LLM 看，用户看不到
    DirectModelOnly,
    /// 隐藏，内部用
    Hidden,
}

/// 工具规范 (纯数据，可序列化)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// 工具名
    pub name: String,
    /// 描述
    pub description: String,
    /// 暴露级别
    pub exposure: ToolExposure,
    /// 输入 JSON Schema
    pub input_schema: serde_json::Value,
    /// 是否支持并行调用
    pub supports_parallel: bool,
    /// 命名空间 (可选，用于分组)
    pub namespace: Option<String>,
    /// 延迟加载提示 (Deferred 时使用)
    pub search_hint: Option<String>,
}

impl ToolSpec {
    /// 创建 Direct 工具
    pub fn direct(name: impl Into<String>, description: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            exposure: ToolExposure::Direct,
            input_schema: schema,
            supports_parallel: false,
            namespace: None,
            search_hint: None,
        }
    }

    /// 创建 Deferred 工具
    pub fn deferred(name: impl Into<String>, description: impl Into<String>, schema: serde_json::Value, hint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            exposure: ToolExposure::Deferred,
            input_schema: schema,
            supports_parallel: false,
            namespace: None,
            search_hint: Some(hint.into()),
        }
    }

    /// 创建 Hidden 工具
    pub fn hidden(name: impl Into<String>, description: impl Into<String>, schema: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            exposure: ToolExposure::Hidden,
            input_schema: schema,
            supports_parallel: false,
            namespace: None,
            search_hint: None,
        }
    }
}

/// 工具注册表 (带 exposure 过滤)
pub struct ToolSpecRegistry {
    specs: Vec<ToolSpec>,
}

impl ToolSpecRegistry {
    pub fn new() -> Self {
        Self { specs: Vec::new() }
    }

    /// 注册工具规范
    pub fn register(&mut self, spec: ToolSpec) {
        self.specs.push(spec);
    }

    /// 获取 Direct + DirectModelOnly 工具 (给 LLM)
    pub fn visible_tools(&self) -> Vec<&ToolSpec> {
        self.specs.iter()
            .filter(|s| s.exposure == ToolExposure::Direct || s.exposure == ToolExposure::DirectModelOnly)
            .collect()
    }

    /// 获取所有工具 (包括 Deferred)
    pub fn all_tools(&self) -> Vec<&ToolSpec> {
        self.specs.iter().collect()
    }

    /// 按名搜索工具 (ToolSearch 功能)
    pub fn search(&self, query: &str) -> Vec<&ToolSpec> {
        let query_lower = query.to_lowercase();
        self.specs.iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&query_lower)
                    || s.description.to_lowercase().contains(&query_lower)
                    || s.search_hint.as_ref().map(|h| h.to_lowercase().contains(&query_lower)).unwrap_or(false)
            })
            .collect()
    }

    /// 获取 Deferred 工具
    pub fn deferred_tools(&self) -> Vec<&ToolSpec> {
        self.specs.iter()
            .filter(|s| s.exposure == ToolExposure::Deferred)
            .collect()
    }

    /// 获取指定工具
    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.specs.iter().find(|s| s.name == name)
    }

    /// 工具总数
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

impl Default for ToolSpecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 注册 Orion 默认工具规范
pub fn register_default_tools(registry: &mut ToolSpecRegistry) {
    // Direct 工具 (LLM 直接使用)
    registry.register(ToolSpec::direct("read", "Read file content", serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "File path"},
            "offset": {"type": "integer", "description": "Start line (optional)"},
            "limit": {"type": "integer", "description": "Max lines (optional)"}
        },
        "required": ["path"]
    })));

    registry.register(ToolSpec::direct("write", "Write content to file", serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "content": {"type": "string"}
        },
        "required": ["path", "content"]
    })));

    registry.register(ToolSpec::direct("edit", "Edit file by string replacement", serde_json::json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "old_string": {"type": "string"},
            "new_string": {"type": "string"},
            "replace_all": {"type": "boolean"}
        },
        "required": ["path", "old_string", "new_string"]
    })));

    registry.register(ToolSpec::direct("bash", "Execute shell command", serde_json::json!({
        "type": "object",
        "properties": {
            "command": {"type": "string"},
            "timeout_secs": {"type": "integer"}
        },
        "required": ["command"]
    })));

    registry.register(ToolSpec::direct("grep", "Search text in files", serde_json::json!({
        "type": "object",
        "properties": {
            "pattern": {"type": "string"},
            "path": {"type": "string"},
            "glob": {"type": "string"}
        },
        "required": ["pattern"]
    })));

    registry.register(ToolSpec::direct("glob", "Find files by pattern", serde_json::json!({
        "type": "object",
        "properties": {
            "pattern": {"type": "string"}
        },
        "required": ["pattern"]
    })));

    // Deferred 工具 (按需加载)
    registry.register(ToolSpec::deferred("project_map", "Get project structure overview", serde_json::json!({}), "项目结构 文件树"));

    registry.register(ToolSpec::deferred("symbol_search", "Search code symbols", serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"}
        },
        "required": ["query"]
    }), "符号 函数 类 变量"));

    registry.register(ToolSpec::deferred("find_callers", "Find function callers", serde_json::json!({
        "type": "object",
        "properties": {
            "function_name": {"type": "string"}
        },
        "required": ["function_name"]
    }), "调用者 引用 函数"));

    registry.register(ToolSpec::deferred("snapshot_history", "View file snapshot history", serde_json::json!({
        "type": "object",
        "properties": {
            "file_path": {"type": "string"}
        },
        "required": ["file_path"]
    }), "快照 历史 版本"));

    registry.register(ToolSpec::deferred("snapshot_rollback", "Rollback file to snapshot", serde_json::json!({
        "type": "object",
        "properties": {
            "file_path": {"type": "string"},
            "snapshot_index": {"type": "integer"},
            "current_content": {"type": "string"}
        },
        "required": ["file_path", "snapshot_index", "current_content"]
    }), "回溯 恢复 快照"));

    registry.register(ToolSpec::deferred("snapshot_risky", "View risky snapshots (>80% change)", serde_json::json!({
        "type": "object",
        "properties": {}
    }), "风险 快照 误操作"));

    registry.register(ToolSpec::deferred("web_search", "Search the web for information", serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "Search query"},
            "engine": {"type": "string", "description": "Search engine: bing (default), duckduckgo"},
            "max_results": {"type": "integer", "description": "Maximum results (default: 5)"}
        },
        "required": ["query"]
    }), "搜索 联网 查找"));

    registry.register(ToolSpec::deferred("terminal", "Execute commands in various terminals", serde_json::json!({
        "type": "object",
        "properties": {
            "command": {"type": "string", "description": "Command to execute"},
            "shell": {"type": "string", "description": "Terminal: powershell, cmd, bash, sh, wsl"},
            "input": {"type": "string", "description": "Pre-provide input for interactive commands"},
            "timeout": {"type": "integer", "description": "Timeout in seconds (default: 120)"}
        },
        "required": ["command"]
    }), "终端 命令 shell"));

    // Hidden 工具 (内部使用)
    registry.register(ToolSpec::hidden("ask_user", "Ask user a question", serde_json::json!({
        "type": "object",
        "properties": {
            "question": {"type": "string"}
        },
        "required": ["question"]
    })));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exposure_filter() {
        let mut registry = ToolSpecRegistry::new();
        register_default_tools(&mut registry);

        let visible = registry.visible_tools();
        assert!(visible.iter().all(|s| s.exposure == ToolExposure::Direct || s.exposure == ToolExposure::DirectModelOnly));

        let deferred = registry.deferred_tools();
        assert!(deferred.iter().all(|s| s.exposure == ToolExposure::Deferred));
    }

    #[test]
    fn test_search() {
        let mut registry = ToolSpecRegistry::new();
        register_default_tools(&mut registry);

        let results = registry.search("文件");
        assert!(!results.is_empty());
    }
}
