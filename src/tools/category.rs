//! 工具聚类系统 — ToolCluster
//!
//! 设计理念:
//!   每个 cluster 是一组相关工具的集合。
//!   System prompt 只写 cluster 简介（短），不展开每个工具的完整 schema。
//!   Tool schema 仍然通过 function calling 传给 LLM（不变）。
//!   Sub-agent 通过 allowed_clusters 限制可见工具范围。
//!
//! 优势:
//!   1. Sys prompt 简短且稳定 → prompt cache 命中率高
//!   2. Sub-agent 只能看到被允许的 cluster → 连线即权限
//!   3. 新增工具只需注册到对应 cluster
//!   4. 主 agent 全能力，sub-agent 按需分配

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 工具聚类元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMeta {
    /// 聚类标识 (如 "file_ops")
    pub name: String,
    /// 显示名 (如 "文件操作")
    pub display_name: String,
    /// 一句话简介 (用于 system prompt)
    pub brief: String,
    /// 包含的工具名称列表
    pub tool_names: Vec<String>,
}

/// 工具聚类定义
///
/// 纯数据，不持有工具实例。
/// 工具实例由 ToolRegistry 统一管理，通过 tool_name 关联。
pub struct ToolCluster {
    pub meta: ClusterMeta,
    /// 系统提示片段 — 注入 Agent 的 system prompt
    /// 告诉 LLM 这类工具的使用规则和注意事项
    pub sys_prompt_fragment: String,
}

/// 聚类注册表
pub struct ClusterRegistry {
    clusters: HashMap<String, ToolCluster>,
    /// tool_name → cluster_name 的反向索引
    tool_to_cluster: HashMap<String, String>,
}

impl Default for ClusterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterRegistry {
    pub fn new() -> Self {
        Self {
            clusters: HashMap::new(),
            tool_to_cluster: HashMap::new(),
        }
    }

    /// 注册一个工具聚类
    pub fn register(&mut self, cluster: ToolCluster) {
        let cluster_name = cluster.meta.name.clone();
        for tool_name in &cluster.meta.tool_names {
            self.tool_to_cluster.insert(tool_name.clone(), cluster_name.clone());
        }
        self.clusters.insert(cluster_name, cluster);
    }

    /// 查找工具所属的聚类
    pub fn cluster_of(&self, tool_name: &str) -> Option<&str> {
        self.tool_to_cluster.get(tool_name).map(|s| s.as_str())
    }

    /// 获取聚类
    pub fn get(&self, name: &str) -> Option<&ToolCluster> {
        self.clusters.get(name)
    }

    /// 获取所有聚类名称
    pub fn cluster_names(&self) -> Vec<String> {
        self.clusters.keys().cloned().collect()
    }

    /// 获取所有聚类元数据
    pub fn all_meta(&self) -> Vec<ClusterMeta> {
        self.clusters.values().map(|c| c.meta.clone()).collect()
    }

    /// 生成 system prompt 的聚类概述（所有聚类）
    pub fn sys_prompt_all(&self) -> String {
        let mut prompt = String::new();
        // 按固定顺序输出，保证 prompt 稳定性（有利于 cache 命中）
        let order = ["file_ops", "search", "code_intel", "command", "snapshot", "communication", "sub_agent"];
        for name in &order {
            if let Some(cluster) = self.clusters.get(*name) {
                prompt.push_str(&cluster.sys_prompt_fragment);
                prompt.push('\n');
            }
        }
        // 动态聚类（MCP 等）追加到末尾
        for (name, cluster) in &self.clusters {
            if !order.contains(&name.as_str()) {
                prompt.push_str(&cluster.sys_prompt_fragment);
                prompt.push('\n');
            }
        }
        prompt
    }

    /// 生成 system prompt 的聚类概述（仅指定聚类）
    pub fn sys_prompt_for_clusters(&self, allowed: &[String]) -> String {
        let mut prompt = String::new();
        for name in allowed {
            if let Some(cluster) = self.clusters.get(name) {
                prompt.push_str(&cluster.sys_prompt_fragment);
                prompt.push('\n');
            }
        }
        prompt
    }

    /// 聚类数量
    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }
}

// ============================================================
//  默认聚类定义
// ============================================================

/// 创建默认的工具聚类注册表
pub fn create_default_clusters() -> ClusterRegistry {
    let mut reg = ClusterRegistry::new();

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "file_ops".into(),
            display_name: "文件操作".into(),
            brief: "read/write/edit — 读写编辑文件".into(),
            tool_names: vec!["read".into(), "write".into(), "edit".into()],
        },
        sys_prompt_fragment: "[file_ops] read/write/edit\n\
            read: Read file content (max 256KB, supports offset/limit for large files, binary hex dump).\n\
            write: Create or overwrite a file (creates parent dirs). ALWAYS read before write.\n\
            edit: Precise string replacement (old_string → new_string). Supports replace_all.\n\
            Rule: Always read a file before modifying it. Use edit for partial changes, write for new files."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "search".into(),
            display_name: "搜索".into(),
            brief: "glob/grep/web_search — 按名/按内容/联网搜索".into(),
            tool_names: vec!["glob".into(), "grep".into(), "web_search".into()],
        },
        sys_prompt_fragment: "[search] glob/grep/web_search\n\
            glob: Find files by pattern (e.g. **/*.rs). Use for file discovery.\n\
            grep: Search file content by regex. Supports file type filter (-t js) and context lines.\n\
            web_search: Search the internet (Bing/DuckDuckGo). Returns titles, URLs, snippets.\n\
            Rule: Finding files → glob. Finding code → grep. Finding info → web_search."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "code_intel".into(),
            display_name: "代码智能".into(),
            brief: "symbol_search/find_callers/project_map/skeleton — 代码分析".into(),
            tool_names: vec![
                "symbol_search".into(), "find_callers".into(),
                "project_map".into(), "skeleton".into(),
            ],
        },
        sys_prompt_fragment: "[code_intel] symbol_search/find_callers/project_map/skeleton\n\
            symbol_search: Search functions/classes/variables by name across the codebase.\n\
            find_callers: Find who calls a specific function (call graph analysis).\n\
            project_map: Generate project structure overview (file count, languages, key symbols).\n\
            skeleton: Show the structural outline of a file (functions, classes, imports).\n\
            Rule: Use these to understand code architecture before making changes."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "command".into(),
            display_name: "命令执行".into(),
            brief: "bash/terminal — 执行系统命令".into(),
            tool_names: vec!["bash".into(), "terminal".into()],
        },
        sys_prompt_fragment: "[command] bash/terminal\n\
            bash: Execute shell command (default shell: PowerShell on Windows, sh on Unix). Timeout 120s.\n\
            terminal: Multi-terminal support (PowerShell/CMD/Bash/WSL/SSH). For when you need a specific shell.\n\
            Risk levels: Safe→Low→Medium→High→Critical. Critical commands (rm -rf /) are blocked.\n\
            Rule: Chain commands with && for sequential execution. Use specific commands to avoid output bloat."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "snapshot".into(),
            display_name: "文件快照".into(),
            brief: "snapshot_history/snapshot_rollback/snapshot_risky — 文件变更追踪".into(),
            tool_names: vec![
                "snapshot_history".into(), "snapshot_rollback".into(), "snapshot_risky".into(),
            ],
        },
        sys_prompt_fragment: "[snapshot] snapshot_history/snapshot_rollback/snapshot_risky\n\
            snapshot_history: View change history for a file (who changed what, when).\n\
            snapshot_rollback: Restore a file to a previous snapshot state.\n\
            snapshot_risky: List files with >80% lines changed (potential mistakes).\n\
            Rule: Check snapshot_risky before finishing complex tasks to catch accidental overwrites."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "communication".into(),
            display_name: "通信".into(),
            brief: "ask_user/send_message/list_peers — 用户交互与 Agent 通信".into(),
            tool_names: vec!["ask_user".into(), "send_message".into(), "list_peers".into()],
        },
        sys_prompt_fragment: "[communication] ask_user/send_message/list_peers\n\
            ask_user: Ask the user a question (use when you need confirmation or input).\n\
            send_message: Send a message to another agent (multi-agent collaboration).\n\
            list_peers: List available agents in the current session.\n\
            Rule: Use ask_user before destructive operations. Multi-agent messaging requires active peers."
            .into(),
    });

    reg.register(ToolCluster {
        meta: ClusterMeta {
            name: "sub_agent".into(),
            display_name: "子代理".into(),
            brief: "create_sub_agent — 创建子代理执行子任务".into(),
            tool_names: vec!["create_sub_agent".into()],
        },
        sys_prompt_fragment: "[sub_agent] create_sub_agent\n\
            create_sub_agent: Spawn a sub-agent for a specific task. Sub-agents have their own tool scope.\n\
            Rule: Use for parallel work or when a task needs isolated context. Don't nest sub-agents."
            .into(),
    });

    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_registry() {
        let reg = create_default_clusters();
        assert_eq!(reg.len(), 7);
        assert!(reg.get("file_ops").is_some());
        assert!(reg.get("search").is_some());
        assert!(reg.get("sub_agent").is_some());
    }

    #[test]
    fn test_tool_to_cluster() {
        let reg = create_default_clusters();
        assert_eq!(reg.cluster_of("read"), Some("file_ops"));
        assert_eq!(reg.cluster_of("bash"), Some("command"));
        assert_eq!(reg.cluster_of("grep"), Some("search"));
        assert_eq!(reg.cluster_of("symbol_search"), Some("code_intel"));
        assert_eq!(reg.cluster_of("snapshot_history"), Some("snapshot"));
        assert_eq!(reg.cluster_of("ask_user"), Some("communication"));
        assert_eq!(reg.cluster_of("create_sub_agent"), Some("sub_agent"));
        assert_eq!(reg.cluster_of("nonexistent"), None);
    }

    #[test]
    fn test_sys_prompt_all() {
        let reg = create_default_clusters();
        let prompt = reg.sys_prompt_all();
        assert!(prompt.contains("[file_ops]"));
        assert!(prompt.contains("[search]"));
        assert!(prompt.contains("[command]"));
        assert!(prompt.contains("[sub_agent]"));
    }

    #[test]
    fn test_sys_prompt_for_clusters() {
        let reg = create_default_clusters();
        let prompt = reg.sys_prompt_for_clusters(&["file_ops".into(), "search".into()]);
        assert!(prompt.contains("[file_ops]"));
        assert!(prompt.contains("[search]"));
        assert!(!prompt.contains("[command]"));
        assert!(!prompt.contains("[sub_agent]"));
    }

    #[test]
    fn test_cluster_order_stability() {
        let reg = create_default_clusters();
        let p1 = reg.sys_prompt_all();
        let p2 = reg.sys_prompt_all();
        // 同一个 cluster 注册表，输出应该完全一致（cache 友好）
        assert_eq!(p1, p2);
    }
}
