use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};

/// 检查路径是否为禁止访问的系统目录
fn is_blocked_path(path: &str) -> bool {
    let canonical = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));
    let lower = canonical.to_string_lossy().to_lowercase();
    let blocked = [
        "\\windows\\", "/windows/",
        "\\system32\\", "/system32/",
        "\\etc\\", "/etc/",
        "\\proc\\", "/proc/",
        "\\sys\\", "/sys/",
    ];
    for b in &blocked {
        if lower.contains(b) {
            return true;
        }
    }
    false
}

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str {
        "Find files by name pattern. Supports glob patterns like '*.rs', 'src/**/*.ts', '**/*.json'. \
         Supports recursive ** matching and ignore patterns. \
         Returns matching file paths with sizes, sorted by modification time. \
         Use to find files before reading them."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern (e.g. '*.rs', 'src/**/*.ts', '**/*.json')"},
                "path": {"type": "string", "description": "Directory to search in (default: current working directory)"},
                "ignore": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Directory/file names to exclude (e.g. ['target', 'node_modules', '.git']). Default exclusions still apply."
                }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let pattern = input["pattern"].as_str().ok_or_else(|| crate::Error::Tool("missing pattern".into()))?;
        let root = input["path"].as_str().unwrap_or(&ctx.working_dir);

        // 路径安全检查: 禁止访问系统目录
        if is_blocked_path(root) {
            return Ok(ToolResult {
                content: format!("安全拦截: 禁止访问系统目录 '{}'", root),
                is_error: true,
                metadata: None,
            });
        }

        // 解析 ignore 列表 (追加到默认排除)
        let user_ignore: Vec<String> = input["ignore"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let mut matches = Vec::new();
        find_matches(std::path::Path::new(root), std::path::Path::new(root), pattern, &user_ignore, &mut matches, 0);

        // Sort by modification time (newest first)
        matches.sort_by(|a, b| {
            let a_time = std::fs::metadata(a).and_then(|m| m.modified()).ok();
            let b_time = std::fs::metadata(b).and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        // Limit results
        matches.truncate(50);

        if matches.is_empty() {
            Ok(ToolResult {
                content: format!("No files matching '{}' found in {}", pattern, root),
                is_error: false,
                metadata: None,
            })
        } else {
            // 格式化: 路径 + 文件大小
            let lines: Vec<String> = matches.iter()
                .map(|p| {
                    let rel = p.strip_prefix(root).unwrap_or(p).to_string_lossy().to_string();
                    let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                    format!("{} ({})", rel, format_size(size))
                })
                .collect();
            Ok(ToolResult {
                content: lines.join("\n"),
                is_error: false,
                metadata: Some(json!({"count": matches.len()})),
            })
        }
    }
}

/// 默认排除的目录名
const DEFAULT_IGNORES: &[&str] = &["target", "node_modules", ".git"];

fn find_matches(
    dir: &std::path::Path,
    root: &std::path::Path,
    pattern: &str,
    user_ignore: &[String],
    results: &mut Vec<std::path::PathBuf>,
    depth: usize,
) {
    if depth > 10 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            // 跳过隐藏目录
            if name.starts_with('.') {
                continue;
            }
            // 跳过默认排除 + 用户排除
            if DEFAULT_IGNORES.contains(&name.as_str()) || user_ignore.contains(&name) {
                continue;
            }
            find_matches(&path, root, pattern, user_ignore, results, depth + 1);
        } else {
            let rel_path = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
            if matches_glob(&rel_path, pattern) {
                results.push(path);
            }
        }
    }
}

/// 增强的 glob 匹配: 支持 ** 递归、* 通配、*.{ext} 多扩展名
fn matches_glob(path: &str, pattern: &str) -> bool {
    // 通配所有
    if pattern == "*" { return true; }

    // 路径归一化: Windows 反斜杠转正斜杠
    let path_normalized = path.replace('\\', "/");

    if pattern.contains("**") {
        // ** 递归匹配: 分割为 prefix 和 suffix
        let parts: Vec<&str> = pattern.splitn(2, "**").collect();
        let prefix = parts[0].trim_end_matches('/');
        let suffix = parts.get(1).map(|s| s.trim_start_matches('/')).unwrap_or("");

        // 前缀匹配: 路径开头必须匹配 prefix
        let prefix_match = prefix.is_empty()
            || path_normalized == prefix
            || path_normalized.starts_with(&format!("{}/", prefix));

        if !prefix_match { return false; }

        // 后缀匹配: 路径末尾必须匹配 suffix
        if suffix.is_empty() { return true; }

        // 检查后缀是否以路径分隔符正确衔接
        let suffix_start_ok = if path_normalized.len() > prefix.len() {
            let after_prefix = &path_normalized[prefix.len()..];
            after_prefix.starts_with('/') || prefix.is_empty()
        } else {
            prefix.is_empty()
        };

        if !suffix_start_ok { return false; }

        // suffix 可能是 "**/*.ts" (再次包含 **) 或 "*.ts" 等
        if suffix.contains("**") {
            // 递归处理嵌套 **
            return matches_glob(&path_normalized, suffix);
        }

        // suffix 包含 '/' (多段路径): 检查路径是否以 suffix 结尾
        if suffix.contains('/') {
            return path_normalized.ends_with(&format!("/{}", suffix))
                || path_normalized == suffix;
        }

        // suffix 是单段 (如 "*.ts"): 只匹配文件名
        let filename = path_normalized.rsplit('/').next().unwrap_or(&path_normalized);
        return matches_glob_segment(filename, suffix);
    }

    // 分割 pattern 为路径段
    let parts: Vec<&str> = pattern.split('/').collect();

    // 单段模式 (无 /): 只匹配文件名
    if parts.len() == 1 {
        let filename = path_normalized.rsplit('/').next().unwrap_or(&path_normalized);
        return matches_glob_segment(filename, parts[0]);
    }

    // 多段模式: 匹配完整相对路径
    let pattern_path = parts.join("/");
    if path_normalized == pattern_path {
        return true;
    }
    // 也尝试只匹配文件名 (兼容 "src/*.rs" 对 "src/foo.rs" 的匹配)
    if let Some(last) = parts.last() {
        let filename = path_normalized.rsplit('/').next().unwrap_or(&path_normalized);
        if matches_glob_segment(filename, last) {
            // 确保路径前缀也匹配
            let expected_prefix = &parts[..parts.len() - 1].join("/");
            let path_prefix = path_normalized.rsplit_once('/').map(|x| x.0).unwrap_or("");
            return path_prefix == expected_prefix
                || path_prefix.ends_with(&format!("/{}", expected_prefix));
        }
    }

    false
}

/// 匹配单个 glob 段 (支持 *, **, *.{ext1,ext2}, exact)
fn matches_glob_segment(name: &str, segment: &str) -> bool {
    // ** 匹配任意
    if segment == "**" { return true; }

    // *.{ext1,ext2,...} 多扩展名
    if let Some(rest) = segment.strip_prefix("*.") {
        // 支持 *.{rs,toml} 语法
        if rest.starts_with('{') && rest.ends_with('}') {
            let exts = &rest[1..rest.len() - 1];
            return exts.split(',').any(|ext| name.ends_with(&format!(".{}", ext.trim())));
        }
        return name.ends_with(&format!(".{}", rest));
    }

    // * 前缀通配
    if let Some(suffix) = segment.strip_prefix('*') {
        return name.ends_with(suffix);
    }

    // * 后缀通配
    if let Some(prefix) = segment.strip_suffix('*') {
        return name.starts_with(prefix);
    }

    // 包含 * 的模式: 简单的前后缀匹配
    if let Some(star_pos) = segment.find('*') {
        let prefix = &segment[..star_pos];
        let suffix = &segment[star_pos + 1..];
        return name.starts_with(prefix) && name.ends_with(suffix);
    }

    // 精确匹配
    name == segment
}

/// 格式化文件大小
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
