//! Subsystem 声明与过滤

/// 所有已注册的 subsystem
pub const SUBSYSTEMS: &[&str] = &[
    "gateway",      // 网关调度
    "agent",        // Agent 运行时
    "tool",         // 工具执行
    "provider",     // LLM Provider
    "session",      // 会话管理
    "orchestrator", // 编排器
    "index",        // 代码索引
    "event",        // 事件总线
    "cache",        // 缓存系统
    "context",      // 上下文管理
];

/// 检查 subsystem 是否启用
pub fn is_enabled(subsystem: &str) -> bool {
    if let Ok(log_spec) = std::env::var("ORION_LOG") {
        // 检查是否有明确的 subsystem 过滤
        for part in log_spec.split(',') {
            let mut parts = part.splitn(2, ':');
            if let Some(name) = parts.next() {
                if name == subsystem {
                    return true;
                }
            }
        }
        // 如果没有明确指定, 默认启用
        !log_spec.contains(':')
    } else {
        true
    }
}
