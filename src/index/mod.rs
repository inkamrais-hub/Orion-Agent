pub mod engine;
pub mod skeleton;
pub use engine::CodeIndex;

use std::sync::LazyLock;
use tokio::sync::Mutex;

/// 持久化索引连接 (全局单例, 所有工具共享)
/// 使用 tokio::sync::Mutex 避免阻塞异步运行时
pub static CODE_INDEX: LazyLock<Mutex<Option<CodeIndex>>> =
    LazyLock::new(|| Mutex::new(None));
