use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};
use crate::index::CODE_INDEX;

pub struct SymbolSearchTool;

#[async_trait]
impl Tool for SymbolSearchTool {
    fn name(&self) -> &str { "symbol_search" }
    fn description(&self) -> &str {
        "Search for code symbols (functions, structs, traits, modules) by name. Returns file path, line number, and signature. Use when: find where X is defined. Do NOT use for: text search (use bash grep)."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Symbol name"},
                "kind": {"type": "string", "description": "function|struct|trait|enum|module"},
                "limit": {"type": "integer", "description": "Max results (default 10)"}
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let query = input["query"].as_str().ok_or_else(|| crate::Error::Tool("missing query".into()))?;
        let kind_filter = input["kind"].as_str();
        let limit = input["limit"].as_u64().unwrap_or(10).min(30) as usize;
        let root = std::path::PathBuf::from(&ctx.working_dir);

        // 使用持久化索引 (首次 open, 后续复用)
        {
            let mut guard = CODE_INDEX.lock().await;
            if guard.is_none() {
                *guard = Some(crate::index::CodeIndex::open(&root)?);
            }
            let idx = guard.as_mut().unwrap();
            let _ = idx.index(); // 增量更新 (mtime/hash 变化才处理)
            if !idx.is_empty() {
                let results = idx.search_symbols(query, kind_filter, limit).unwrap_or_default();
                return Ok(ToolResult { content: serde_json::to_string_pretty(&results).unwrap_or_default(), is_error: false, metadata: None });
            }
        }

        // 回退: 全量扫描 (仅当索引为空时)
        let mut results = Vec::new();
        scan_dir(&root, &root, query, kind_filter, &mut results, limit * 3);
        results.truncate(limit);
        if results.is_empty() {
            Ok(ToolResult { content: format!("No symbols matching '{}' found", query), is_error: false, metadata: None })
        } else {
            Ok(ToolResult { content: serde_json::to_string_pretty(&results).unwrap_or_default(), is_error: false, metadata: None })
        }
    }
}

#[derive(serde::Serialize)]
struct Sym { name: String, kind: String, file: String, line: usize, signature: String, score: f64 }

fn scan_dir(root: &std::path::Path, dir: &std::path::Path, q: &str, kf: Option<&str>, res: &mut Vec<Sym>, max: usize) {
    if res.len() >= max { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if res.len() >= max { return; }
        let p = e.path();
        let n = e.file_name().to_string_lossy().to_string();
        if n.starts_with('.') || n=="target" || n=="node_modules" || n==".git" { continue; }
        if p.is_dir() { scan_dir(root, &p, q, kf, res, max); }
        else if is_src(&p) { scan_file(root, &p, q, kf, res); }
    }
}
fn is_src(p: &std::path::Path) -> bool { matches!(p.extension().and_then(|e|e.to_str()), Some("rs"|"py"|"ts"|"js"|"go")) }

fn scan_file(root: &std::path::Path, path: &std::path::Path, q: &str, kf: Option<&str>, res: &mut Vec<Sym>) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string();
    for (i, line) in content.lines().enumerate() {
        let t = line.trim();
        let ln = i+1;
        for &(kw, kind) in &[("fn ","function"),("pub fn ","function"),("async fn ","function"),("pub async fn ","function"),("struct ","struct"),("pub struct ","struct"),("trait ","trait"),("pub trait ","trait"),("enum ","enum"),("pub enum ","enum"),("mod ","module"),("pub mod ","module")] {
            if let Some(name) = extract(t, kw) {
                if kf.is_none_or(|f| f==kind) && !name.is_empty() {
                    let sc = score(&name, q);
                    if sc > 0.0 { res.push(Sym{name,kind:kind.into(),file:format!("{}:{}",rel,ln),line:ln,signature:t.to_string(),score:sc}); }
                }
            }
        }
    }
    res.sort_by(|a,b| b.score.partial_cmp(&a.score).unwrap());
}
fn extract(line: &str, kw: &str) -> Option<String> {
    let pos = line.find(kw)?;
    let rest = &line[pos+kw.len()..];
    let end = rest.find(|c:char| !c.is_alphanumeric() && c!='_').unwrap_or(rest.len());
    let n = rest[..end].to_string();
    if n.is_empty() || !n.chars().next().unwrap().is_alphabetic() { None } else { Some(n) }
}
fn score(name: &str, q: &str) -> f64 {
    let (nl,ql)=(name.to_lowercase(),q.to_lowercase());
    if nl==ql {1.0} else if nl.starts_with(&ql){0.8} else if nl.contains(&ql){0.5} else {0.0}
}