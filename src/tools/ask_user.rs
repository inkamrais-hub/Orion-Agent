use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{Tool, ToolContext, ToolResult};

pub struct AskUserTool;

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str { "ask_user" }
    fn description(&self) -> &str {
        "Ask the user a question when you need clarification, confirmation, or have multiple options. Blocks until the user responds. Use when uncertain or need approval."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"question":{"type":"string","description":"Question to ask"},"options":{"type":"array","items":{"type":"string"},"description":"Multiple choice options (optional)"}},"required":["question"]})
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let question = input["question"].as_str().ok_or_else(||crate::Error::Tool("missing question".into()))?;
        let options: Option<Vec<String>> = input["options"].as_array().map(|a| a.iter().filter_map(|v|v.as_str().map(String::from)).collect());
        if !atty::is(atty::Stream::Stdout) {
            return Ok(ToolResult { content: "Not available in non-interactive mode".into(), is_error: true, metadata: None });
        }
        eprintln!("
🤖 Agent asks: {}", question);
        if let Some(ref opts) = options {
            for(i,o)in opts.iter().enumerate(){eprintln!("  [{}] {}",i+1,o);}
            eprint!("Your choice: ");
        } else {
            eprint!("Your answer: ");
        }
        use std::io::Write;
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).map_err(crate::Error::Io)?;
        let answer = answer.trim().to_string();
        if answer.is_empty() {
            Ok(ToolResult { content: "User did not respond".into(), is_error: false, metadata: None })
        } else {
            Ok(ToolResult { content: answer, is_error: false, metadata: None })
        }
    }
}