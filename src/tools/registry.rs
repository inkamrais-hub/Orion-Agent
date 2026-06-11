use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use crate::tools::{Tool, ToolContext, ToolResult};
use crate::tools::category::ClusterRegistry;
use serde_json::Value;

// ============================================================
//  积木: ToolRegistry (工具注册表)
//  职责: 管理所有已注册的工具 + 工具聚类 + 权限过滤
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
    /// 统一存储（用于文件快照持久化）
    store: Option<Arc<crate::session::UnifiedStore>>,
    /// 工具聚类注册表（可选）
    clusters: Option<ClusterRegistry>,
    /// 允许的聚类集合（None = 全部允许，Some = 仅限指定聚类）
    allowed_clusters: Option<HashSet<String>>,
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
            clusters: self.clusters.as_ref().map(|_| {
                // ClusterRegistry 不实现 Clone，重新构建
                // 实际上 clone registry 时通常会重新设置 clusters
                crate::tools::category::create_default_clusters()
            }),
            allowed_clusters: self.allowed_clusters.clone(),
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
            clusters: None,
            allowed_clusters: None,
        }
    }

    /// 启用延迟装载模式
    pub fn enable_lazy_mode(&mut self) {
        self.lazy_mode = true;
    }

    /// 注入统一存储（用于文件快照持久化）
    pub fn set_store(&mut self, store: Arc<crate::session::UnifiedStore>) {
        self.store = Some(store);
    }

    /// 注入工具聚类注册表
    pub fn set_clusters(&mut self, clusters: ClusterRegistry) {
        self.clusters = Some(clusters);
    }

    /// 限制允许的聚类（sub-agent 用）
    ///
    /// 设置后，definitions() 只返回属于允许聚类的工具，
    /// execute() 会拒绝不在允许范围内的工具调用。
    pub fn set_allowed_clusters(&mut self, clusters: HashSet<String>) {
        self.allowed_clusters = Some(clusters);
    }

    /// 是否处于延迟装载模式
    pub fn is_lazy_mode(&self) -> bool {
        self.lazy_mode
    }

    /// 激活指定工具（线程安全，内部使用 Mutex）
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

    /// 检查工具是否在当前权限范围内
    fn is_tool_allowed(&self, name: &str) -> bool {
        match (&self.allowed_clusters, &self.clusters) {
            (Some(allowed), Some(clusters)) => {
                match clusters.cluster_of(name) {
                    Some(cluster_name) => allowed.contains(cluster_name),
                    // MCP 动态工具和元工具不受聚类限制
                    None => true,
                }
            }
            _ => true,
        }
    }

    /// 获取所有工具定义 (用于 LLM function calling schema)
    ///
    /// 过滤逻辑:
    ///   1. 延迟模式 → 只返回已激活的工具
    ///   2. 聚类限制 → 只返回属于允许聚类的工具
    ///   3. 元工具和 MCP 工具不受聚类限制
    pub fn definitions(&self) -> Vec<Value> {
        self.tools
            .values()
            .filter(|tool| {
                // 延迟模式过滤
                if self.lazy_mode {
                    let activated = self.activated.lock().unwrap_or_else(|p| p.into_inner());
                    if !activated.contains(tool.name()) {
                        return false;
                    }
                }
                // 聚类权限过滤
                self.is_tool_allowed(tool.name())
            })
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema()
                })
            })
            .collect()
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

    /// 获取聚类系统提示（全部或按允许的聚类过滤）
    pub fn cluster_sys_prompt(&self) -> String {
        match (&self.clusters, &self.allowed_clusters) {
            (Some(clusters), Some(allowed)) => {
                let allowed_vec: Vec<String> = allowed.iter().cloned().collect();
                clusters.sys_prompt_for_clusters(&allowed_vec)
            }
            (Some(clusters), None) => clusters.sys_prompt_all(),
            _ => String::new(),
        }
    }

    /// 执行工具（含切面拦截 + 聚类权限检查）
    pub async fn execute(
        &self,
        name: &str,
        mut input: Value,
        ctx: &ToolContext,
    ) -> crate::Result<ToolResult> {
        // ── 聚类权限检查 ──
        if !self.is_tool_allowed(name) {
            let cluster_name = self.clusters.as_ref()
                .and_then(|c| c.cluster_of(name))
                .unwrap_or("unknown");
            return Ok(ToolResult {
                content: format!(
                    "Tool '{}' (cluster: {}) is not available in this agent's scope. \
                     Connect the '{}' cluster to enable this tool.",
                    name, cluster_name, cluster_name
                ),
                is_error: true,
                metadata: Some(serde_json::json!({
                    "denied": true,
                    "cluster": cluster_name,
                })),
            });
        }

        let tool = self.tools.get(name)
            .ok_or_else(|| crate::Error::Tool(format!("Unknown tool: {}", name)))?;

        // ── 前置拦截器：路径规范化与安全校验 ──
        if let Some(path_val) = input.get("path") {
            if let Some(path_str) = path_val.as_str() {
                let normalized = normalize_and_validate_path(path_str, &ctx.working_dir)?;
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

        // ── 执行原始的工具 ──
        let result = tool.execute(input, ctx).await?;

        // ── 后置拦截器：保存快照到 SQLite ──
        if (name == "write" || name == "edit") && !result.is_error {
            if let Some(ref path_str) = snapshot_path {
                save_file_snapshot(ctx, name, path_str, content_before.as_deref(), self.store.as_deref()).await;

                // ── 行级快照: 调用 SnapshotStore.create_snapshot() ──
                if let Ok(content_after) = tokio::fs::read_to_string(path_str).await {
                    let workspace = std::env::current_dir().unwrap_or_default();
                    let mut snapshot_store = crate::tools::code_intelligence::file_snapshot::SnapshotStore::new(&workspace);
                    let _ = snapshot_store.init();
                    let _ = snapshot_store.load_from_disk();
                    let old_content = content_before.as_deref().unwrap_or("");
                    let snapshot_result = snapshot_store.create_snapshot(
                        path_str,
                        old_content,
                        &content_after,
                        &ctx.session_id,
                        &ctx.agent_id,
                        Some(format!("{} tool operation", name)),
                    );
                    match snapshot_result {
                        crate::tools::code_intelligence::file_snapshot::SnapshotResult::RiskyChange(entry) => {
                            tracing::warn!(
                                file = %path_str,
                                ratio = %format!("{:.0}%", entry.change_ratio * 100.0),
                                "Risky file change detected (>80% lines changed)"
                            );
                        }
                        crate::tools::code_intelligence::file_snapshot::SnapshotResult::Created(_) => {
                            tracing::debug!(file = %path_str, "File snapshot created");
                        }
                        crate::tools::code_intelligence::file_snapshot::SnapshotResult::SkippedUnchanged => {}
                    }
                }
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

    /// 创建受限副本 — 用于 sub-agent，只允许指定聚类的工具
    pub fn with_clusters(&self, allowed: Vec<String>) -> Self {
        let mut cloned = self.clone();
        cloned.set_allowed_clusters(allowed.into_iter().collect());
        cloned
    }
}

// ============================================================
//  切面拦截: 路径规范化与安全校验
// ============================================================

/// 路径规范化与安全校验
fn normalize_and_validate_path(path_str: &str, working_dir: &str) -> crate::Result<String> {
    let path = Path::new(path_str);

    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => components.push(s.to_string_lossy().to_string()),
            Component::ParentDir => { components.pop(); }
            Component::CurDir => {}
            _ => {}
        }
    }

    let normalized = components.join("/");

    if path.is_absolute() {
        let work = PathBuf::from(working_dir);
        let work_str = work.to_string_lossy().to_lowercase().replace('\\', "/");
        let norm_lower = normalized.to_lowercase();
        if !norm_lower.starts_with(&work_str) {
            return Err(crate::Error::Tool(format!(
                "安全拦截: 路径 '{}' 超出工作区范围 (working_dir: {})",
                path_str, working_dir
            )));
        }
    }

    if !path.is_absolute() && normalized.starts_with("..") {
        return Err(crate::Error::Tool(format!(
            "安全拦截: 路径 '{}' 规范化后逃逸出工作区范围",
            path_str
        )));
    }

    Ok(normalized)
}

// ============================================================
//  快照存储: 文件修改前自动备份到 SQLite
// ============================================================

async fn save_file_snapshot(
    ctx: &ToolContext,
    tool_name: &str,
    target_path: &str,
    content_before: Option<&str>,
    store: Option<&crate::session::UnifiedStore>,
) {
    if ctx.session_id.is_empty() {
        return;
    }

    if let Some(store) = store {
        let snapshot = crate::session::unified::SessionSnapshot {
            snapshot_id: uuid::Uuid::new_v4().to_string(),
            session_id: ctx.session_id.clone(),
            agent_id: ctx.agent_id.clone(),
            turn_index: ctx.turn_number as i64,
            tool_name: tool_name.to_string(),
            target_path: target_path.to_string(),
            content_before: content_before.map(|s| s.to_string()),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        if let Err(e) = store.save_snapshot(&snapshot).await {
            tracing::warn!(error = %e, "Failed to save file snapshot");
        }
    }
}
