//! 工作区安全系统 (简化版)
//! 
//! 只保护框架核心源码不被修改，其他文件用户自行负责
//! 
//! 设计原则:
//! - 框架源码 = 硬编码保护（绝对不能改）
//! - 其他文件 = 用户自己负责（不拦）
//! - 企业级安全 → 推荐虚拟机/容器隔离

use std::path::{Component, Path, PathBuf};

/// 框架核心路径（硬编码保护，不可配置）
const FRAMEWORK_PROTECTED_PATHS: &[&str] = &[
    "src/main.rs",
    "src/lib.rs",
    "src/core/",
    "src/tools/",
    "src/agent/",
    "src/orchestrator/",
    "src/cli/",
    "src/session/",
    "src/index/",
    "src/plugins/",
    "src/events/",
    "src/logging/",
    "src/audit/",
    "src/model/",
    "src/gateway/",
    "src/ui/",
    "src/config.rs",
    "src/telemetry.rs",
    "Cargo.toml",
    "Cargo.lock",
];

/// 解析路径 (处理不存在的文件)
/// 如果文件不存在，解析父目录再拼接文件名
fn resolve_path(path: &Path) -> std::path::PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    // 文件不存在，尝试解析父目录
    if let Some(parent) = path.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if let Some(file_name) = path.file_name() {
                return canonical_parent.join(file_name);
            }
        }
    }
    // 回退: 手动解析 .. 组件
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            other => components.push(other),
        }
    }
    components.iter().collect()
}

/// 规范化路径字符串（移除 `.` 和 `..` 分量，统一使用 `/`）
fn normalize_path(p: &str) -> String {
    let path = Path::new(p);
    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(s) => components.push(s.to_string_lossy().to_string()),
            Component::ParentDir => { components.pop(); }
            Component::CurDir => {} // 跳过 .
            _ => {}
        }
    }
    components.join("/")
}

/// 检查单条命令（不含 shell 连接符）是否安全
fn is_single_command_safe(cmd: &str) -> Result<(), String> {
    let lower = cmd.to_lowercase().replace('\\', "/");

    // 禁止 cd 命令（切换目录后可绕过路径保护）
    if lower.starts_with("cd ") || lower.starts_with("cd\t") || lower == "cd" {
        return Err("安全限制: 不允许使用 cd 命令切换目录".to_string());
    }

    // 写操作关键词列表
    let write_keywords = [
        ">", ">>", "write", "edit", "del", "rm ", "remove",
        "copy", "cp ", "move", "mv ", "set-content", "out-file",
        "tee ", "truncate", "install", "ln ", "chmod", "chown",
        "redirect", "invoke-webrequest", "new-item",
    ];

    let is_write = write_keywords.iter().any(|kw| lower.contains(kw));

    if is_write {
        for protected in FRAMEWORK_PROTECTED_PATHS {
            let protected_normalized = protected.replace('\\', "/");
            // 对命令中的路径参数做规范化后再匹配
            // 先做简单子串匹配（覆盖大多数场景）
            if lower.contains(&protected_normalized) {
                return Err(format!(
                    "安全限制: 命令可能修改框架核心文件 '{}'",
                    protected
                ));
            }
            // 对命令中提取的每个词做路径规范化后再匹配
            for word in lower.split_whitespace() {
                let normalized = normalize_path(word);
                if normalized.contains(&protected_normalized) {
                    return Err(format!(
                        "安全限制: 命令可能修改框架核心文件 '{}' (规范化后匹配)",
                        protected
                    ));
                }
            }
        }
    }

    Ok(())
}

/// 工作区守卫
pub struct WorkspaceGuard {
    /// 框架根目录（包含 src/ 的目录）
    framework_root: PathBuf,
}

impl WorkspaceGuard {
    pub fn new(_workspace_root: PathBuf) -> Self {
        // 框架根目录 = 当前可执行文件所在目录
        let framework_root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        
        Self { framework_root }
    }
    
    /// 检查文件是否可以写入
    pub fn can_write(&self, path: &Path) -> Result<(), String> {
        // 解析路径: 如果文件不存在，解析父目录再拼接文件名
        let resolved = resolve_path(path);
        let framework_root = self.framework_root.canonicalize()
            .unwrap_or_else(|_| self.framework_root.clone());
        
        // 只检查: 是否在框架目录内
        if resolved.starts_with(&framework_root) {
            // 在框架目录内，检查是否是受保护的核心文件
            if let Ok(relative) = resolved.strip_prefix(&framework_root) {
                // 统一使用 / 分隔符进行比较
                let relative_str = relative.to_string_lossy().replace('\\', "/");
                for protected in FRAMEWORK_PROTECTED_PATHS {
                    // 规范化保护路径
                    let protected_normalized = protected.replace('\\', "/");
                    if relative_str.starts_with(&protected_normalized) 
                        || relative_str.contains(&protected_normalized) {
                        return Err(format!(
                            "安全限制: 不能修改框架核心文件: {}",
                            path.display()
                        ));
                    }
                }
            }
        }
        
        // 不在框架目录内，或者不是核心文件 → 允许
        Ok(())
    }
    
    /// 检查命令是否安全
    pub fn is_command_safe(&self, cmd: &str) -> Result<(), String> {
        // 检查 shell 连接符，拆分后逐一检查每条子命令
        let shell_operators = ["&&", "||", "|", ";"];
        for op in &shell_operators {
            if cmd.contains(op) {
                let parts: Vec<&str> = cmd.split(op).collect();
                for part in parts {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        is_single_command_safe(trimmed)?;
                    }
                }
                return Ok(());
            }
        }

        // 无连接符，直接检查
        is_single_command_safe(cmd)
    }
}

/// 全局工作区守卫
use std::sync::LazyLock;
use tokio::sync::RwLock;

pub static WORKSPACE_GUARD: LazyLock<RwLock<Option<WorkspaceGuard>>> = 
    LazyLock::new(|| RwLock::new(None));

/// 初始化工作区守卫
pub async fn init_workspace_guard(workspace_root: PathBuf) {
    let mut guard = WORKSPACE_GUARD.write().await;
    *guard = Some(WorkspaceGuard::new(workspace_root));
}

/// 检查文件是否可以写入
pub async fn can_write_file(path: &Path) -> Result<(), String> {
    let guard = WORKSPACE_GUARD.read().await;
    if let Some(guard) = guard.as_ref() {
        guard.can_write(path)
    } else {
        Ok(())
    }
}

/// 检查命令是否安全
pub async fn is_command_safe(cmd: &str) -> Result<(), String> {
    let guard = WORKSPACE_GUARD.read().await;
    if let Some(guard) = guard.as_ref() {
        guard.is_command_safe(cmd)
    } else {
        Ok(())
    }
}
