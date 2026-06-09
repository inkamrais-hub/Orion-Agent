use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};
use crate::index::CODE_INDEX;

pub struct FindCallersTool;

#[async_trait]
impl Tool for FindCallersTool {
    fn name(&self) -> &str { "find_callers" }
    fn description(&self) -> &str {
        "Find all call sites of a function. Shows which files and functions will be affected by changes. More precise than grep: only finds actual calls, not string matches."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"symbol":{"type":"string","description":"Function name"},"file":{"type":"string","description":"Definition file (optional, for precision)"}},"required":["symbol"]})
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> crate::Result<ToolResult> {
        let sym = input["symbol"].as_str().ok_or_else(||crate::Error::Tool("missing symbol".into()))?;
        let file_filter = input["file"].as_str();
        let root = std::path::PathBuf::from(&ctx.working_dir);

        // 使用持久化索引
        {
            let mut guard = CODE_INDEX.lock().await;
            if guard.is_none() {
                *guard = Some(crate::index::CodeIndex::open(&root)?);
            }
            let idx = guard.as_mut().unwrap();
            let _ = idx.index();
            if !idx.is_empty() {
                let results = idx.find_callers(sym, file_filter).unwrap_or_default();
                return Ok(ToolResult { content: serde_json::to_string_pretty(&results).unwrap_or_default(), is_error: false, metadata: None });
            }
        }

        // 回退: 全量扫描 (仅当索引为空时)
        let mut callers = Vec::new();
        find_in_dir(&root, &root, sym, &mut callers, 50);
        if callers.is_empty() {
            Ok(ToolResult { content: format!("No callers of '{}' found", sym), is_error: false, metadata: None })
        } else {
            Ok(ToolResult { content: serde_json::to_string_pretty(&callers).unwrap_or_default(), is_error: false, metadata: None })
        }
    }
}

#[derive(serde::Serialize)]
struct Caller { file: String, line: usize, code: String, caller_func: String }

fn find_in_dir(root:&std::path::Path,dir:&std::path::Path,sym:&str,res:&mut Vec<Caller>,max:usize){
    if res.len()>=max{return;}
    let Ok(entries)=std::fs::read_dir(dir) else{return};
    for e in entries.flatten(){
        if res.len()>=max{return;}
        let p=e.path();let n=e.file_name().to_string_lossy().to_string();
        if n.starts_with('.')||n=="target"||n=="node_modules"||n==".git"{continue;}
        if p.is_dir(){find_in_dir(root,&p,sym,res,max);}
        else if matches!(p.extension().and_then(|e|e.to_str()),Some("rs"|"py"|"ts"|"js"|"go")){
            find_in_file(root,&p,sym,res);
        }
    }
}

fn find_in_file(root:&std::path::Path,path:&std::path::Path,sym:&str,res:&mut Vec<Caller>){
    let Ok(content)=std::fs::read_to_string(path)else{return};
    let rel=path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string();
    let mut current_fn=String::new();
    for(i,line)in content.lines().enumerate(){
        let t=line.trim();
        if let Some(name)=extract_fn_name(t){current_fn=name;}
        if t.contains(sym)&&!t.starts_with("//")&&!t.starts_with("///")&&t.contains('('){
            let clean=t.trim_start_matches("pub ").trim_start_matches("async ");
            if clean.starts_with("fn ")||clean.starts_with("struct ")||clean.starts_with("trait "){continue;}
            res.push(Caller{file:rel.clone(),line:i+1,code:t.to_string(),caller_func:current_fn.clone()});
        }
    }
}

fn extract_fn_name(line:&str)->Option<String>{
    let t=line.trim();
    for kw in &["pub async fn ","async fn ","pub fn ","fn "]{
        if let Some(pos)=t.find(kw){
            let rest=&t[pos+kw.len()..];
            let end=rest.find(|c:char|!c.is_alphanumeric()&&c!='_').unwrap_or(rest.len());
            let name=rest[..end].to_string();
            if !name.is_empty()&&name.chars().next().unwrap().is_alphabetic(){return Some(name);}
        }
    }
    None
}