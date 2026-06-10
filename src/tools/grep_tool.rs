use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};

/// 单文件读取上限: 10MB，防止 OOM
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

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

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str {
        "Search file contents using regex patterns. Returns matching lines with file paths and line numbers. \
         Use for: finding function definitions, variable usages, error messages, specific code patterns. \
         Supports context_lines (show surrounding code), glob filter (limit to file types), ignore_case. \
         For finding files by name, use 'glob' instead."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regex pattern to search for"},
                "path": {"type": "string", "description": "Directory or file to search in (default: CWD)"},
                "glob": {"type": "string", "description": "File glob filter (e.g. '*.rs', '*.py')"},
                "max_results": {"type": "integer", "description": "Max results to return (default: 30)"},
                "context_lines": {"type": "integer", "description": "Number of lines to show before/after each match, like grep -C (default: 0)"},
                "ignore_case": {"type": "boolean", "description": "Case-insensitive search (default: false)"},
                "count_only": {"type": "boolean", "description": "Only return match counts per file, not the actual lines (default: false)"}
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let pattern = input["pattern"].as_str().ok_or_else(|| crate::Error::Tool("missing pattern".into()))?;
        let root = input["path"].as_str().unwrap_or(&ctx.working_dir);
        let glob_filter = input["glob"].as_str();

        // 路径安全检查: 禁止访问系统目录
        if is_blocked_path(root) {
            return Ok(ToolResult {
                content: format!("安全拦截: 禁止访问系统目录 '{}'", root),
                is_error: true,
                metadata: None,
            });
        }
        let max_results = input["max_results"].as_u64().unwrap_or(30) as usize;
        let context_lines = input["context_lines"].as_u64().unwrap_or(0) as usize;
        let ignore_case = input["ignore_case"].as_bool().unwrap_or(false);
        let count_only = input["count_only"].as_bool().unwrap_or(false);

        // 构建 regex (可选忽略大小写)
        let re = if ignore_case {
            regex::RegexBuilder::new(pattern).case_insensitive(true).build()
        } else {
            regex::Regex::new(pattern)
        }.map_err(|e| crate::Error::Tool(format!("Invalid regex '{}': {}", pattern, e)))?;

        let mut file_results: Vec<FileSearchResult> = Vec::new();
        let mut running_total = 0usize;
        search_dir(std::path::Path::new(root), &re, glob_filter, &mut file_results, max_results, context_lines, count_only, 0, &mut running_total);

        if file_results.is_empty() {
            return Ok(ToolResult {
                content: format!("No matches for '{}' found in {}", pattern, root),
                is_error: false,
                metadata: None,
            });
        }

        if count_only {
            // ── 仅返回计数 ──
            let lines: Vec<String> = file_results.iter()
                .map(|fr| {
                    let rel = fr.path.strip_prefix(root).unwrap_or(&fr.path);
                    format!("{}: {} match(es)", rel.display(), fr.matches.len())
                })
                .collect();
            let total: usize = file_results.iter().map(|fr| fr.matches.len()).sum();
            return Ok(ToolResult {
                content: format!("{}\n\nTotal: {} match(es) in {} file(s)", lines.join("\n"), total, file_results.len()),
                is_error: false,
                metadata: Some(json!({"total_matches": total, "files": file_results.len()})),
            });
        }

        // ── 带上下文的输出 ──
        let mut output_lines: Vec<String> = Vec::new();
        let mut match_count = 0usize;

        for fr in &file_results {
            let rel = fr.path.strip_prefix(root).unwrap_or(&fr.path);
            let path_display = rel.display().to_string();

            if context_lines > 0 {
                // 带上下文输出
                for mi in &fr.matches {
                    match_count += 1;
                    if match_count > max_results { break; }
                    output_lines.push(format!("--- {}:{}", path_display, mi.line_num));
                    for ctx_line in &mi.context_before {
                        output_lines.push(format!("  {}: {}", ctx_line.line_num, ctx_line.text.trim()));
                    }
                    output_lines.push(format!("> {}: {}", mi.line_num, mi.matched_line.trim()));
                    for ctx_line in &mi.context_after {
                        output_lines.push(format!("  {}: {}", ctx_line.line_num, ctx_line.text.trim()));
                    }
                }
            } else {
                // 无上下文: 简洁输出
                for mi in &fr.matches {
                    match_count += 1;
                    if match_count > max_results { break; }
                    output_lines.push(format!("{}:{}: {}", path_display, mi.line_num, mi.matched_line.trim()));
                }
            }
            if match_count >= max_results { break; }
        }

        Ok(ToolResult {
            content: output_lines.join("\n"),
            is_error: false,
            metadata: Some(json!({"matches": match_count, "files": file_results.len()})),
        })
    }
}

/// 单个文件的搜索结果
struct FileSearchResult {
    path: std::path::PathBuf,
    matches: Vec<LineMatch>,
}

/// 单行匹配结果 (含上下文)
struct LineMatch {
    line_num: usize,
    matched_line: String,
    context_before: Vec<ContextLine>,
    context_after: Vec<ContextLine>,
}

struct ContextLine {
    line_num: usize,
    text: String,
}

fn search_dir(
    dir: &std::path::Path,
    re: &regex::Regex,
    glob_filter: Option<&str>,
    results: &mut Vec<FileSearchResult>,
    max: usize,
    context_lines: usize,
    count_only: bool,
    depth: usize,
    running_total: &mut usize,
) {
    if depth > 10 || *running_total >= max { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *running_total >= max { return; }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            search_dir(&path, re, glob_filter, results, max, context_lines, count_only, depth + 1, running_total);
        } else {
            // Apply glob filter
            if let Some(glob) = glob_filter {
                if !matches_glob_simple(&name, glob) { continue; }
            }
            // Skip binary files
            if is_binary(&path) { continue; }
            // 跳过过大文件，防止 OOM
            if let Ok(metadata) = std::fs::metadata(&path) {
                if metadata.len() > MAX_FILE_SIZE { continue; }
            }
            // Search file contents
            if let Ok(content) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = content.lines().collect();
                let mut file_matches = Vec::new();

                for (i, line) in lines.iter().enumerate() {
                    if *running_total + file_matches.len() >= max { break; }
                    if re.is_match(line) {
                        if count_only {
                            // count_only 模式: 只记录行号和内容, 不需要上下文
                            file_matches.push(LineMatch {
                                line_num: i + 1,
                                matched_line: line.to_string(),
                                context_before: Vec::new(),
                                context_after: Vec::new(),
                            });
                        } else {
                            // 收集上下文
                            let ctx_before = if context_lines > 0 {
                                let start = i.saturating_sub(context_lines);
                                (start..i).map(|j| ContextLine {
                                    line_num: j + 1,
                                    text: lines[j].to_string(),
                                }).collect()
                            } else {
                                Vec::new()
                            };
                            let ctx_after = if context_lines > 0 {
                                let end = (i + 1 + context_lines).min(lines.len());
                                ((i + 1)..end).map(|j| ContextLine {
                                    line_num: j + 1,
                                    text: lines[j].to_string(),
                                }).collect()
                            } else {
                                Vec::new()
                            };
                            file_matches.push(LineMatch {
                                line_num: i + 1,
                                matched_line: line.to_string(),
                                context_before: ctx_before,
                                context_after: ctx_after,
                            });
                        }
                    }
                }

                if !file_matches.is_empty() {
                    *running_total += file_matches.len();
                    results.push(FileSearchResult {
                        path: path.clone(),
                        matches: file_matches,
                    });
                }
            }
        }
    }
}

fn matches_glob_simple(name: &str, glob: &str) -> bool {
    if glob == "*" { return true; }
    if let Some(ext) = glob.strip_prefix("*.") {
        return name.ends_with(&format!(".{}", ext));
    }
    name == glob
}

fn is_binary(path: &std::path::Path) -> bool {
    // Check by extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(ext, "exe" | "dll" | "so" | "dylib" | "bin" | "o" | "obj" | "pdb" | "pdf" | "png" | "jpg" | "gif" | "zip" | "tar" | "gz")
    } else {
        false
    }
}
