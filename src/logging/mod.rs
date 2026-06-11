//! Orion 标准化日志系统
//!
//! 分层架构:
//!   Layer 1: 日志接口 (本文件) — 统一宏 + 初始化
//!   Layer 2: 审计系统 — 基于日志的审计记录
//!   Layer 3: 上层系统 — Gateway 调度
//!   Layer 4: Hook 系统 — 工具拦截/修改
//!
//! 使用方式:
//!   log_info!("gateway", "服务启动完成");
//!   log_error!("tool", "工具执行失败: {}", err);
//!   log_debug!("agent", "LLM 响应: {}", response);
//!
//! 环境变量:
//!   ORION_LOG=gateway:debug,agent:info,tool:warn
//!   ORION_LOG_FILE=/path/to/logfile.log

pub mod subsystem;
pub mod redact;

use tracing_subscriber::{fmt, EnvFilter, Layer};
use tracing_subscriber::layer::SubscriberExt;
use std::path::PathBuf;
use std::sync::Once;

/// 全局初始化标志
static INIT: Once = Once::new();

/// 初始化日志系统 (幂等，多次调用只执行一次)
pub fn init_logging() {
    INIT.call_once(|| {
        do_init();
    });
}

fn do_init() {
    let filter = build_filter();

    // 只用 stderr 输出 (简洁)
    let subscriber = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(false)
                .with_line_number(false)
                .with_timer(tracing_subscriber::fmt::time::uptime())
                .with_filter(filter)
        );

    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("Warning: tracing subscriber already set, logging may not work");
    }

    tracing::info!(
        subsystem = "gateway",
        "Orion 日志系统初始化完成"
    );
}

/// 从 ORION_LOG 环境变量构建过滤器
fn build_filter() -> EnvFilter {
    if let Ok(log_spec) = std::env::var("ORION_LOG") {
        EnvFilter::new(log_spec)
    } else {
        EnvFilter::new("info")
    }
}

/// 获取日志文件路径
pub fn log_file_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ORION_LOG_FILE") {
        Some(PathBuf::from(path))
    } else {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        Some(PathBuf::from(home).join(".orion").join("orion.log"))
    }
}

// ── 标准化日志宏 ──────────────────────────────────────────

/// 日志宏: 带 subsystem 标签
///
/// 用法:
///   log_info!("gateway", "服务启动");
///   log_error!("tool", "执行失败: {}", err);
#[macro_export]
macro_rules! log_info {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::info!(subsystem = $subsystem, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_warn {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::warn!(subsystem = $subsystem, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_error {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::error!(subsystem = $subsystem, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_debug {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::debug!(subsystem = $subsystem, $($arg)*)
    };
}

#[macro_export]
macro_rules! log_trace {
    ($subsystem:expr, $($arg:tt)*) => {
        tracing::trace!(subsystem = $subsystem, $($arg)*)
    };
}
