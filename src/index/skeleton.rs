//! 代码骨架提取器
//!
//! 从源码文件中提取函数/结构体/枚举等定义的签名，折叠函数体。
//! 输出精简的骨架视图，Token 消耗仅为原始文件的 10-20%。

use std::path::Path;

/// 骨架条目
#[derive(Debug, Clone)]
pub struct SkeletonEntry {
    /// 类型: fn, struct, enum, impl, trait, mod, const, static, type
    pub kind: String,
    /// 名称
    pub name: String,
    /// 签名行（含参数、返回类型等）
    pub signature: String,
    /// 起始行号
    pub start_line: usize,
    /// 结束行号（函数体结束）
    pub end_line: usize,
    /// 是否被折叠（函数体被省略）
    pub collapsed: bool,
}

/// 从文件提取骨架
pub fn extract_skeleton(content: &str, file_path: &Path) -> Vec<SkeletonEntry> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        "rs" => extract_rust_skeleton(content),
        "py" => extract_python_skeleton(content),
        "js" | "ts" | "jsx" | "tsx" => extract_js_skeleton(content),
        "go" => extract_go_skeleton(content),
        _ => extract_generic_skeleton(content),
    }
}

/// 格式化骨架为可读文本
pub fn format_skeleton(entries: &[SkeletonEntry], file_path: &Path) -> String {
    let mut output = format!("// Skeleton: {}\n", file_path.display());
    output.push_str(&format!("// {} definitions\n\n", entries.len()));

    for entry in entries {
        if entry.collapsed {
            output.push_str(&format!(
                "{} // L{}-L{} ({} lines)\n",
                entry.signature,
                entry.start_line,
                entry.end_line,
                entry.end_line - entry.start_line + 1
            ));
        } else {
            output.push_str(&format!("{}\n", entry.signature));
        }
    }

    output
}

/// Rust 骨架提取
fn extract_rust_skeleton(content: &str) -> Vec<SkeletonEntry> {
    let mut entries = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // 跳过注释和空行
        if line.is_empty() || line.starts_with("//") || line.starts_with("/*") {
            i += 1;
            continue;
        }

        // 函数定义
        if let Some((entry, next_i)) = try_extract_fn(&lines, i) {
            entries.push(entry);
            i = next_i;
            continue;
        }

        // impl 块（要在 struct 之前，因为 impl 可以含 struct 关键字）
        if line.starts_with("impl ") || line.starts_with("pub impl ") {
            if let Some((entry, next_i)) = try_extract_impl(&lines, i) {
                entries.push(entry);
                i = next_i;
                continue;
            }
        }

        // 结构体定义
        if line.contains("struct ") {
            if let Some((entry, next_i)) = try_extract_struct(&lines, i) {
                entries.push(entry);
                i = next_i;
                continue;
            }
        }

        // 枚举定义
        if line.contains("enum ") {
            if let Some((entry, next_i)) = try_extract_enum(&lines, i) {
                entries.push(entry);
                i = next_i;
                continue;
            }
        }

        // trait 定义
        if line.contains("trait ") {
            if let Some((entry, next_i)) = try_extract_trait(&lines, i) {
                entries.push(entry);
                i = next_i;
                continue;
            }
        }

        // mod 声明
        if line.starts_with("pub mod ") || line.starts_with("mod ") {
            let kind = "mod";
            let name = extract_name_from_after(line, "mod");
            entries.push(SkeletonEntry {
                kind: kind.into(),
                name,
                signature: line.trim_end_matches('{').trim().to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: false,
            });
            i += 1;
            continue;
        }

        // const/static/type
        if line.starts_with("pub const ")
            || line.starts_with("const ")
            || line.starts_with("pub static ")
            || line.starts_with("static ")
            || line.starts_with("pub type ")
            || line.starts_with("type ")
        {
            let kind = if line.contains("const") {
                "const"
            } else if line.contains("static") {
                "static"
            } else {
                "type"
            };
            entries.push(SkeletonEntry {
                kind: kind.into(),
                name: extract_name_from_line(line, kind),
                signature: line.trim_end_matches(';').trim().to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: false,
            });
            i += 1;
            continue;
        }

        // macro_rules!
        if line.starts_with("macro_rules!") || line.starts_with("pub macro_rules!") {
            let name = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("?")
                .trim_end_matches('(')
                .to_string();
            let end = find_brace_end(&lines, i);
            let sig = format!("macro_rules! {} {{ ... }}", name);
            entries.push(SkeletonEntry {
                kind: "macro".into(),
                name,
                signature: sig,
                start_line: i + 1,
                end_line: end + 1,
                collapsed: true,
            });
            i = end + 1;
            continue;
        }

        i += 1;
    }

    entries
}

fn try_extract_fn(lines: &[&str], start: usize) -> Option<(SkeletonEntry, usize)> {
    let line = lines[start].trim();

    // 匹配: pub async fn name(...) -> ... { 或 fn name(...) {
    if !line.contains("fn ") {
        return None;
    }
    if line.starts_with("//") {
        return None;
    }
    // 排除非函数定义行 (e.g. inside a string or comment)
    // 确保 "fn " 是关键字而非字符串内
    let fn_pos = line.find("fn ")?;
    // 检查 fn 之前的部分是否看起来像函数声明（可含 pub/async/unsafe/extern 等）
    let before_fn = &line[..fn_pos];
    let trimmed_before = before_fn.trim();
    // 如果 fn 之前有引号，说明可能是字符串，跳过
    if trimmed_before.contains('"') || trimmed_before.contains('\'') {
        return None;
    }
    // before_fn 应该只包含 pub/async/unsafe/extern/"C" 等关键字
    for token in trimmed_before.split_whitespace() {
        let t = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if !t.is_empty()
            && t != "pub"
            && t != "async"
            && t != "unsafe"
            && t != "extern"
            && t != "const"
            && t != "fn"
        {
            // 可能是返回类型如 "fn foo(...) -> Result<" 中的 "fn" 不应到这里
            // 但也可能有 vis qualifiers like pub(crate)
            if !t.starts_with("pub") && !t.starts_with("crate") {
                return None;
            }
        }
    }

    // 收集多行签名（函数签名可能跨多行）
    let mut sig = String::new();
    let mut brace_count = 0i32;
    let mut found_open_brace = false;
    let mut i = start;

    while i < lines.len() {
        let current = lines[i];
        sig.push_str(current.trim());
        sig.push(' ');

        for ch in current.chars() {
            match ch {
                '{' => {
                    brace_count += 1;
                    found_open_brace = true;
                }
                '}' => {
                    brace_count -= 1;
                }
                _ => {}
            }
        }

        if found_open_brace && brace_count == 0 {
            break;
        }

        if !found_open_brace && sig.len() > 500 {
            // 签名太长，可能解析出错，放弃
            return None;
        }

        i += 1;
    }

    // 提取函数名
    let name = if let Some(fn_pos) = sig.find("fn ") {
        let after_fn = &sig[fn_pos + 3..];
        after_fn
            .split('(')
            .next()
            .unwrap_or("?")
            .trim()
            .to_string()
    } else {
        "?".to_string()
    };

    // 构造精简签名（去掉函数体）
    let sig_only = if let Some(brace_pos) = sig.find('{') {
        format!("{}{{ ... }}", sig[..brace_pos].trim())
    } else {
        sig.trim().to_string()
    };

    Some((
        SkeletonEntry {
            kind: "fn".into(),
            name,
            signature: sig_only,
            start_line: start + 1,
            end_line: i + 1,
            collapsed: true,
        },
        i + 1,
    ))
}

fn try_extract_struct(lines: &[&str], start: usize) -> Option<(SkeletonEntry, usize)> {
    let line = lines[start].trim();
    if !line.contains("struct ") || line.starts_with("//") {
        return None;
    }

    let name = extract_name_after_keyword(line, "struct");
    let end = find_brace_end(lines, start);

    // 结构体签名：只保留字段名，不保留完整定义
    let sig = if end > start {
        let fields: Vec<String> = lines[start + 1..end]
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .map(|l| {
                if let Some(colon_pos) = l.find(':') {
                    format!("  {},", l[..colon_pos].trim())
                } else {
                    format!("  {},", l.trim_end_matches(',').trim())
                }
            })
            .collect();
        if fields.is_empty() {
            format!("struct {} {{ }}", name)
        } else {
            format!("struct {} {{\n{}\n}}", name, fields.join("\n"))
        }
    } else {
        // 可能是 unit struct 或 tuple struct
        format!(
            "struct {}",
            line.trim_end_matches(';').trim_end_matches('{').trim()
        )
    };

    Some((
        SkeletonEntry {
            kind: "struct".into(),
            name,
            signature: sig,
            start_line: start + 1,
            end_line: end + 1,
            collapsed: false,
        },
        end + 1,
    ))
}

fn try_extract_enum(lines: &[&str], start: usize) -> Option<(SkeletonEntry, usize)> {
    let line = lines[start].trim();
    if !line.contains("enum ") || line.starts_with("//") {
        return None;
    }

    let name = extract_name_after_keyword(line, "enum");
    let end = find_brace_end(lines, start);

    let variants: Vec<String> = lines[start + 1..end]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| format!("  {},", l.trim_end_matches(',').trim()))
        .collect();

    let sig = if variants.is_empty() {
        "enum { }".to_string()
    } else {
        format!("enum {} {{\n{}\n}}", name, variants.join("\n"))
    };

    Some((
        SkeletonEntry {
            kind: "enum".into(),
            name,
            signature: sig,
            start_line: start + 1,
            end_line: end + 1,
            collapsed: false,
        },
        end + 1,
    ))
}

fn try_extract_impl(lines: &[&str], start: usize) -> Option<(SkeletonEntry, usize)> {
    let line = lines[start].trim();
    if !line.starts_with("impl ") || line.starts_with("//") {
        return None;
    }

    let end = find_brace_end(lines, start);
    let name = line.trim_end_matches('{').trim().to_string();

    // 提取 impl 块内的方法签名
    let mut methods = Vec::new();
    let mut j = start + 1;
    while j < end {
        let inner = lines[j].trim();
        if inner.contains("fn ") && !inner.starts_with("//") {
            if let Some((entry, next_j)) = try_extract_fn(lines, j) {
                methods.push(format!("  {}", entry.signature));
                j = next_j;
                continue;
            }
        }
        j += 1;
    }

    let sig = if methods.is_empty() {
        format!("{} {{ /* empty */ }}", name)
    } else {
        format!("{} {{\n{}\n}}", name, methods.join("\n"))
    };

    Some((
        SkeletonEntry {
            kind: "impl".into(),
            name: name.trim_start_matches("impl ").to_string(),
            signature: sig,
            start_line: start + 1,
            end_line: end + 1,
            collapsed: false,
        },
        end + 1,
    ))
}

fn try_extract_trait(lines: &[&str], start: usize) -> Option<(SkeletonEntry, usize)> {
    let line = lines[start].trim();
    if !line.contains("trait ") || line.starts_with("//") {
        return None;
    }

    let name = extract_name_after_keyword(line, "trait");
    let end = find_brace_end(lines, start);

    // 提取 trait 内的方法签名
    let mut methods = Vec::new();
    let mut j = start + 1;
    while j < end {
        let inner = lines[j].trim();
        if inner.contains("fn ") && !inner.starts_with("//") {
            if let Some((entry, next_j)) = try_extract_fn(lines, j) {
                methods.push(format!("  {}", entry.signature));
                j = next_j;
                continue;
            }
        }
        j += 1;
    }

    let sig = if methods.is_empty() {
        format!("trait {} {{ ... }}", name)
    } else {
        format!("trait {} {{\n{}\n}}", name, methods.join("\n"))
    };

    Some((
        SkeletonEntry {
            kind: "trait".into(),
            name,
            signature: sig,
            start_line: start + 1,
            end_line: end + 1,
            collapsed: true,
        },
        end + 1,
    ))
}

// ── 辅助函数 ──

fn find_brace_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    let mut found = false;
    #[allow(clippy::needless_range_loop)] // Index i tracks position for brace depth counting
    for i in start..lines.len() {
        for ch in lines[i].chars() {
            match ch {
                '{' => {
                    depth += 1;
                    found = true;
                }
                '}' => {
                    depth -= 1;
                }
                _ => {}
            }
        }
        if found && depth == 0 {
            return i;
        }
    }
    lines.len().saturating_sub(1)
}

fn extract_name_after_keyword(line: &str, keyword: &str) -> String {
    if let Some(pos) = line.find(keyword) {
        let after = &line[pos + keyword.len()..];
        let trimmed = after.trim_start();
        trimmed
            .split(['{', ':', '<', ' ', '\t', '('])
            .next()
            .unwrap_or("?")
            .to_string()
    } else {
        "?".to_string()
    }
}

fn extract_name_from_after(line: &str, keyword: &str) -> String {
    if let Some(pos) = line.find(keyword) {
        let after = &line[pos + keyword.len()..];
        after
            .trim()
            .split(['{', ';', ' ', '\t'])
            .next()
            .unwrap_or("?")
            .to_string()
    } else {
        "?".to_string()
    }
}

fn extract_name_from_line(line: &str, keyword: &str) -> String {
    if let Some(pos) = line.find(keyword) {
        let after = &line[pos + keyword.len()..];
        after
            .trim()
            .split([':', '=', '{', ' ', '\t'])
            .next()
            .unwrap_or("?")
            .to_string()
    } else {
        "?".to_string()
    }
}

// ── 非 Rust 语言的简化骨架提取 ──

fn extract_python_skeleton(content: &str) -> Vec<SkeletonEntry> {
    let mut entries = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            let name = trimmed
                .split('(')
                .next()
                .unwrap_or("?")
                .trim_start_matches("def ")
                .trim_start_matches("async def ")
                .trim();
            entries.push(SkeletonEntry {
                kind: "fn".into(),
                name: name.to_string(),
                signature: format!("{}: ...", trimmed.trim_end_matches(':')),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        } else if trimmed.starts_with("class ") {
            let name = trimmed
                .split(['(', ':', '{'])
                .next()
                .unwrap_or("?")
                .trim_start_matches("class ")
                .trim();
            entries.push(SkeletonEntry {
                kind: "class".into(),
                name: name.to_string(),
                signature: trimmed.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        }
    }
    entries
}

fn extract_js_skeleton(content: &str) -> Vec<SkeletonEntry> {
    let mut entries = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("function ")
            || trimmed.starts_with("async function ")
            || trimmed.contains("=> {")
            || trimmed.contains("=>  ")
        {
            let name =
                if trimmed.starts_with("function") || trimmed.starts_with("async function") {
                    trimmed
                        .split('(')
                        .next()
                        .unwrap_or("?")
                        .trim_start_matches("function ")
                        .trim_start_matches("async function ")
                        .trim()
                        .to_string()
                } else {
                    format!("L{}_arrow", i + 1)
                };
            entries.push(SkeletonEntry {
                kind: "fn".into(),
                name,
                signature: trimmed.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        } else if trimmed.starts_with("class ") || trimmed.starts_with("export class ") {
            let name = trimmed
                .split(['{', '(', ' '])
                .nth(1)
                .unwrap_or("?")
                .to_string();
            entries.push(SkeletonEntry {
                kind: "class".into(),
                name,
                signature: trimmed.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        }
    }
    entries
}

fn extract_go_skeleton(content: &str) -> Vec<SkeletonEntry> {
    let mut entries = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("func ") {
            let sig = trimmed.trim_end_matches('{').trim();
            let name = sig
                .split('(')
                .next()
                .unwrap_or("?")
                .trim_start_matches("func ")
                .trim()
                .to_string();
            entries.push(SkeletonEntry {
                kind: "fn".into(),
                name,
                signature: sig.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        } else if trimmed.starts_with("type ") && trimmed.contains("struct") {
            let name = trimmed.split_whitespace().nth(1).unwrap_or("?").to_string();
            entries.push(SkeletonEntry {
                kind: "struct".into(),
                name,
                signature: trimmed.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        }
    }
    entries
}

fn extract_generic_skeleton(content: &str) -> Vec<SkeletonEntry> {
    // 通用：提取看起来像函数定义的行
    let mut entries = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if (trimmed.contains("function ") || trimmed.contains("def ") || trimmed.starts_with("fn "))
            && trimmed.contains('(')
        {
            entries.push(SkeletonEntry {
                kind: "fn".into(),
                name: format!("L{}", i + 1),
                signature: trimmed.to_string(),
                start_line: i + 1,
                end_line: i + 1,
                collapsed: true,
            });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_rust_skeleton() {
        let code = r#"
pub fn hello(name: &str) -> String {
    format!("Hello, {}!", name)
}

pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), value: 0 }
    }
}
"#;
        let entries = extract_rust_skeleton(code);
        assert!(entries.len() >= 3, "Expected >=3 entries, got {}", entries.len());
        assert!(
            entries.iter().any(|e| e.name == "hello" && e.kind == "fn"),
            "Missing hello fn"
        );
        assert!(
            entries.iter().any(|e| e.name == "Config" && e.kind == "struct"),
            "Missing Config struct"
        );
    }

    #[test]
    fn test_rust_enum() {
        let code = r#"
pub enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let entries = extract_rust_skeleton(code);
        assert!(entries.len() >= 1);
        assert!(entries.iter().any(|e| e.name == "Color" && e.kind == "enum"));
    }

    #[test]
    fn test_rust_trait() {
        let code = r#"
pub trait Drawable {
    fn draw(&self);
    fn size(&self) -> (u32, u32);
}
"#;
        let entries = extract_rust_skeleton(code);
        assert!(entries.iter().any(|e| e.name == "Drawable" && e.kind == "trait"));
    }

    #[test]
    fn test_rust_const() {
        let code = r#"
pub const MAX_SIZE: usize = 1024;
const DEFAULT_NAME: &str = "test";
"#;
        let entries = extract_rust_skeleton(code);
        assert!(entries.len() >= 2);
        assert!(entries.iter().any(|e| e.name == "MAX_SIZE" && e.kind == "const"));
    }

    #[test]
    fn test_python_skeleton() {
        let code = r#"
def hello(name):
    print(f"Hello {name}")

class Foo:
    pass
"#;
        let entries = extract_python_skeleton(code);
        assert!(entries.len() >= 2);
        assert!(entries.iter().any(|e| e.name == "hello" && e.kind == "fn"));
        assert!(entries.iter().any(|e| e.name == "Foo" && e.kind == "class"));
    }

    #[test]
    fn test_go_skeleton() {
        let code = r#"
func main() {
    fmt.Println("hello")
}

type Config struct {
    Name string
}
"#;
        let entries = extract_go_skeleton(code);
        assert!(entries.len() >= 2);
        assert!(entries.iter().any(|e| e.name == "main" && e.kind == "fn"));
    }

    #[test]
    fn test_js_skeleton() {
        let code = r#"
function hello(name) {
    console.log(name)
}

class Foo {
    constructor() {}
}
"#;
        let entries = extract_js_skeleton(code);
        assert!(entries.len() >= 2);
    }

    #[test]
    fn test_format_skeleton() {
        let code = r#"pub fn add(a: i32, b: i32) -> i32 { a + b }"#;
        let path = PathBuf::from("test.rs");
        let entries = extract_rust_skeleton(code);
        let formatted = format_skeleton(&entries, &path);
        assert!(formatted.contains("Skeleton: test.rs"));
        assert!(formatted.contains("definitions"));
    }

    #[test]
    fn test_multiline_fn_signature() {
        let code = r#"
pub fn very_long_function_name(
    param1: String,
    param2: Vec<i32>,
    param3: Option<bool>,
) -> Result<String, Error> {
    Ok("hello".to_string())
}
"#;
        let entries = extract_rust_skeleton(code);
        assert!(entries.len() >= 1);
        let e = &entries[0];
        assert_eq!(e.name, "very_long_function_name");
        assert_eq!(e.kind, "fn");
        assert!(e.collapsed);
    }

    #[test]
    fn test_extract_skeleton_dispatches_by_extension() {
        let content = "fn main() {}";
        let rs_path = PathBuf::from("main.rs");
        let py_path = PathBuf::from("main.py");
        let go_path = PathBuf::from("main.go");

        let rs_entries = extract_skeleton(content, &rs_path);
        let py_entries = extract_skeleton(content, &py_path);
        let go_entries = extract_skeleton(content, &go_path);

        // Rust parses fn
        assert!(rs_entries.iter().any(|e| e.kind == "fn"));
        // Python doesn't recognize Rust syntax
        assert!(py_entries.is_empty());
        // Go doesn't recognize Rust syntax
        assert!(go_entries.is_empty());
    }
}
