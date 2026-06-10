//! 敏感信息脱敏
//!
//! 在日志输出前自动脱敏 API Key、密码、Token 等敏感信息

use regex::Regex;
use std::sync::LazyLock;

/// API Key 模式 (sk-xxx, ghp_xxx, Bearer xxx)
static API_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]{36}|Bearer\s+[a-zA-Z0-9._-]{20,})").unwrap()
});

/// 密码/密钥模式
static PASSWORD_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(password|passwd|secret|token|api_key|api_secret|secret_key)\s*[=:]\s*\S+").unwrap()
});

/// 脱敏文本
pub fn redact(text: &str) -> String {
    let mut result = text.to_string();
    result = API_KEY_PATTERN.replace_all(&result, "***REDACTED***").to_string();
    result = PASSWORD_PATTERN.replace_all(&result, "$1=***REDACTED***").to_string();
    result
}

/// 检查文本是否包含敏感信息
pub fn contains_sensitive(text: &str) -> bool {
    API_KEY_PATTERN.is_match(text) || PASSWORD_PATTERN.is_match(text)
}

/// 脱敏审计事件中的值 (用于 ConfigChange)
pub fn redact_value(key: &str, value: &str) -> String {
    let lower_key = key.to_lowercase();
    if lower_key.contains("key") || lower_key.contains("password") 
        || lower_key.contains("secret") || lower_key.contains("token") {
        // 只显示前 4 位 (char-based to avoid UTF-8 panic)
        if value.chars().count() > 4 {
            let prefix: String = value.chars().take(4).collect();
            format!("{}***", prefix)
        } else {
            "***".to_string()
        }
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_api_key() {
        let text = "Using key sk-abc123def456ghi789jkl012mno345pqr678stu";
        let result = redact(text);
        assert!(result.contains("***REDACTED***"));
        assert!(!result.contains("sk-abc123"));
    }

    #[test]
    fn test_redact_password() {
        let text = "password=mysecret123";
        let result = redact(text);
        assert!(result.contains("***REDACTED***"));
        assert!(!result.contains("mysecret123"));
    }

    #[test]
    fn test_redact_value() {
        assert_eq!(redact_value("api_key", "sk-abc123def456"), "sk-a***");
        assert_eq!(redact_value("password", "secret123"), "secr***");
        assert_eq!(redact_value("name", "test"), "test");
    }

    #[test]
    fn test_contains_sensitive() {
        assert!(contains_sensitive("api_key=sk-abc123"));
        assert!(!contains_sensitive("hello world"));
    }
}
