//! Session 内斜杠命令系统
//!
//! 注册由 `chat::loop` fallback 路径处理的命令 (/clear, /status, /history, /memory 等)。
//! /help, /new, /model, /resume, /think 等已在 `chat::loop::handle_command()` 中直接实现。

use std::collections::HashMap;

/// 斜杠命令处理器
pub type SlashHandler = Box<dyn Fn(&[&str]) -> String + Send + Sync>;

/// 斜杠命令定义
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub handler: SlashHandler,
}

/// 斜杠命令注册表
pub struct SlashRegistry {
    commands: HashMap<String, SlashCommand>,
}

impl SlashRegistry {
    pub fn new() -> Self {
        let mut registry = Self { commands: HashMap::new() };
        registry.register_builtins();
        registry
    }

    pub fn register(&mut self, cmd: SlashCommand) {
        self.commands.insert(cmd.name.clone(), cmd);
    }

    /// 处理输入, 如果是 /命令 则执行并返回结果, 否则返回 None
    pub fn handle(&self, input: &str) -> Option<String> {
        let input = input.trim();
        if !input.starts_with('/') {
            return None;
        }

        let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
        let cmd_name = parts[0];
        let args: Vec<&str> = if parts.len() > 1 {
            parts[1].split_whitespace().collect()
        } else {
            vec![]
        };

        if let Some(cmd) = self.commands.get(cmd_name) {
            Some((cmd.handler)(&args))
        } else {
            None
        }
    }

    /// 检查输入是否是斜杠命令
    pub fn is_command(input: &str) -> bool {
        input.trim().starts_with('/')
    }

    /// 注册内置命令
    fn register_builtins(&mut self) {
        // /clear - 清屏
        self.register(SlashCommand {
            name: "clear".into(),
            description: "清屏".into(),
            usage: "/clear".into(),
            handler: Box::new(|_| {
                print!("\x1B[2J\x1B[H");
                String::new()
            }),
        });

        // /history - 显示历史（占位，chat/loop 尚未实现真实版本，由 SlashRegistry fallback 处理）
        self.register(SlashCommand {
            name: "history".into(),
            description: "显示对话历史".into(),
            usage: "/history [count]".into(),
            handler: Box::new(|args| {
                let count = args.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
                format!("显示最近 {} 条历史 (功能开发中)", count)
            }),
        });

        // /memory - 查看记忆（占位，chat/loop 尚未实现真实版本，由 SlashRegistry fallback 处理）
        self.register(SlashCommand {
            name: "memory".into(),
            description: "查看记忆".into(),
            usage: "/memory".into(),
            handler: Box::new(|_| {
                "记忆系统 (功能开发中)".into()
            }),
        });

        // /status - 显示状态
        self.register(SlashCommand {
            name: "status".into(),
            description: "显示当前状态".into(),
            usage: "/status".into(),
            handler: Box::new(|_| {
                let os = std::env::consts::OS;
                let cwd = std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "unknown".into());
                format!("OS: {}\nCWD: {}\nVersion: {}", os, cwd, env!("CARGO_PKG_VERSION"))
            }),
        });

        // /sessions - 列出最近的 Session
        self.register(SlashCommand {
            name: "sessions".into(),
            description: "列出最近的会话".into(),
            usage: "/sessions [count]".into(),
            handler: Box::new(|args| {
                let count = args.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(5);
                // 尝试从默认路径加载
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".into());
                let db_path = std::path::PathBuf::from(home).join(".config").join("orion").join("sessions.db");

                if let Ok(store) = crate::session::store::SessionStore::new(&db_path) {
                    if let Ok(sessions) = store.list_sessions(count) {
                        if sessions.is_empty() {
                            return "没有历史会话。使用 /new 创建新会话".into();
                        }
                        let mut result = String::from("最近会话:\n");
                        for s in &sessions {
                            result.push_str(&format!(
                                "  {} ({}) - {} 轮, {} 工具调用\n",
                                s.session_id, s.status_str(), s.turn_count, s.tool_call_count
                            ));
                        }
                        result.push_str("\n使用 /resume <id> 恢复会话");
                        return result;
                    }
                }
                "无法加载会话列表".into()
            }),
        });

        // /delete - 删除 Session (软删除)
        self.register(SlashCommand {
            name: "delete".into(),
            description: "删除会话 (软删除，保留1周)".into(),
            usage: "/delete <session_id>".into(),
            handler: Box::new(|args| {
                if args.is_empty() {
                    return "用法: /delete <session_id>\n使用 /sessions 查看可用会话".into();
                }
                let session_id = args[0];

                // 验证 Session ID 格式
                if !session_id.starts_with("session_") {
                    return "无效的 Session ID (必须以 session_ 开头)".into();
                }

                let fm = crate::session::files::SessionFileManager::new();

                // 检查是否存在
                if !fm.session_exists(session_id) {
                    return format!("Session 不存在: {}", session_id);
                }

                // 软删除
                match fm.soft_delete(session_id) {
                    Ok(_) => {
                        // 更新 SQLite 状态
                        let fm2 = crate::session::files::SessionFileManager::new();
                        let _ = fm2.soft_delete(session_id);
                        format!("✓ 已删除 Session: {}\n已移至回收站，1周后自动清理", session_id)
                    }
                    Err(e) => format!("删除失败: {}", e),
                }
            }),
        });

        // /restore - 恢复已删除的 Session
        self.register(SlashCommand {
            name: "restore".into(),
            description: "恢复已删除的会话".into(),
            usage: "/restore <session_id>".into(),
            handler: Box::new(|args| {
                if args.is_empty() {
                    return "用法: /restore <session_id>".into();
                }
                let session_id = args[0];
                let fm = crate::session::files::SessionFileManager::new();

                match fm.restore(session_id) {
                    Ok(_) => {
                        format!("✓ 已恢复 Session: {}", session_id)
                    }
                    Err(e) => format!("恢复失败: {}", e),
                }
            }),
        });

        // /trash - 查看回收站
        self.register(SlashCommand {
            name: "trash".into(),
            description: "查看回收站".into(),
            usage: "/trash".into(),
            handler: Box::new(|_| {
                let fm = crate::session::files::SessionFileManager::new();
                match fm.list_trash_dirs() {
                    Ok(dirs) => {
                        if dirs.is_empty() {
                            return "回收站为空".into();
                        }
                        let mut result = String::from("回收站:\n");
                        for dir in &dirs {
                            result.push_str(&format!("  {} (已删除)\n", dir));
                        }
                        result.push_str("\n使用 /restore <id> 恢复");
                        result
                    }
                    Err(_) => "无法读取回收站".into(),
                }
            }),
        });
    }
}

impl Default for SlashRegistry {
    fn default() -> Self {
        Self::new()
    }
}
