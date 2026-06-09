//! 分层工具架构 — 工具类别抽象层
//!
//! 设计理念:
//!   Agent → ToolCategory (抽象层) → Tool (具体类)
//!
//! 优势:
//!   1. System Prompt 只声明类别，不展开具体工具
//!   2. Agent 按需调用类别，获取具体工具详情
//!   3. 新增工具只需注册到对应类别
//!   4. 积木化组合，易于维护和扩展

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use super::Tool;

/// 工具类别元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMeta {
    /// 类别名称 (如 "file_ops")
    pub name: String,
    /// 类别显示名 (如 "文件操作")
    pub display_name: String,
    /// 类别简介 (用于 system prompt，尽量简短)
    pub brief: String,
    /// 类别详细描述 (用于展开时)
    pub description: String,
    /// 包含的工具名称列表
    pub tools: Vec<String>,
}

/// 工具类别 trait
///
/// 每个类别是一组相关工具的集合。
/// Agent 先看到类别简介，按需展开获取具体工具。
#[async_trait]
pub trait ToolCategory: Send + Sync {
    /// 类别元数据
    fn meta(&self) -> CategoryMeta;

    /// 获取类别下的所有工具
    fn tools(&self) -> Vec<Arc<dyn Tool>>;

    /// 获取类别详细描述 (包含每个工具的用法)
    fn detailed_description(&self) -> String {
        let meta = self.meta();
        let mut desc = format!("# {} — {}\n\n{}\n\n", meta.display_name, meta.name, meta.description);
        desc.push_str("## 可用工具\n\n");

        for tool in self.tools() {
            desc.push_str(&format!("### `{}`\n", tool.name()));
            desc.push_str(&format!("{}\n\n", tool.description()));
            desc.push_str(&format!("参数: {}\n\n", serde_json::to_string_pretty(&tool.input_schema()).unwrap_or_default()));
        }

        desc
    }

    /// 按名称查找工具
    fn find_tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools().into_iter().find(|t| t.name() == name)
    }
}

/// 工具类别注册表
pub struct CategoryRegistry {
    categories: HashMap<String, Arc<dyn ToolCategory>>,
}

impl CategoryRegistry {
    pub fn new() -> Self {
        Self {
            categories: HashMap::new(),
        }
    }

    /// 注册工具类别
    pub fn register(&mut self, category: Arc<dyn ToolCategory>) {
        let name = category.meta().name.clone();
        self.categories.insert(name, category);
    }

    /// 获取类别
    pub fn get(&self, name: &str) -> Option<&Arc<dyn ToolCategory>> {
        self.categories.get(name)
    }

    /// 获取所有类别名称
    pub fn category_names(&self) -> Vec<String> {
        self.categories.keys().cloned().collect()
    }

    /// 获取所有类别元数据
    pub fn all_meta(&self) -> Vec<CategoryMeta> {
        self.categories.values().map(|c| c.meta()).collect()
    }

    /// 生成简短的类别列表 (用于 system prompt)
    pub fn brief_list(&self) -> String {
        let mut list = String::new();
        for meta in self.all_meta() {
            list.push_str(&format!("- {}: {}\n", meta.display_name, meta.brief));
        }
        list
    }

    /// 展开指定类别的详细描述
    pub fn expand_category(&self, name: &str) -> Option<String> {
        self.categories.get(name).map(|c| c.detailed_description())
    }

    /// 按工具名查找所属类别和工具
    pub fn find_tool(&self, tool_name: &str) -> Option<(String, Arc<dyn Tool>)> {
        for (cat_name, category) in &self.categories {
            if let Some(tool) = category.find_tool(tool_name) {
                return Some((cat_name.clone(), tool));
            }
        }
        None
    }

    /// 获取所有工具 (扁平化)
    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.categories.values().flat_map(|c| c.tools()).collect()
    }
}

// ============================================================
//  具体工具类别实现
// ============================================================

/// 文件操作类工具
pub struct FileOpsCategory;

#[async_trait]
impl ToolCategory for FileOpsCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "file_ops".into(),
            display_name: "文件操作".into(),
            brief: "读写编辑文件".into(),
            description: "文件的读取、写入、编辑操作。支持文本文件和二进制文件。".into(),
            tools: vec!["read".into(), "write".into(), "edit".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::ReadTool),
            Arc::new(super::WriteTool),
            Arc::new(super::edit::EditTool),
        ]
    }
}

/// 命令执行类工具
pub struct CommandCategory;

#[async_trait]
impl ToolCategory for CommandCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "command".into(),
            display_name: "命令执行".into(),
            brief: "bash(默认shell)/terminal(多终端选择)。危险命令会被标记风险等级。".into(),
            description: "在终端中执行系统命令。bash 使用系统默认 shell；terminal 支持 PowerShell/CMD/Bash/WSL/SSH。危险命令自动分级: Safe/Low/Medium/High/Critical。".into(),
            tools: vec!["bash".into(), "terminal".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::BashTool),
            Arc::new(super::multi_shell::MultiShellTool),
        ]
    }
}

/// 搜索类工具
pub struct SearchCategory;

#[async_trait]
impl ToolCategory for SearchCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "search".into(),
            display_name: "搜索".into(),
            brief: "glob(按文件名)/grep(按内容)/web_search(联网搜索)。找文件用 glob，找代码用 grep。".into(),
            description: "文件名搜索、文件内容搜索、网络搜索。glob 支持通配符模式；grep 支持正则表达式和文件类型过滤；web_search 支持多语言搜索。".into(),
            tools: vec!["glob".into(), "grep".into(), "web_search".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::glob_tool::GlobTool),
            Arc::new(super::grep_tool::GrepTool),
            Arc::new(super::web_search::WebSearchTool::new()),
        ]
    }
}

/// 代码智能类工具
pub struct CodeIntelCategory;

#[async_trait]
impl ToolCategory for CodeIntelCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "code_intel".into(),
            display_name: "代码智能".into(),
            brief: "symbol_search(搜符号)/find_callers(查调用)/project_map(项目结构)。理解代码库用这组工具。".into(),
            description: "代码分析工具。symbol_search 按名称搜索函数/类/变量；find_callers 查找谁调用了某个函数；project_map 生成项目结构概览（文件数、符号数、语言分布）。".into(),
            tools: vec!["symbol_search".into(), "find_callers".into(), "project_map".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::code_intelligence::SymbolSearchTool),
            Arc::new(super::code_intelligence::FindCallersTool),
            Arc::new(super::code_intelligence::ProjectMapTool),
        ]
    }
}

/// 快照类工具
pub struct SnapshotCategory;

#[async_trait]
impl ToolCategory for SnapshotCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "snapshot".into(),
            display_name: "文件快照".into(),
            brief: "snapshot_history(查看历史)/snapshot_rollback(回溯)/snapshot_risky(高风险检测)。文件变更追踪。".into(),
            description: "行级文件快照系统。每次文件变更自动记录，支持按时间点回溯、变更风险评估（Normal/Large/Risky）、session/agent 追踪。".into(),
            tools: vec!["snapshot_history".into(), "snapshot_rollback".into(), "snapshot_risky".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::code_intelligence::file_snapshot::SnapshotHistoryTool),
            Arc::new(super::code_intelligence::file_snapshot::SnapshotRollbackTool),
            Arc::new(super::code_intelligence::file_snapshot::SnapshotRiskyTool),
        ]
    }
}

/// 通信类工具
pub struct CommunicationCategory;

#[async_trait]
impl ToolCategory for CommunicationCategory {
    fn meta(&self) -> CategoryMeta {
        CategoryMeta {
            name: "communication".into(),
            display_name: "通信".into(),
            brief: "ask_user(向用户提问)/send_message(Agent间消息)/list_peers(列出Agent)。需要用户输入时用 ask_user。".into(),
            description: "Agent 间消息传递和用户交互。ask_user 在需要用户确认或输入时使用；send_message/list_peers 用于多 Agent 协作场景。".into(),
            tools: vec!["send_message".into(), "list_peers".into(), "ask_user".into()],
        }
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(super::a2a_message::SendMessageTool),
            Arc::new(super::a2a_message::ListPeersTool),
            Arc::new(super::ask_user::AskUserTool),
        ]
    }
}

/// 创建默认的类别注册表
pub fn create_default_categories() -> CategoryRegistry {
    let mut registry = CategoryRegistry::new();
    registry.register(Arc::new(FileOpsCategory));
    registry.register(Arc::new(CommandCategory));
    registry.register(Arc::new(SearchCategory));
    registry.register(Arc::new(CodeIntelCategory));
    registry.register(Arc::new(SnapshotCategory));
    registry.register(Arc::new(CommunicationCategory));
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_registry() {
        let registry = create_default_categories();
        assert_eq!(registry.category_names().len(), 6);
        assert!(registry.get("file_ops").is_some());
        assert!(registry.get("search").is_some());
    }

    #[test]
    fn test_brief_list() {
        let registry = create_default_categories();
        let brief = registry.brief_list();
        assert!(brief.contains("文件操作"));
        assert!(brief.contains("搜索"));
        assert!(brief.contains("代码智能"));
    }

    #[test]
    fn test_find_tool() {
        let registry = create_default_categories();
        let (cat_name, tool) = registry.find_tool("read").unwrap();
        assert_eq!(cat_name, "file_ops");
        assert_eq!(tool.name(), "read");
    }

    #[test]
    fn test_expand_category() {
        let registry = create_default_categories();
        let desc = registry.expand_category("search").unwrap();
        assert!(desc.contains("glob"));
        assert!(desc.contains("grep"));
        assert!(desc.contains("web_search"));
    }

    #[test]
    fn test_all_tools() {
        let registry = create_default_categories();
        let tools = registry.all_tools();
        assert!(tools.len() >= 14); // 至少 14 个工具
    }
}
