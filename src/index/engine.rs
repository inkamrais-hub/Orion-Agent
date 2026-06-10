use std::path::{Path, PathBuf};
use std::collections::HashMap;

pub struct CodeIndex { data: IndexData, root: PathBuf, db_path: PathBuf, symbol_index: HashMap<String, Vec<usize>>, caller_index: HashMap<String, Vec<usize>> }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct IndexData { files: HashMap<String, FileEntry>, symbols: Vec<SymbolEntry>, callers: Vec<CallerEntry> }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FileEntry { language: String, hash: String, lines: usize, #[serde(default)] mtime: u64 }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SymbolEntry { file: String, name: String, kind: String, signature: String, line: usize }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CallerEntry { callee: String, file: String, line: usize, code: String, caller_func: String }

#[derive(Debug)]
pub struct IndexReport { pub total_files: usize, pub changed_files: usize, pub total_symbols: usize, pub elapsed_ms: u64 }

#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolResult { pub name: String, pub kind: String, pub file: String, pub line: usize, pub signature: String }

#[derive(Debug, Clone, serde::Serialize)]
pub struct CallerResult { pub file: String, pub line: usize, pub code: String, pub caller_func: String }

impl CodeIndex {
    pub fn open(root: &Path) -> crate::Result<Self> {
        let orion_dir = root.join(".orion");
        std::fs::create_dir_all(&orion_dir)?;
        let db_path = orion_dir.join("index.json");
        let data = if db_path.exists() {
            let content = std::fs::read_to_string(&db_path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else { IndexData::default() };
        let mut idx = Self { data, root: root.to_path_buf(), db_path, symbol_index: HashMap::new(), caller_index: HashMap::new() };
        idx.rebuild_indexes();
        Ok(idx)
    }

    pub fn index(&mut self) -> crate::Result<IndexReport> {
        let start = std::time::Instant::now();
        let files = discover_source_files(&self.root);
        let mut changed = 0usize;
        for file_path in &files {
            let rel = file_path.strip_prefix(&self.root).unwrap_or(file_path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let meta = std::fs::metadata(file_path).ok();
            let file_mtime = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(entry) = self.data.files.get(&rel_str) {
                if entry.mtime == file_mtime { continue; }
                let current_hash = hash_file(file_path);
                if entry.hash == current_hash {
                    self.data.files.get_mut(&rel_str).unwrap().mtime = file_mtime;
                    continue;
                }
            }
            let content = std::fs::read_to_string(file_path).unwrap_or_default();
            let lang = detect_lang(file_path);
            let line_count = content.lines().count();
            let current_hash = hash_file(file_path);
            self.data.files.insert(rel_str.clone(), FileEntry { language: lang.to_string(), hash: current_hash, lines: line_count, mtime: file_mtime });
            self.data.symbols.retain(|s| s.file != rel_str);
            self.data.callers.retain(|c| c.file != rel_str);
            for sym in parse_symbols(&content, lang) {
                self.data.symbols.push(SymbolEntry { file: rel_str.clone(), name: sym.name, kind: sym.kind, signature: sym.signature, line: sym.line });
            }
            for call in parse_callers(&content) {
                self.data.callers.push(CallerEntry { callee: call.callee, file: rel_str.clone(), line: call.line, code: call.code, caller_func: call.caller_func });
            }
            changed += 1;
        }
        let file_set: std::collections::HashSet<String> = files.iter().map(|p| p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")).collect();
        self.data.files.retain(|k, _| file_set.contains(k));
        self.data.symbols.retain(|s| file_set.contains(&s.file));
        self.data.callers.retain(|c| file_set.contains(&c.file));
        self.save()?;
        self.rebuild_indexes();
        Ok(IndexReport { total_files: files.len(), changed_files: changed, total_symbols: self.data.symbols.len(), elapsed_ms: start.elapsed().as_millis() as u64 })
    }

    fn rebuild_indexes(&mut self) {
        self.symbol_index.clear();
        for (i, s) in self.data.symbols.iter().enumerate() {
            self.symbol_index.entry(s.name.to_lowercase()).or_default().push(i);
        }
        self.caller_index.clear();
        for (i, c) in self.data.callers.iter().enumerate() {
            self.caller_index.entry(c.callee.to_lowercase()).or_default().push(i);
        }
    }

    fn save(&self) -> crate::Result<()> {
        let json = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&self.db_path, json)?;
        Ok(())
    }

    pub fn search_symbols(&self, query: &str, kind: Option<&str>, limit: usize) -> crate::Result<Vec<SymbolResult>> {
        let ql = query.to_lowercase();
        let mut seen = std::collections::HashSet::new();
        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        if let Some(indices) = self.symbol_index.get(&ql) {
            for &i in indices { if seen.insert(i) { exact.push(i); } }
        }
        for (key, indices) in &self.symbol_index {
            if key != &ql && key.starts_with(&ql) {
                for &i in indices { if seen.insert(i) { prefix.push(i); } }
            }
        }
        let mut results: Vec<SymbolResult> = Vec::new();
        let mut remaining = limit;
        for &i in &exact {
            if remaining == 0 { break; }
            let s = &self.data.symbols[i];
            if kind.is_none_or(|k| s.kind == k) {
                results.push(SymbolResult { name: s.name.clone(), kind: s.kind.clone(), file: s.file.clone(), line: s.line, signature: s.signature.clone() });
                remaining -= 1;
            }
        }
        for &i in &prefix {
            if remaining == 0 { break; }
            let s = &self.data.symbols[i];
            if kind.is_none_or(|k| s.kind == k) {
                results.push(SymbolResult { name: s.name.clone(), kind: s.kind.clone(), file: s.file.clone(), line: s.line, signature: s.signature.clone() });
                remaining -= 1;
            }
        }
        if remaining > 0 {
            let candidates: Vec<usize> = self.data.symbols.iter().enumerate()
                .filter(|(i, s)| !seen.contains(i) && s.name.to_lowercase().contains(&ql) && kind.is_none_or(|k| s.kind == k))
                .take(remaining)
                .map(|(i, _)| i)
                .collect();
            for i in candidates {
                let s = &self.data.symbols[i];
                results.push(SymbolResult { name: s.name.clone(), kind: s.kind.clone(), file: s.file.clone(), line: s.line, signature: s.signature.clone() });
            }
        }
        Ok(results)
    }

    pub fn find_callers(&self, symbol: &str, file: Option<&str>) -> crate::Result<Vec<CallerResult>> {
        let sl = symbol.to_lowercase();
        let indices = self.caller_index.get(&sl).map(|v| v.as_slice()).unwrap_or(&[]);
        Ok(indices.iter()
            .map(|&i| &self.data.callers[i])
            .filter(|c| file.is_none_or(|f| c.file.contains(f)))
            .map(|c| CallerResult { file: c.file.clone(), line: c.line, code: c.code.clone(), caller_func: c.caller_func.clone() })
            .collect())
    }

    pub fn project_map(&self, depth: usize) -> crate::Result<String> {
        let tf = self.data.files.len();
        let ts = self.data.symbols.len();
        let tl: usize = self.data.files.values().map(|f| f.lines).sum();
        let mut lc: HashMap<&str, usize> = HashMap::new();
        for f in self.data.files.values() { *lc.entry(&f.language).or_insert(0) += 1; }
        let ls: String = lc.iter().map(|(l, c)| format!("{} ({} files)", l, c)).collect::<Vec<_>>().join(", ");
        let mut tree = String::new();
        let mut paths: Vec<&String> = self.data.files.keys().collect();
        paths.sort();
        let mut pd: Vec<String> = Vec::new();
        for path in &paths {
            let parts: Vec<&str> = path.split('/').collect();
            let d = parts.len().saturating_sub(1).min(depth);
            for i in 0..d {
                let dp: String = parts[..=i].join("/");
                if !pd.contains(&dp) { pd.push(dp); tree.push_str(&format!("{}{}/\n", "  ".repeat(i), parts[i])); }
            }
            tree.push_str(&format!("{}{}\n", "  ".repeat(d), parts.last().unwrap_or(&"")));
        }
        Ok(format!("Project: {} ({})\nFiles: {}\nSymbols: {}\nLines: {}\n\nDirectory tree:\n{}",
            self.root.file_name().unwrap_or_default().to_string_lossy(), ls, tf, ts, tl, tree))
    }

    pub fn is_empty(&self) -> bool { self.data.files.is_empty() }
}

fn discover_source_files(root: &Path) -> Vec<PathBuf> {
    let walker = ignore::WalkBuilder::new(root).hidden(false).git_ignore(true).git_global(true).git_exclude(true).build();
    let mut files: Vec<PathBuf> = walker
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
        .filter(|e| {
            let p = e.path();
            p.strip_prefix(root).is_ok_and(|rel| {
                let rel_s = rel.to_string_lossy();
                !rel_s.starts_with(".orion") && is_source_file(p)
            })
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    files
}

fn is_source_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "py" | "ts" | "js" | "go" | "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "java"))
}

fn detect_lang(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts") | Some("js") => "javascript",
        Some("go") => "go",
        Some("c") | Some("h") | Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => "c",
        Some("java") => "java",
        _ => "unknown",
    }
}

fn hash_file(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

struct RawSymbol { name: String, kind: String, line: usize, signature: String }
struct RawCall { callee: String, line: usize, code: String, caller_func: String }

fn parse_symbols(content: &str, lang: &str) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let t = line.trim();
        let ln = i + 1;
        let kws: &[(&str, &str)] = match lang {
            "rust" => &[("pub async fn ","function"),("async fn ","function"),("pub fn ","function"),("fn ","function"),("pub struct ","struct"),("struct ","struct"),("pub trait ","trait"),("trait ","trait"),("pub enum ","enum"),("enum ","enum"),("pub mod ","module"),("mod ","module")],
            "python" => &[("def ","function"),("class ","struct")],
            "javascript" => &[("function ","function"),("async function ","function"),("class ","struct")],
            "go" => &[("func ","function")],
            "c" => &[("class ","struct"),("struct ","struct"),("#define ","module")],
            "java" => &[("class ","struct"),("interface ","trait")],
            _ => &[],
        };
        for &(kw, kind) in kws {
            if let Some(name) = extract_after_kw(t, kw) {
                symbols.push(RawSymbol { name, kind: kind.to_string(), line: ln, signature: t.to_string() });
            }
        }
        if lang == "c" {
            if let Some(name) = extract_c_func(t) {
                symbols.push(RawSymbol { name, kind: "function".to_string(), line: ln, signature: t.to_string() });
            }
        }
        if lang == "java" {
            for kw in &["public ","private ","protected "] {
                if let Some(name) = extract_java_after_modifier(t, kw) {
                    symbols.push(RawSymbol { name, kind: "function".to_string(), line: ln, signature: t.to_string() });
                }
            }
        }
    }
    symbols
}

fn parse_callers(content: &str) -> Vec<RawCall> {
    let mut calls = Vec::new();
    let mut current_fn = String::new();
    for (i, line) in content.lines().enumerate() {
        let t = line.trim();
        for kw in &["pub async fn ","async fn ","pub fn ","fn ","def ","func ","void ","public ","private ","protected "] {
            if let Some(name) = extract_after_kw(t, kw) { current_fn = name; break; }
        }
        if t.contains('(') && !t.starts_with("//") && !t.starts_with('#') {
            if let Some(call) = extract_call(t, &current_fn, i + 1) { calls.push(call); }
        }
    }
    calls
}

fn extract_call(line: &str, caller_func: &str, line_num: usize) -> Option<RawCall> {
    for kw in &["fn ","def ","func ","struct ","trait ","enum ","mod ","class ","interface "] {
        if line.trim_start().starts_with(kw) { return None; }
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
            let name = &line[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j] == b' ' { j += 1; }
            if j < bytes.len() && bytes[j] == b'(' && !name.is_empty() && name.chars().next().unwrap().is_alphabetic() {
                return Some(RawCall { callee: name.to_string(), line: line_num, code: line.trim().to_string(), caller_func: caller_func.to_string() });
            }
        }
        i += 1;
    }
    None
}

fn extract_after_kw(line: &str, kw: &str) -> Option<String> {
    let pos = line.find(kw)?;
    let rest = &line[pos + kw.len()..];
    let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
    let name = rest[..end].to_string();
    if name.is_empty() || !name.chars().next().unwrap().is_alphabetic() { None } else { Some(name) }
}

/// C/C++ style: return_type identifier(...) e.g. `void my_func(int arg) {`
fn extract_c_func(line: &str) -> Option<String> {
    let t = line.trim();
    if !(t.ends_with('{') || t.ends_with(';')) { return None; }
    let c_types = ["void ","int ","char ","bool ","float ","double ","long ","short ","unsigned ","signed ","size_t ","ssize_t ","int8_t ","int16_t ","int32_t ","int64_t ","uint8_t ","uint16_t ","uint32_t ","uint64_t "];
    let kw_skip = ["if ","for ","while ","switch ","return ","else ","sizeof ","case ","do "];
    for ct in &c_types {
        if let Some(rest) = t.strip_prefix(ct) {
            let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
            let name = rest[..end].to_string();
            if name.is_empty() || !name.chars().next().unwrap().is_alphabetic() { continue; }
            if kw_skip.iter().any(|k| k.trim() == name) { continue; }
            let after = &rest[end..];
            let after_trim = after.trim_start();
            if after_trim.starts_with('(') {
                return Some(name);
            }
        }
    }
    None
}

/// Java: after access modifier, recursively skip `static`/`final`/`void`/`int`/etc., extract identifier
fn extract_java_after_modifier(line: &str, modifier: &str) -> Option<String> {
    let t = line.trim();
    if !t.starts_with(modifier) { return None; }
    let rest = &t[modifier.len()..];
    let java_skip = ["static ","final ","abstract ","synchronized ","native ","strictfp ","transient ","volatile ","void ","int ","long ","short ","byte ","char ","float ","double ","boolean ","String ","var "];
    for sk in &java_skip {
        if rest.starts_with(sk) {
            return extract_java_after_modifier(rest, sk);
        }
    }
    let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(rest.len());
    let name = rest[..end].to_string();
    if name.is_empty() || !name.chars().next().unwrap().is_alphabetic() { return None; }
    let after = &rest[end..];
    let after_trim = after.trim_start();
    if after_trim.starts_with('(') { Some(name) } else { None }
}
