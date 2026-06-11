use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use crate::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;

// ============================================================
//  积木: ToolRegistry (工具注册表)
//  职责: 管理所有已注册的工具
//  设计: 用 Arc<dyn Tool> 存储, 支持 Clone 共享给子 Agent
// ============================================================

type ToolBox = Arc<dyn Tool>;

/// 工具注册表（可 Clone，子 Agent 可继承全部工具）
pub struct ToolRegistry {
    tools: HashMap<String, ToolBox>,
    /// 已激活的工具名（延迟模式下使用，内部可变性）
    activated: Arc<std::sync::Mutex<HashSet<String>>>,
    /// 是否启用延迟装载模式
    lazy_mode: bool,
    /// 统一存储（用于快照等持久化操作）
    store: Option<Arc<crate::session::UnifiedStore>>,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            activated: Arc::new(std::sync::Mutex::new(
                self.activated.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            )),
            lazy_mode: self.lazy_mode,
            store: self.store.clone(),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            activated: Arc::new(std::sync::Mutex::new(HashSet::new())),
            lazy_mode: false,
            store: None,
        }
    }

    /// 设置统一存储（用于快照等持久化操作）
    pub fn set_store(&mut self, store: Arc<crate::session::UnifiedStore>) {
        self.store = Some(store);
    }

    /// 启用延迟装载模式
    pub fn enable_lazy_mode(&mut self) {
        self.lazy_mode = true;
    }

    /// 是否处于延迟装载模式
    pub fn is_lazy_mode(&self) -> bool {
        self.lazy_mode
    }

    /// 激活指定工具（线程安全，内部使用 Mutex）
    ///
    /// 如果工具存在，将其加入激活集合，返回 true；
    /// 如果工具不存在，返回 false。
    pub fn activate(&self, name: &str) -> bool {
        if self.tools.contains_key(name) {
            let mut activated = self.activated.lock().unwrap_or_else(|p| p.into_inner());
            activated.insert(name.to_string());
            true
        } else {
            false
        }
    }

    /// 获取所有工具名列表
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// 注册一个工具
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    /// 批量注册
    pub fn register_all(&mut self, tools: Vec<ToolBox>) {
        for tool in tools {
            let name = tool.name().to_string();
            self.tools.insert(name, tool);
        }
    }

    /// 按名称查找
    pub fn get(&self, name: &str) -> Option<&ToolBox> {
        self.tools.get(name)
    }

    /// 获取所有工具定义 (用于 LLM function calling schema)
    ///
    /// 延迟模式下只返回已激活的工具；非延迟模式返回全部。
    pub fn definitions(&self) -> Vec<Value> {
        if self.lazy_mode {
            let activated = self.activated.lock().unwrap_or_else(|p| p.into_inner());
            self.tools
                .values()
                .filter(|t| activated.contains(t.name()))
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name(),
                        "description": tool.description(),
                        "input_schema": tool.input_schema()
                    })
                })
                .collect()
        } else {
            self.tools
                .values()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name(),
                        "description": tool.description(),
                        "input_schema": tool.input_schema()
                    })
                })
                .collect()
        }
    }

    /// 获取所有工具的简要列表（名称+描述，不含 schema）
    pub fn brief_list(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                })
            })
            .collect()
    }

    /// 获取指定工具的完整 schema（用于 load_tool 元工具）
    pub fn tool_schema(&self, name: &str) -> Option<Value> {
        self.tools.get(name).map(|tool| {
            serde_json::json!({
                "name": tool.name(),
                "description": tool.description(),
                "input_schema": tool.input_schema()
            })
        })
    }

    /// 执行工具（含切面拦截）
    pub async fn execute(
        &self,
        name: &str,
        mut input: Value,
        ctx: &ToolContext,
    ) -> crate::Result<ToolResult> {
        let tool = self.tools.get(name)
            .ok_or_else(|| crate::Error::Tool(format!("Unknown tool: {}", name)))?;

        // ── 前置拦截器：路径规范化与安全校验 ──
        if let Some(path_val) = input.get("path") {
            if let Some(path_str) = path_val.as_str() {
                let normalized = normalize_and_validate_path(path_str, &ctx.working_dir)?;
                // 将规范化后的路径写回 input
                input["path"] = Value::String(normalized);
            }
        }

        // ── 前置拦截器：文件快照备份（write/edit 工具） ──
        let snapshot_path = if name == "write" || name == "edit" {
            input["path"].as_str().map(|s| s.to_string())
        } else {
            None
        };
        let content_before = if let Some(ref path_str) = snapshot_path {
            tokio::fs::read_to_string(path_str).await.ok()
        } else {
            None
        };

        // ── 执行原始工具 ──
        let result = tool.execute(input, ctx).await?;

        // ── 后置拦截器：保存快照到 SQLite ──
        if (name == "write" || name == "edit") && !result.is_error {
            if let Some(ref path_str) = snapshot_path {
                save_file_snapshot(ctx, name, path_str, content_before, self.store.as_deref()).await;
            }
        }

        Ok(result)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

// ============================================================
//  切面拦截: 路径规范化与安全校验
// ============================================================

/// 路径规范化与安全校验
///
/// 1. 移除 `.` 和 `..` 组件
/// 2. 统一路径分隔符为 `/`
/// 3. 绝对路径校验: 不能逃逸出工作区目录
fn normalize_and_validate_path(path_str: &str, working_dir: &str) -> crate::Result<String> {
    let path = Path::new(path_str);

    // 1. 规范化路径组件（移除 . 和 ..，保留 Prefix 和 RootDir）
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(_) | Component::Prefix(_) | Component::RootDir => {
                normalized.push(comp.as_os_str());
            }
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(crate::Error::Tool(format!(
                        "Path traversal detected: path '{}' escapes working directory",
                        path_str
                    )));
                }
            }
            Component::CurDir => {} // 跳过 .
        }
    }

    // 2. 检查路径中是否含有 .. 组件 (路径穿越防护)
    if normalized.components().any(|c| c.as_os_str() == "..") {
        return Err(crate::Error::Tool("Path traversal detected: path escapes working directory".to_string()));
    }

    // 3. 统一路径分隔符为 /
    let normalized_str = path_to_forward_slash(&normalized);

    // 4. 绝对路径逃逸检查
    if path.is_absolute() {
        let work = PathBuf::from(working_dir);
        let work_str = path_to_forward_slash(&work).to_lowercase();
        let norm_lower = normalized_str.to_lowercase();
        if !norm_lower.starts_with(&work_str) {
            return Err(crate::Error::Tool(format!(
                "安全拦截: 路径 '{}' 超出工作区范围 (working_dir: {})",
                path_str, working_dir
            )));
        }
        // 绝对路径且通过验证，尝试返回 canonicalized 路径
        if let Ok(canonical) = normalized.canonicalize() {
            return Ok(path_to_forward_slash(&canonical));
        }
        return Ok(normalized_str);
    }

    // 5. 相对路径不再以 .. 开头即可
    //    (ParentDir pop 失败已在循环中拦截，此处做额外兜底)
    if normalized.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(crate::Error::Tool(format!(
            "安全拦截: 路径 '{}' 规范化后逃逸出工作区范围",
            path_str
        )));
    }

    // 相对路径尝试 canonicalize
    if let Ok(canonical) = normalized.canonicalize() {
        return Ok(path_to_forward_slash(&canonical));
    }

    Ok(normalized_str)
}

/// 将 PathBuf 转换为正斜杠分隔的字符串
fn path_to_forward_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// ============================================================
//  快照存储: 文件修改前自动备份到 SQLite
// ============================================================

/// 将文件快照保存到 SQLite（异步 fire-and-forget）
async fn save_file_snapshot(
    ctx: &ToolContext,
    tool_name: &str,
    target_path: &str,
    content_before: Option<String>,
    store: Option<&crate::session::UnifiedStore>,
) {
    // 跳过空 session（非 API 调用场景可能没有 session_id）
    if ctx.session_id.is_empty() {
        return;
    }

    let store = match store {
        Some(s) => s,
        None => {
            tracing::warn!("No UnifiedStore available for snapshot, skipping");
            return;
        }
    };

    let snapshot = crate::agent::store::SessionSnapshot {
        snapshot_id: uuid::Uuid::new_v4().to_string(),
        session_id: ctx.session_id.clone(),
        agent_id: ctx.agent_id.clone(),
        turn_index: ctx.turn_number as i64,
        tool_name: tool_name.to_string(),
        target_path: target_path.to_string(),
        content_before,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    // 异步写入失败只 warn 不阻断主流程
    if let Err(e) = store.save_session_snapshot(&snapshot).await {
        tracing::warn!(error = %e, "Failed to save file snapshot");
    }
}

// ============================================================
//  Tests: 路径规范化与安全校验
// ============================================================

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn test_absolute_path_preserved() {
        // Absolute paths within workspace should pass validation
        let result = normalize_and_validate_path("/home/user/project/src/main.rs", "/home/user/project");
        assert!(result.is_ok());
        let normalized = result.unwrap();
        assert!(normalized.starts_with("/home/user/project"));
    }

    #[test]
    fn test_path_traversal_blocked() {
        // foo/../../etc/passwd should be blocked (escapes via ..)
        let result = normalize_and_validate_path("foo/../../etc/passwd", "/home/user/project");
        assert!(result.is_err());
    }

    #[test]
    fn test_relative_path_within_workspace() {
        let result = normalize_and_validate_path("src/main.rs", "/home/user/project");
        assert!(result.is_ok());
    }

    #[test]
    fn test_dotdot_escaping_blocked() {
        let result = normalize_and_validate_path("../../etc/passwd", "/home/user/project");
        assert!(result.is_err());
    }

    #[test]
    fn test_single_dotdot_escaping_blocked() {
        // A single .. that escapes should also be blocked
        let result = normalize_and_validate_path("../../../etc/passwd", "/home/user/project");
        assert!(result.is_err());
    }

    #[test]
    fn test_dot_components_stripped() {
        // Current-dir components (.) should be stripped
        let result = normalize_and_validate_path("./src/./main.rs", "/home/user/project");
        assert!(result.is_ok());
        let normalized = result.unwrap();
        assert!(!normalized.contains("./"));
    }

    #[test]
    fn test_absolute_path_outside_workspace_blocked() {
        // Absolute path outside working directory should be blocked
        #[cfg(unix)]
        {
            let result = normalize_and_validate_path("/etc/passwd", "/home/user/project");
            assert!(result.is_err());
        }
        #[cfg(windows)]
        {
            let result = normalize_and_validate_path(
                "C:\\Windows\\System32\\config",
                "C:\\Users\\test\\project",
            );
            assert!(result.is_err());
        }
    }
}
