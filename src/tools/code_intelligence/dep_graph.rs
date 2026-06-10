//! 依赖图分析 — 分析项目依赖关系
//!
//! 支持:
//! - Cargo.toml 依赖解析
//! - package.json 依赖解析
//! - 模块 use/import 关系分析
//! - 影响范围分析 (改这个文件会影响谁)

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 依赖节点
#[derive(Debug, Clone, Serialize)]
pub struct DepNode {
    /// 包名/模块名
    pub name: String,
    /// 版本 (如有)
    pub version: Option<String>,
    /// 是否为开发依赖
    pub dev_only: bool,
    /// 来源 (文件路径)
    pub source: String,
}

/// 依赖边
#[derive(Debug, Clone, Serialize)]
pub struct DepEdge {
    /// 依赖者
    pub from: String,
    /// 被依赖者
    pub to: String,
    /// 依赖类型 (runtime, dev, build)
    pub dep_type: String,
}

/// 依赖图
#[derive(Debug, Clone, Serialize)]
pub struct DepGraph {
    pub nodes: Vec<DepNode>,
    pub edges: Vec<DepEdge>,
}

/// 文件依赖关系 (模块级)
#[derive(Debug, Clone, Serialize)]
pub struct FileDepGraph {
    /// 文件路径 -> 它依赖的文件列表
    pub imports: HashMap<String, Vec<String>>,
    /// 文件路径 -> 依赖它的文件列表 (反向索引)
    pub imported_by: HashMap<String, Vec<String>>,
}

/// 分析项目依赖图
pub fn analyze_deps(root: &Path) -> DepGraph {
    let mut graph = DepGraph { nodes: Vec::new(), edges: Vec::new() };

    // Cargo.toml 依赖
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        parse_cargo_deps(&cargo_toml, &mut graph);
    }

    // package.json 依赖
    let package_json = root.join("package.json");
    if package_json.exists() {
        parse_package_json_deps(&package_json, &mut graph);
    }

    graph
}

/// 分析文件级依赖 (模块 use/import 关系)
pub fn analyze_file_deps(root: &Path, language: &str) -> FileDepGraph {
    let mut graph = FileDepGraph {
        imports: HashMap::new(),
        imported_by: HashMap::new(),
    };

    let src_dir = root.join("src");
    if !src_dir.exists() { return graph; }

    let files = collect_source_files(&src_dir, language);

    for file in &files {
        let content = std::fs::read_to_string(file).unwrap_or_default();
        let imports = extract_imports(&content, language, root, file);
        if !imports.is_empty() {
            let file_str = file.to_string_lossy().to_string();
            for imp in &imports {
                graph.imported_by
                    .entry(imp.clone())
                    .or_default()
                    .push(file_str.clone());
            }
            graph.imports.insert(file_str, imports);
        }
    }

    graph
}

/// 查找影响范围: 修改某个文件会影响哪些文件
pub fn find_impact(graph: &FileDepGraph, file: &str) -> Vec<String> {
    let mut impacted = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = vec![file.to_string()];

    while let Some(current) = queue.pop() {
        if visited.contains(&current) { continue; }
        visited.insert(current.clone());

        if let Some(dependents) = graph.imported_by.get(&current) {
            for dep in dependents {
                if !visited.contains(dep) {
                    impacted.push(dep.clone());
                    queue.push(dep.clone());
                }
            }
        }
    }

    impacted
}

// ── Cargo.toml 解析 ──────────────────────────────────────

fn parse_cargo_deps(path: &Path, graph: &mut DepGraph) {
    // 简化版: 用字符串解析而非完整 TOML 解析
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let source = path.to_string_lossy().to_string();

    // 解析 [dependencies]
    parse_dep_section(&content, "dependencies", &source, false, graph);
    // 解析 [dev-dependencies]
    parse_dep_section(&content, "dev-dependencies", &source, true, graph);
}

fn parse_dep_section(content: &str, section: &str, source: &str, dev_only: bool, graph: &mut DepGraph) {
    // 找到 section 的开始
    let section_header = format!("[{}]", section);
    let section_start = content.find(&section_header);
    if section_start.is_none() { return; }
    let section_start = section_start.unwrap() + section_header.len();

    // 找到下一个 [section] 的开始
    let section_end = content[section_start..]
        .find("\n[")
        .map(|i| section_start + i)
        .unwrap_or(content.len());

    let section_content = &content[section_start..section_end];

    // 解析每行: name = "version" 或 name = { version = "x" }
    for line in section_content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() { continue; }

        if let Some(eq_pos) = line.find('=') {
            let name = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim();

            // 提取版本
            let version = if value.starts_with('"') {
                value.trim_matches('"').to_string()
            } else if value.starts_with('{') {
                // name = { version = "x", features = [...] }
                extract_field(value, "version")
                    .unwrap_or_else(|| "*".into())
            } else {
                "*".into()
            };

            graph.nodes.push(DepNode {
                name: name.clone(),
                version: Some(version),
                dev_only,
                source: source.to_string(),
            });
        }
    }
}

// ── package.json 解析 ────────────────────────────────────

fn parse_package_json_deps(path: &Path, graph: &mut DepGraph) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let source = path.to_string_lossy().to_string();

    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else { return; };

    // dependencies
    if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
        for (name, version) in deps {
            graph.nodes.push(DepNode {
                name: name.clone(),
                version: version.as_str().map(|s| s.to_string()),
                dev_only: false,
                source: source.clone(),
            });
        }
    }

    // devDependencies
    if let Some(deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
        for (name, version) in deps {
            graph.nodes.push(DepNode {
                name: name.clone(),
                version: version.as_str().map(|s| s.to_string()),
                dev_only: true,
                source: source.clone(),
            });
        }
    }
}

// ── 导入提取 ──────────────────────────────────────────────

fn extract_imports(content: &str, language: &str, root: &Path, current_file: &Path) -> Vec<String> {
    let mut imports = Vec::new();

    match language {
        "rust" => {
            for line in content.lines() {
                let line = line.trim();
                // use crate::xxx
                if line.starts_with("use crate::") {
                    let path = line.trim_start_matches("use ")
                        .trim_end_matches(';')
                        .trim_end_matches("::{*}")
                        .replace("crate::", "src/")
                        .replace("::", "/");
                    let full = root.join(format!("{}.rs", path));
                    let mod_path = root.join(format!("{}/mod.rs", path));
                    if full.exists() {
                        imports.push(full.to_string_lossy().to_string());
                    } else if mod_path.exists() {
                        imports.push(mod_path.to_string_lossy().to_string());
                    }
                }
                // mod xxx
                if line.starts_with("mod ") && !line.contains(';') {
                    let mod_name = line.trim_start_matches("mod ")
                        .trim_end_matches(';')
                        .trim();
                    let parent = current_file.parent().unwrap_or(root);
                    let mod_file = parent.join(format!("{}.rs", mod_name));
                    let mod_dir = parent.join(format!("{}/mod.rs", mod_name));
                    if mod_file.exists() {
                        imports.push(mod_file.to_string_lossy().to_string());
                    } else if mod_dir.exists() {
                        imports.push(mod_dir.to_string_lossy().to_string());
                    }
                }
            }
        }
        "python" => {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("from .") && line.contains("import") {
                    let module = line.split_whitespace().nth(1).unwrap_or("");
                    let path = module.replace('.', "/");
                    let full = root.join(format!("src/{}.py", path));
                    let init = root.join(format!("src/{}/__init__.py", path));
                    if full.exists() {
                        imports.push(full.to_string_lossy().to_string());
                    } else if init.exists() {
                        imports.push(init.to_string_lossy().to_string());
                    }
                }
            }
        }
        "javascript" | "typescript" => {
            for line in content.lines() {
                let line = line.trim();
                // import ... from './xxx'
                if line.contains("from '") || line.contains("from \"") {
                    if let Some(start) = line.find("from '").or_else(|| line.find("from \"")) {
                        let quote = if line.contains("from '") { '\'' } else { '"' };
                        let path_start = start + 6;
                        if let Some(end) = line[path_start..].find(quote) {
                            let import_path = &line[path_start..path_start + end];
                            if import_path.starts_with('.') {
                                let parent = current_file.parent().unwrap_or(root);
                                let resolved = parent.join(import_path);
                                for ext in &[".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx", "/index.js"] {
                                    let full = PathBuf::from(format!("{}{}", resolved.display(), ext));
                                    if full.exists() {
                                        imports.push(full.to_string_lossy().to_string());
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    imports
}

// ── 辅助函数 ──────────────────────────────────────────────

fn collect_source_files(dir: &Path, language: &str) -> Vec<PathBuf> {
    let ext = match language {
        "rust" => ".rs",
        "python" => ".py",
        "javascript" => ".js",
        "typescript" => ".ts",
        _ => return Vec::new(),
    };

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name == "target" || name == "node_modules" || name == ".git" || name == "__pycache__" {
                    continue;
                }
                files.extend(collect_source_files(&path, language));
            } else if path.to_string_lossy().ends_with(ext) {
                files.push(path);
            }
        }
    }
    files
}

fn extract_field(toml_str: &str, field: &str) -> Option<String> {
    let pattern = format!("{} = ", field);
    let start = toml_str.find(&pattern)?;
    let rest = &toml_str[start + pattern.len()..];
    let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
    Some(rest[..end].trim().trim_matches('"').to_string())
}
