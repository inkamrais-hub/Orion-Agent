//! .orionignore 文件支持
//!
//! 类似 .gitignore，定义 Agent 应忽略的文件和目录

use std::path::Path;

/// 检查路径是否应被忽略
pub fn should_ignore(path: &Path) -> bool {
    should_ignore_with_reason(path).is_some()
}

/// 检查路径是否应被忽略，并返回忽略原因
///
/// 返回 `Some(reason)` 表示应忽略该路径，`reason` 说明忽略原因；
/// 返回 `None` 表示不应忽略。
pub fn should_ignore_with_reason(path: &Path) -> Option<String> {
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
                return Some(format!("matches ignore pattern '{}'", pattern));
            }
        } else if pattern.starts_with("*.") {
            // 扩展名模式
            let ext = &pattern[2..];
            if path_str.ends_with(ext) {
                return Some(format!("matches extension pattern '{}'", pattern));
            }
        } else {
            // 文件名模式
            if path_str.contains(pattern) {
                return Some(format!("matches ignore pattern '{}'", pattern));
            }
        }
    }

    // 检查敏感文件 (额外保护)
    if is_sensitive_file_path(&path_str) {
        return Some("sensitive file detected".into());
    }

    None
}

/// 检查路径是否为敏感文件
///
/// 识别 `.env`、`.pem`、`.key`、`credentials.*`、`id_rsa` 等含有
/// 凭据或密钥的文件，以及 `secrets` 目录下的文件。
pub fn is_sensitive_file_path(path_str: &str) -> bool {
    let path_lower = path_str.to_lowercase();

    // 检查文件名 (取最后一段路径)
    let file_name = path_lower.rsplit(['/', '\\']).next().unwrap_or(&path_lower);

    // 敏感文件名前缀
    if file_name.starts_with(".env") {
        return true;
    }

    // 敏感文件扩展名
    let sensitive_exts = ["pem", "key", "p12", "pfx", "jks", "keystore"];
    if let Some(ext) = file_name.rsplit('.').next() {
        if sensitive_exts.contains(&ext) {
            return true;
        }
    }

    // 敏感文件名模式
    let sensitive_names = [
        "credentials.json",
        "credentials.yaml",
        "credentials.yml",
        "credentials.toml",
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        "service_account.json",
        "gcloud.json",
        "aws_credentials",
        ".htpasswd",
        ".netrc",
        ".npmrc",
        ".pypirc",
    ];
    if sensitive_names.iter().any(|n| file_name == *n) {
        return true;
    }

    // secrets 目录
    if path_lower.contains("secrets/") || path_lower.contains("secrets\\") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_should_ignore_target_dir() {
        assert!(should_ignore(Path::new("target/debug/build")));
    }

    #[test]
    fn test_should_ignore_env_file() {
        assert!(should_ignore(Path::new(".env")));
        assert!(should_ignore(Path::new(".env.local")));
    }

    #[test]
    fn test_should_ignore_pem_file() {
        assert!(should_ignore(Path::new("certs/server.pem")));
    }

    #[test]
    fn test_should_not_ignore_normal_file() {
        assert!(!should_ignore(Path::new("src/main.rs")));
    }

    #[test]
    fn test_should_ignore_with_reason_returns_reason() {
        let reason = should_ignore_with_reason(Path::new("target/debug"));
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("target/"));
    }

    #[test]
    fn test_should_ignore_with_reason_none_for_normal() {
        let reason = should_ignore_with_reason(Path::new("src/main.rs"));
        assert!(reason.is_none());
    }

    #[test]
    fn test_is_sensitive_file_credentials() {
        assert!(is_sensitive_file_path("config/credentials.json"));
        assert!(is_sensitive_file_path("credentials.yaml"));
    }

    #[test]
    fn test_is_sensitive_file_key() {
        assert!(is_sensitive_file_path("server.key"));
        assert!(is_sensitive_file_path("certs/server.pem"));
    }

    #[test]
    fn test_is_sensitive_file_env() {
        assert!(is_sensitive_file_path(".env"));
        assert!(is_sensitive_file_path(".env.production"));
    }

    #[test]
    fn test_is_sensitive_file_normal() {
        assert!(!is_sensitive_file_path("src/main.rs"));
        assert!(!is_sensitive_file_path("Cargo.toml"));
    }

    #[test]
    fn test_is_sensitive_file_secrets_dir() {
        assert!(is_sensitive_file_path("secrets/config.yaml"));
    }
}
