use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};
use crate::index::CODE_INDEX;

pub struct ProjectMapTool;

#[async_trait]
impl Tool for ProjectMapTool {
    fn name(&self) -> &str { "project_map" }
    fn description(&self) -> &str {
        "Get a compact overview of the project: language, file count, directory layout. Use FIRST when starting a new task to understand the codebase. Result is cached for the session."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"depth":{"type":"integer","description":"Directory depth (default 2)"}}})
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let depth = input["depth"].as_u64().unwrap_or(2) as usize;
        let root = std::path::PathBuf::from(&ctx.working_dir);

        // 使用持久化索引
        {
            let mut guard = CODE_INDEX.lock().await;
            if guard.is_none() {
                *guard = Some(crate::index::CodeIndex::open(&root)?);
            }
            let idx = guard.as_mut().unwrap();
            if let Ok(report) = idx.index() {
                tracing::info!(files = report.total_files, symbols = report.total_symbols, changed = report.changed_files, ms = report.elapsed_ms, "Index updated");
            }
            if !idx.is_empty() {
                if let Ok(output) = idx.project_map(depth) {
                    return Ok(ToolResult { content: output, is_error: false, metadata: None });
                }
            }
        }

        // 回退: 全量扫描 (仅当索引为空时)
        let mut stats = Stats::default();
        let tree = build_tree(&root, &root, depth, 0, &mut stats);
        let lang = detect_language(&root);
        let output = format!(
            "Project: {} ({})
Files: {} source, {} total
Lines: {}

Directory tree:
{}",
            root.file_name().unwrap_or_default().to_string_lossy(),
            lang, stats.src_files, stats.total_files, stats.total_lines, tree
        );
        Ok(ToolResult { content: output, is_error: false, metadata: None })
    }
}

#[derive(Default)]
struct Stats { src_files: usize, total_files: usize, total_lines: usize }

fn detect_language(root: &std::path::Path) -> &str {
    if root.join("Cargo.toml").exists() { "Rust" }
    else if root.join("package.json").exists() { "JS/TS" }
    else if root.join("go.mod").exists() { "Go" }
    else if root.join("pyproject.toml").exists()||root.join("setup.py").exists() { "Python" }
    else { "Unknown" }
}

fn build_tree(_root:&std::path::Path,dir:&std::path::Path,max_d:usize,d:usize,stats:&mut Stats)->String{
    if d>=max_d{return String::new();}
    let mut result=String::new();
    let Ok(entries)=std::fs::read_dir(dir)else{return result;};
    let mut dirs=Vec::new();let mut files=Vec::new();
    for e in entries.flatten(){
        let p=e.path();let n=e.file_name().to_string_lossy().to_string();
        if n.starts_with('.')||n=="target"||n=="node_modules"||n==".git"{continue;}
        if p.is_dir(){dirs.push((n,p));}else{files.push(n);stats.total_files+=1;
            if matches!(p.extension().and_then(|e|e.to_str()),Some("rs"|"py"|"ts"|"js"|"go")){
                stats.src_files+=1;
                if let Ok(c)=std::fs::read_to_string(&p){stats.total_lines+=c.lines().count();}
            }
        }
    }
    let indent = "  ".repeat(d);
    for (n, p) in dirs {
        result.push_str(&format!("{}{}/\n", indent, n));
        result.push_str(&build_tree(_root, &p, max_d, d + 1, stats));
    }
    for n in files {
        result.push_str(&format!("{}{}\n", indent, n));
    }
    result
}