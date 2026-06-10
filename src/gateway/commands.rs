//! 命令系统 — 子命令注册与路由

use super::GatewayContext;
use std::collections::HashMap;

/// 命令处理器
pub type CommandHandler = Box<
    dyn Fn(Vec<String>, GatewayContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send>>
        + Send
        + Sync,
>;

/// 命令定义
pub struct Command {
    pub name: String,
    pub description: String,
    pub handler: CommandHandler,
}

/// 命令注册表
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self { Self { commands: HashMap::new() } }
    pub fn register(&mut self, command: Command) { self.commands.insert(command.name.clone(), command); }
    pub fn get(&self, name: &str) -> Option<&Command> { self.commands.get(name) }
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.commands.values().map(|c| (c.name.as_str(), c.description.as_str())).collect()
    }
}

impl Default for CommandRegistry { fn default() -> Self { Self::new() } }

/// 路由命令
pub async fn route_command(
    command: &str,
    args: Vec<String>,
    ctx: GatewayContext,
) -> crate::Result<()> {
    let mut registry = CommandRegistry::new();
    register_builtin_commands(&mut registry);

    if let Some(cmd) = registry.get(command) {
        (cmd.handler)(args, ctx).await
    } else {
        if command.is_empty() {
            crate::cli::chat::run(ctx.config).await
        } else {
            Err(crate::Error::Config(format!("未知命令: {}", command)))
        }
    }
}

/// 注册内置命令
fn register_builtin_commands(registry: &mut CommandRegistry) {
    // Chat 命令
    registry.register(Command {
        name: "chat".into(),
        description: "启动交互式对话".into(),
        handler: Box::new(|_args, ctx| {
            Box::pin(async move {
                crate::cli::chat::run(ctx.config).await
            })
        }),
    });

    // API 命令
    #[cfg(feature = "api")]
    registry.register(Command {
        name: "api".into(),
        description: "启动 REST API 服务器".into(),
        handler: Box::new(|args, ctx| {
            Box::pin(async move {
                let port = args.first().and_then(|s| s.parse::<u16>().ok()).unwrap_or(8080);
                let store = std::sync::Arc::new(crate::session::UnifiedStore::open().await?);
                let api_state = std::sync::Arc::new(crate::api::ApiState {
                    store,
                    config: ctx.config,
                });
                let app = crate::api::create_router(api_state);
                let addr = format!("0.0.0.0:{}", port);
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                tracing::info!(addr = %addr, "API server starting");
                axum::serve(listener, app).await.map_err(|e| crate::Error::Agent(format!("API server error: {}", e)))
            })
        }),
    });

    // Run 命令
    registry.register(Command {
        name: "run".into(),
        description: "执行单次任务 (--sandbox 启用无网络沙箱)".into(),
        handler: Box::new(|args, ctx| {
            Box::pin(async move {
                // 解析参数: --image, --sandbox
                let mut image_paths: Vec<String> = Vec::new();
                let mut task_parts: Vec<String> = Vec::new();
                let mut sandbox = false;
                let mut i = 0;
                while i < args.len() {
                    if args[i] == "--image" && i + 1 < args.len() {
                        image_paths.push(args[i + 1].clone());
                        i += 2;
                    } else if args[i] == "--sandbox" {
                        sandbox = true;
                        i += 1;
                    } else {
                        task_parts.push(args[i].clone());
                        i += 1;
                    }
                }
                let task = task_parts.join(" ");
                if task.is_empty() {
                    return Err(crate::Error::Config("缺少任务描述".into()));
                }

                let images = load_images(&image_paths)?;
                let images = if images.is_empty() { None } else { Some(images) };

                let message = super::run_task_once(&task, &ctx.config, images, sandbox).await?;
                println!("{}", message);
                Ok(())
            })
        }),
    });

    // 配置命令
    registry.register(Command {
        name: "config".into(),
        description: "查看/修改配置".into(),
        handler: Box::new(|_args, _ctx| {
            Box::pin(async {
                println!("配置管理功能开发中...");
                Ok(())
            })
        }),
    });

    // 索引命令
    registry.register(Command {
        name: "index".into(),
        description: "索引项目代码".into(),
        handler: Box::new(|_args, _ctx| {
            Box::pin(async {
                println!("🔍 正在分析项目并建立代码索引...");
                let cwd = std::env::current_dir().map_err(|e| crate::Error::Config(e.to_string()))?;
                let mut index = crate::index::engine::CodeIndex::open(&cwd)?;
                match index.index() {
                    Ok(report) => {
                        println!("✓ 索引建立完成！");
                        println!("  • 总文件数: {}", report.total_files);
                        println!("  • 变更/新增文件: {}", report.changed_files);
                        println!("  • 提取符号数: {}", report.total_symbols);
                        println!("  • 耗时: {}ms", report.elapsed_ms);
                    }
                    Err(e) => {
                        eprintln!("❌ 建立索引失败: {}", e);
                    }
                }
                Ok(())
            })
        }),
    });

}

/// 加载图片文件并转为 Base64 ContentBlock
pub fn load_images(paths: &[String]) -> crate::Result<Vec<crate::core::provider::ContentBlock>> {
    use base64::Engine;
    let mut images = Vec::new();
    for path in paths {
        let img_path = std::path::Path::new(path);
        if !img_path.exists() {
            return Err(crate::Error::Config(format!("图片文件不存在: {}", path)));
        }
        let data = std::fs::read(img_path)
            .map_err(|e| crate::Error::Config(format!("读取图片失败: {}", e)))?;
        if data.len() > 20 * 1024 * 1024 {
            return Err(crate::Error::Config(format!("图片文件过大 ({}MB > 20MB)", data.len() / 1024 / 1024)));
        }
        let media_type = match img_path.extension().and_then(|e| e.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
        images.push(crate::core::provider::ContentBlock::Image {
            data: b64,
            media_type: media_type.to_string(),
        });
        eprintln!("📎 Image: {} ({}KB, {})", path, data.len() / 1024, media_type);
    }
    Ok(images)
}
