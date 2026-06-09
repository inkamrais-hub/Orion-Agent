//! .orionignore 文件支持
//! 
//! 类似 .gitignore，定义 Agent 应忽略的文件和目录

use std::path::Path;

/// 检查路径是否应被忽略
pub fn should_ignore(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    
    // 默认忽略列表
    let ignore_patterns = [
        // 构建产物
        "target/", "build/", "dist/", "out/",
        // 依赖
        "node_modules/", "vendor/", ".cargo/",
        // 版本控制
        ".git/", ".svn/", ".hg/",
        // IDE
        ".vscode/", ".idea/", "*.swp", "*.swo",
        // 系统文件
        ".ds_store", "thumbs.db",
        // Orion 内部
        ".orion/",
        // 敏感文件
        ".env", ".env.local", "*.pem", "*.key",
    ];
    
    for pattern in &ignore_patterns {
        if pattern.contains('/') {
            // 目录模式
            if path_str.contains(pattern) {
                return true;
            }
        } else if pattern.starts_with("*.") {
            // 扩展名模式
            let ext = &pattern[2..];
            if path_str.ends_with(ext) {
                return true;
            }
        } else {
            // 文件名模式
            if path_str.contains(pattern) {
                return true;
            }
        }
    }
    
    false
}
