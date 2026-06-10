//! .orionignore 文件支持
//! 
//! 类似 .gitignore，定义 Agent 应忽略的文件和目录

use std::path::Path;

/// 检查路径是否应被忽略
pub fn should_ignore(path: &Path) -> bool {
    // 提取路径组件 (小写) 用于精确匹配
    let components: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect();

    // 提取文件名 (小写)
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // 默认忽略列表
    let ignore_patterns: &[&str] = &[
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

    for pattern in ignore_patterns {
        if let Some(dir_name) = pattern.strip_suffix('/') {
            // 目录模式: 匹配路径中的目录组件 (精确匹配，非子串)
            if components.iter().any(|c| c == dir_name) {
                return true;
            }
        } else if let Some(rest) = pattern.strip_prefix("*.") {
            // 扩展名模式: 使用标准扩展名匹配
            if matches_ext_pattern(&filename, rest) {
                return true;
            }
        } else {
            // 精确文件名模式: 匹配文件名或任意路径组件
            if filename == *pattern || components.iter().any(|c| c == *pattern) {
                return true;
            }
        }
    }

    false
}

/// 匹配扩展名模式 (支持 .env.local 这样的复合扩展名和 *.{ext1,ext2} 语法)
fn matches_ext_pattern(filename: &str, ext_pattern: &str) -> bool {
    // 支持 *.{ext1,ext2} 语法
    if ext_pattern.starts_with('{') && ext_pattern.ends_with('}') {
        let exts = &ext_pattern[1..ext_pattern.len() - 1];
        return exts.split(',').any(|ext| {
            let ext = ext.trim();
            filename.ends_with(&format!(".{}", ext))
        });
    }
    // 标准扩展名匹配: 检查 ".ext" 后缀
    filename.ends_with(&format!(".{}", ext_pattern))
}
