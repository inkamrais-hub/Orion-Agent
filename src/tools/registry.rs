use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use crate::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;

// ============================================================
//  积木: ToolRegistry (工具注册表)
//  职责: 管理所有已注册的工具
//  设计: 用 Arc<dyn Tool> 存储, 支持按名查找和全部枚举
// ============================================================

type ToolBox = Box<dyn Tool>;

/// 工具注册表
pub struct ToolRegistry {
    tools: HashMap<String, ToolBox>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    /// 注册一个工具
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Box::new(tool));
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
    pub fn definitions(&self) -> Vec<Value> {
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

        // ── 后置拦截器：回滚快照 ──
        // TODO: 对 write/edit 工具，执行前备份文件内容，执行成功后保存到 SQLite
        // 设计要点:
        //   1. 判断 name 是否为 "write" 或 "edit"
        //   2. 如果是，读取 input["path"] 的当前文件内容作为快照
        //   3. 工具执行成功后，将快照存入 SQLite（用于撤销/审计）
        //   4. 快照存储方案待定，暂不实现

        // ── 执行原始工具 ──
        tool.execute(input, ctx).await
    }

    pub fn len(&self) -> usize {
        self.tools.len()
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

    // 1. 规范化路径组件（移除 . 和 ..）
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => components.push(s.to_string_lossy().to_string()),
            Component::ParentDir => { components.pop(); }
            Component::CurDir => {} // 跳过 .
            _ => {} // 跳过 RootDir, Prefix 等系统组件
        }
    }

    // 2. 统一路径分隔符为 /
    let normalized = components.join("/");

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

    // 4. 检查规范化后的相对路径是否存在越权逃逸
    //    例如: "../../etc/passwd" 规范化后变成空或 "../.." 开头
    //    对于相对路径，需要确保规范化后不会逃逸出工作区
    if !path.is_absolute() && normalized.starts_with("..") {
        return Err(crate::Error::Tool(format!(
            "安全拦截: 路径 '{}' 规范化后逃逸出工作区范围",
            path_str
        )));
    }

    Ok(normalized)
}
