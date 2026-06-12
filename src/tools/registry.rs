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

    /// 为动态工具注册聚类（MCP 等）
    ///
    /// 每次 MCP server 连接后调用，自动为该 server 的工具创建一个聚类。
    /// 聚类名建议使用 `mcp_{server_name}` 格式，保证唯一性。
    pub fn register_dynamic_cluster(&mut self, cluster_name: &str, tool_names: Vec<String>, brief: &str) {
        if let Some(ref mut clusters) = self.clusters {
            clusters.register(crate::tools::category::ToolCluster {
                meta: crate::tools::category::ClusterMeta {
                    name: cluster_name.to_string(),
                    display_name: format!("MCP: {}", cluster_name),
                    brief: brief.to_string(),
                    tool_names,
                },
                sys_prompt_fragment: format!(
                    "[{}] Dynamic MCP tools from server '{}'.\n",
                    cluster_name, cluster_name
                ),
            });
        }
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
                    // 元工具（load_tool / list_categories）不受聚类限制；
                    // MCP 动态工具已通过 register_dynamic_cluster 注册聚类，不再走此分支。
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
    ///   3. 元工具不受聚类限制；MCP 工具通过动态聚类受控
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

        // ── 前置拦截器：全局 DryRun 模式覆盖 ──
        if ctx.execution_mode == crate::tools::ExecutionMode::DryRun {
            match name {
                "edit" => {
                    // EditTool 原生支持 dry_run 参数
                    input["dry_run"] = Value::Bool(true);
                }
                "write" => {
                    // WriteTool 没有 dry_run，registry 层直接拦截
                    let path = input["path"].as_str().unwrap_or("?");
                    let content = input["content"].as_str().unwrap_or("");
                    let preview: String = content.chars().take(200).collect();
                    let truncated = content.len() > 200;
                    return Ok(ToolResult {
                        content: format!(
                            "[DRY RUN] Would write {} bytes to {}\n\nPreview:\n{}{}",
                            content.len(), path,
                            preview,
                            if truncated { "..." } else { "" }
                        ),
                        is_error: false,
                        metadata: Some(serde_json::json!({
                            "dry_run": true,
                            "path": path,
                            "bytes": content.len(),
                        })),
                    });
                }
                _ => {}
            }
        }

        // ── 前置拦截器：文件修改前读取快照 ──
        let snapshot_state = self.pre_execute_snapshot(name, &input).await;

        // ── 执行原始的工具 ──
        let result = tool.execute(input, ctx).await?;

        // ── 后置拦截器：文件修改后保存快照 ──
        if !result.is_error {
            self.post_execute_snapshot(name, ctx, &snapshot_state).await;
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

    // ── 快照切面（从 execute() 解耦） ──

    /// 前置：文件修改前读取内容备份
    async fn pre_execute_snapshot(&self, name: &str, input: &Value) -> Option<SnapshotState> {
        if name != "write" && name != "edit" {
            return None;
        }
        let path = input["path"].as_str()?.to_string();
        let content_before = tokio::fs::read_to_string(&path).await.ok();
        Some(SnapshotState { path, content_before })
    }

    /// 后置：文件修改后保存快照（SQLite + 行级）
    async fn post_execute_snapshot(&self, name: &str, ctx: &ToolContext, state: &Option<SnapshotState>) {
        let Some(state) = state else { return };
        if name != "write" && name != "edit" { return; }

        // SQLite 快照
        save_file_snapshot(ctx, name, &state.path, state.content_before.as_deref(), self.store.as_deref()).await;

        // 行级快照
        if let Ok(content_after) = tokio::fs::read_to_string(&state.path).await {
            let workspace = std::env::current_dir().unwrap_or_default();
            let mut snapshot_store = crate::tools::code_intelligence::file_snapshot::SnapshotStore::new(&workspace);
            let _ = snapshot_store.init();
            let _ = snapshot_store.load_from_disk();
            let old_content = state.content_before.as_deref().unwrap_or("");
            let snapshot_result = snapshot_store.create_snapshot(
                &state.path,
                old_content,
                &content_after,
                &ctx.session_id,
                &ctx.agent_id,
                Some(format!("{} tool operation", name)),
            );
            match snapshot_result {
                crate::tools::code_intelligence::file_snapshot::SnapshotResult::RiskyChange(entry) => {
                    tracing::warn!(
                        file = %state.path,
                        ratio = %format!("{:.0}%", entry.change_ratio * 100.0),
                        "Risky file change detected (>80% lines changed)"
                    );
                }
                crate::tools::code_intelligence::file_snapshot::SnapshotResult::Created(_) => {
                    tracing::debug!(file = %state.path, "File snapshot created");
                }
                crate::tools::code_intelligence::file_snapshot::SnapshotResult::SkippedUnchanged => {}
            }
        }
    }
}

/// 快照状态（前置读取 → 后置比较）
struct SnapshotState {
    path: String,
    content_before: Option<String>,
}

// ============================================================
//  切面拦截: 路径规范化与安全校验
// ============================================================

/// 路径规范化与安全校验
///
/// 1. 如果路径存在 → canonicalize 直接解析（处理 symlink、.. 等）
/// 2. 如果路径不存在 → 手动规范化，保留 Prefix/RootDir（防止 Windows 驱动号丢失）
/// 3. 绝对路径必须在工作区范围内
/// 4. 相对路径不能逃逸（.. 开头）
fn normalize_and_validate_path(path_str: &str, working_dir: &str) -> crate::Result<String> {
    let path = Path::new(path_str);

    // 1. 路径存在 → 用 canonicalize（最可靠）
    if let Ok(canonical) = path.canonicalize() {
        let normalized = canonical.to_string_lossy().replace('\\', "/");
        // canonicalize 后的绝对路径做逃逸检查
        let work_canonical = PathBuf::from(working_dir)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(working_dir));
        let work_str = work_canonical.to_string_lossy().to_lowercase().replace('\\', "/");
        if !normalized.to_lowercase().starts_with(&work_str) {
            return Err(crate::Error::Tool(format!(
                "安全拦截: 路径 '{}' 超出工作区范围 (working_dir: {})",
                path_str, working_dir
            )));
        }
        return Ok(normalized);
    }

    // 2. 路径不存在 → 手动规范化，保留系统组件
    let mut prefix = String::new();
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => {
                // Windows 驱动号前缀 (C:, D: 等) — 必须保留
                prefix = p.as_os_str().to_string_lossy().to_string();
            }
            Component::RootDir => {
                // 根目录分隔符 — 保留
                if !prefix.is_empty() {
                    prefix.push('/');
                } else {
                    prefix = "/".to_string();
                }
            }
            Component::Normal(s) => components.push(s.to_string_lossy().to_string()),
            Component::ParentDir => { components.pop(); }
            Component::CurDir => {}
        }
    }

    let normalized = if prefix.is_empty() {
        components.join("/")
    } else {
        format!("{}{}", prefix, components.join("/"))
    };

    // 3. 绝对路径逃逸检查
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

    // 4. 相对路径不能逃逸
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
