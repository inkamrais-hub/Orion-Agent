use async_trait::async_trait;
use serde_json::Value;

use crate::audit::{AuditEvent as GlobalAuditEvent, AUDIT_LOGGER};
use super::{Tool, ToolContext, ToolResult};

/// 精确字符串替换编辑工具
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }
    fn description(&self) -> &str {
        "Precise string replacement in a file. PREFERRED over 'write' for small changes. \
         old_string must be unique in the file (unless replace_all=true). \
         Include enough surrounding context in old_string to uniquely identify the location. \
         Use expected_replacements to verify the replacement count matches expectations."
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path" },
                "old_string": { "type": "string", "description": "Exact text to find and replace (must be unique in file)" },
                "new_string": { "type": "string", "description": "Replacement text" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences (default: false)" },
                "expected_replacements": { "type": "integer", "description": "Expected number of replacements. Error if actual count differs (optional, for safety)" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let path = input["path"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'path' field".into())
        })?;
        let old_string = input["old_string"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'old_string' field".into())
        })?;
        let new_string = input["new_string"].as_str().ok_or_else(|| {
            crate::Error::Tool("missing 'new_string' field".into())
        })?;
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);
        let expected = input["expected_replacements"].as_u64().map(|v| v as usize);

        // 工作区安全检查
        if let Err(e) = crate::core::workspace::can_write_file(std::path::Path::new(path)).await {
            return Ok(ToolResult {
                content: e,
                is_error: true,
                metadata: None,
            });
        }

        // 验证: old_string 不为空
        if old_string.is_empty() {
            return Err(crate::Error::Tool("old_string must not be empty".into()));
        }

        // 验证: old_string != new_string
        if old_string == new_string {
            return Err(crate::Error::Tool("old_string and new_string are identical, nothing to change".into()));
        }

        // 读取文件内容
        let content = tokio::fs::read_to_string(path).await
            .map_err(|e| crate::Error::Tool(format!("Cannot read '{}': {}", path, e)))?;

        // 检测原文件是否使用 CRLF，写回时还原
        let has_crlf = content.contains("\r\n");

        // 归一化 CRLF → LF 以便匹配
        let normalized = content.replace("\r\n", "\n");
        let old_normalized = old_string.replace("\r\n", "\n");

        // 计算出现次数
        let count = normalized.matches(&*old_normalized).count();

        if count == 0 {
            // ── Whitespace 建议: 尝试 trim 后匹配 ──
            let trimmed_old = old_normalized.trim();
            if !trimmed_old.is_empty() {
                let trimmed_count = normalized.matches(trimmed_old).count();
                if trimmed_count > 0 {
                    // 找到上下文来展示建议
                    let suggestion = find_whitespace_suggestion(&normalized, trimmed_old);
                    return Err(crate::Error::Tool(format!(
                        "old_string not found exactly in '{}'. \
                         However, a whitespace-trimmed version was found {} time(s). \
                         Suggestion: {}",
                        path, trimmed_count, suggestion
                    )));
                }
            }
            return Err(crate::Error::Tool(format!(
                "old_string not found in file '{}'", path
            )));
        }

        if count > 1 && !replace_all {
            return Err(crate::Error::Tool(format!(
                "old_string found {} times in '{}', not unique. \
                 Use replace_all=true or provide more context to match a single occurrence.",
                count, path
            )));
        }

        let actual_replacements = if replace_all { count } else { 1 };

        // ── expected_replacements 校验 ──
        if let Some(exp) = expected {
            if actual_replacements != exp {
                return Err(crate::Error::Tool(format!(
                    "Expected {} replacement(s) but found {} occurrence(s) of old_string in '{}'. \
                     Aborting to prevent unintended changes.",
                    exp, actual_replacements, path
                )));
            }
        }

        // 标准化 new_string 中的换行符，防止 CRLF 恢复时产生 \r\r\n
        let new_string = new_string.replace("\r\n", "\n").replace('\r', "\n");

        // 执行替换
        let new_content = if replace_all {
            normalized.replace(&*old_normalized, &new_string)
        } else {
            normalized.replacen(&*old_normalized, &new_string, 1)
        };

        // 写回前还原 CRLF
        let final_content = if has_crlf {
            new_content.replace('\n', "\r\n")
        } else {
            new_content
        };

        // 写回文件
        tokio::fs::write(path, &final_content).await
            .map_err(|e| crate::Error::Tool(format!("Cannot write '{}': {}", path, e)))?;

        // 更新文件缓存
        crate::core::cache::file_cache_set(path, final_content);

        // 审计: 文件编辑
        {
            let mut logger = AUDIT_LOGGER.lock().await;
            logger.log(GlobalAuditEvent::FileOperation {
                operation: "edit".to_string(),
                path: path.to_string(),
                bytes: old_string.len(),
            }, "tool");
        }

        // ── 生成变更预览 ──
        let preview = format!("--- old\n+++ new\n- {}\n+ {}",
            truncate_preview(old_string),
            truncate_preview(&new_string),
        );

        Ok(ToolResult {
            content: format!(
                "Successfully replaced {} occurrence(s) in {}\n\nChange preview:\n{}",
                actual_replacements, path, preview
            ),
            is_error: false,
            metadata: Some(serde_json::json!({
                "path": path,
                "replacements": actual_replacements,
                "old_preview": truncate_preview(old_string),
                "new_preview": truncate_preview(&new_string),
            })),
        })
    }
}

/// 截断预览文本 (单行最多 120 字符, 多行取前 3 行)
fn truncate_preview(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= 1 {
        let line = lines.first().unwrap_or(&"");
        if line.chars().count() > 120 {
            let truncated: String = line.chars().take(120).collect();
            format!("{}... ({} chars)", truncated, line.len())
        } else {
            line.to_string()
        }
    } else {
        let preview_lines: Vec<&str> = lines.iter().take(3).copied().collect();
        let mut result = preview_lines.join("\n  ");
        if lines.len() > 3 {
            result.push_str(&format!("... ({} lines total)", lines.len()));
        }
        result
    }
}

/// 当精确匹配失败但 trim 后能匹配时, 提供建议信息
fn find_whitespace_suggestion(content: &str, trimmed_old: &str) -> String {
    for line in content.lines() {
        if line.contains(trimmed_old) {
            let display_line = line.trim();
            if display_line.chars().count() > 100 {
                let truncated: String = display_line.chars().take(100).collect();
                return format!("try matching the exact whitespace in: \"{}...\"", truncated);
            }
            return format!("try matching the exact whitespace in: \"{}\"", display_line);
        }
    }
    "check whitespace/indentation differences between old_string and file content".to_string()
}
