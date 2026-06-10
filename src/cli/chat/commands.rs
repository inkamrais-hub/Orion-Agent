//! Session 内斜杠命令系统
//!
//! 注册由 `chat::loop` fallback 路径处理的命令 (/clear, /status, /history, /memory 等)。
//! /help, /new, /model, /resume, /think 等已在 `chat::loop::handle_command()` 中直接实现。
//!
//! 所有 handler 均为异步，通过 UnifiedStore 访问持久化数据。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

/// 异步斜杠命令处理器
pub type SlashHandler = Box<
    dyn Fn(Vec<String>) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync,
>;

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
    pub async fn handle(&self, input: &str) -> Option<String> {
        let input = input.trim();
        if !input.starts_with('/') {
            return None;
        }

        let parts: Vec<&str> = input[1..].splitn(2, ' ').collect();
        let cmd_name = parts[0];
        let args: Vec<String> = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };

        if let Some(cmd) = self.commands.get(cmd_name) {
            Some((cmd.handler)(args).await)
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
            handler: Box::new(|_args| {
                Box::pin(async move {
                    print!("\x1B[2J\x1B[H");
                    String::new()
                })
            }),
        });

        // /history - 显示历史
        self.register(SlashCommand {
            name: "history".into(),
            description: "显示对话历史".into(),
            usage: "/history [count]".into(),
            handler: Box::new(|args| {
                Box::pin(async move {
                    let count = args.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(10);
                    format!("显示最近 {} 条历史 (功能开发中)", count)
                })
            }),
        });

        // /memory - 查看记忆
        self.register(SlashCommand {
            name: "memory".into(),
            description: "查看记忆".into(),
            usage: "/memory".into(),
            handler: Box::new(|_args| {
                Box::pin(async move {
                    "记忆系统 (功能开发中)".into()
                })
            }),
        });

        // /status - 显示状态
        self.register(SlashCommand {
            name: "status".into(),
            description: "显示当前状态".into(),
            usage: "/status".into(),
            handler: Box::new(|_args| {
                Box::pin(async move {
                    let os = std::env::consts::OS;
                    let cwd = std::env::current_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| "unknown".into());
                    format!("OS: {}\nCWD: {}\nVersion: {}", os, cwd, env!("CARGO_PKG_VERSION"))
                })
            }),
        });

        // /sessions - 列出最近的 Session (异步 UnifiedStore)
        self.register(SlashCommand {
            name: "sessions".into(),
            description: "列出最近的会话".into(),
            usage: "/sessions [count]".into(),
            handler: Box::new(|args| {
                Box::pin(async move {
                    let count: u32 = args.first()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(10);

                    match crate::session::UnifiedStore::open().await {
                        Ok(store) => {
                            match store.list_sessions(count).await {
                                Ok(sessions) => {
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
                                    result
                                }
                                Err(e) => format!("无法加载会话列表: {}", e),
                            }
                        }
                        Err(e) => format!("无法打开数据库: {}", e),
                    }
                })
            }),
        });

        // /delete - 删除 Session (软删除 → 标记为 Deleted)
        self.register(SlashCommand {
            name: "delete".into(),
            description: "删除会话 (软删除)".into(),
            usage: "/delete <session_id>".into(),
            handler: Box::new(|args| {
                Box::pin(async move {
                    if args.is_empty() {
                        return "用法: /delete <session_id>\n使用 /sessions 查看可用会话".into();
                    }
                    let session_id = &args[0];

                    match crate::session::UnifiedStore::open().await {
                        Ok(store) => {
                            match store.update_session_status(
                                session_id,
                                crate::session::store::SessionStatus::Deleted,
                            ).await {
                                Ok(()) => format!("✓ 已删除 Session: {}", session_id),
                                Err(e) => format!("删除失败: {}", e),
                            }
                        }
                        Err(e) => format!("无法打开数据库: {}", e),
                    }
                })
            }),
        });

        // /restore - 恢复已删除的 Session
        self.register(SlashCommand {
            name: "restore".into(),
            description: "恢复已删除的会话".into(),
            usage: "/restore <session_id>".into(),
            handler: Box::new(|args| {
                Box::pin(async move {
                    if args.is_empty() {
                        return "用法: /restore <session_id>".into();
                    }
                    let session_id = &args[0];

                    match crate::session::UnifiedStore::open().await {
                        Ok(store) => {
                            match store.update_session_status(
                                session_id,
                                crate::session::store::SessionStatus::Active,
                            ).await {
                                Ok(()) => format!("✓ 已恢复 Session: {}", session_id),
                                Err(e) => format!("恢复失败: {}", e),
                            }
                        }
                        Err(e) => format!("无法打开数据库: {}", e),
                    }
                })
            }),
        });

        // /trash - 查看已删除的 Session
        self.register(SlashCommand {
            name: "trash".into(),
            description: "查看已删除的会话".into(),
            usage: "/trash".into(),
            handler: Box::new(|_args| {
                Box::pin(async move {
                    match crate::session::UnifiedStore::open().await {
                        Ok(store) => {
                            match store.list_sessions(100).await {
                                Ok(sessions) => {
                                    let deleted: Vec<_> = sessions.iter()
                                        .filter(|s| s.status == crate::session::store::SessionStatus::Deleted)
                                        .collect();
                                    if deleted.is_empty() {
                                        return "回收站为空".into();
                                    }
                                    let mut result = String::from("已删除会话:\n");
                                    for s in &deleted {
                                        result.push_str(&format!("  {} ({})\n", s.session_id, s.status_str()));
                                    }
                                    result.push_str("\n使用 /restore <id> 恢复");
                                    result
                                }
                                Err(_) => "无法读取会话列表".into(),
                            }
                        }
                        Err(e) => format!("无法打开数据库: {}", e),
                    }
                })
            }),
        });
    }
}

impl Default for SlashRegistry {
    fn default() -> Self {
        Self::new()
    }
}
