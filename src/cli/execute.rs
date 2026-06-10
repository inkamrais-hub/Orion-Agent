use crate::core::cache::GlobalCache;
use crate::core::r#loop::{run_simple_loop, LoopOutcome, LoopEvent, EventCallback, tool_title, classify_bash_risk, BashRisk, truncate_str};
use crate::session::UnifiedStore;
use crate::session::unified::TranscriptEntry as UnifiedTranscriptEntry;
use crate::tools::registry::ToolRegistry;
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

/// 构建 System Prompt (静态部分，用于 Prompt Caching)
pub fn build_system_prompt_static() -> String {
    let mut prompt = String::new();

    // ── 角色定义 ──
    prompt.push_str("You are Orion, a powerful coding assistant built in Rust.\n");
    prompt.push_str("You help users with software engineering tasks: writing code, debugging, refactoring, analyzing codebases, and executing commands.\n\n");

    // ── 核心原则 ──
    prompt.push_str("[Core Principles]\n");
    prompt.push_str("1. **Use tools, don't just explain.** When asked to do something, do it with tools.\n");
    prompt.push_str("2. **Read before modify.** Always read a file before editing it.\n");
    prompt.push_str("3. **Minimal changes.** Only change what's needed. Don't refactor unrelated code.\n");
    prompt.push_str("4. **Verify your work.** Run tests or build commands after making changes.\n");
    prompt.push_str("5. **Be direct.** Lead with the answer/action, not reasoning.\n\n");

    // ── 工作流程 ──
    prompt.push_str("[Workflow]\n");
    prompt.push_str("For complex tasks:\n");
    prompt.push_str("1. **Understand** - Read relevant files and code to understand the context\n");
    prompt.push_str("2. **Plan** - Identify what needs to change (don't output the plan, just think it)\n");
    prompt.push_str("3. **Execute** - Make changes using tools\n");
    prompt.push_str("4. **Verify** - Run `cargo check`, `npm run lint`, or similar to validate\n\n");

    // ── 安全约束 ──
    prompt.push_str("[Safety]\n");
    prompt.push_str("- Never run `rm -rf /`, `format`, `dd`, or other destructive commands\n");
    prompt.push_str("- Ask for confirmation before deleting files or running sudo\n");
    prompt.push_str("- Don't commit changes unless explicitly asked\n");
    prompt.push_str("- Don't modify files outside the working directory without permission\n\n");

    // ── 输出格式 ──
    prompt.push_str("[Output Format]\n");
    prompt.push_str("- Use Chinese for explanations (user's primary language)\n");
    prompt.push_str("- Use English for code, comments, and technical terms\n");
    prompt.push_str("- When showing code changes, use markdown code blocks with language tags\n");
    prompt.push_str("- When referencing files, use clickable links: `filename` (path)\n");
    prompt.push_str("- Keep responses concise. One sentence is better than three.\n\n");

    // ── 工具类别 ──
    prompt.push_str("[Tool Categories]\n");
    prompt.push_str("You have access to these tool categories. Call tools directly by name.\n\n");
    let categories = crate::tools::category::create_default_categories();
    prompt.push_str(&categories.brief_list());
    prompt.push('\n');

    prompt
}

/// 构建动态上下文 (每次请求变化的部分)
pub fn build_dynamic_context() -> String {
    let mut ctx = String::new();
    let os = std::env::consts::OS;
    let shell = if cfg!(windows) { "cmd /C (batch)" } else { "sh -c" };
    let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| "(unknown)".into());
    ctx.push_str("[Context]\n");
    ctx.push_str(&format!("OS: {} | Shell: {} | CWD: {}\n", os, shell, cwd));
    if cfg!(windows) {
        let mut drives = Vec::new();
        for letter in b'A'..=b'Z' {
            let path = format!("{}:\\", letter as char);
            if std::path::Path::new(&path).exists() { drives.push(format!("{}:", letter as char)); }
        }
        if !drives.is_empty() { ctx.push_str(&format!("Drives: {}\n", drives.join(", "))); }
    }
    ctx.push('\n');
    ctx
}

/// 兼容旧接口
pub fn build_system_prompt(_tools: &ToolRegistry) -> String {
    build_system_prompt_static()
}

/// 创建默认的 CLI 事件回调 — 负责终端 UI 渲染
///
/// 将 UI 渲染逻辑（Thinking、ToolStart/End、TextDelta 等的终端输出）
/// 从执行逻辑中解耦，便于复用和测试。
pub fn create_cli_event_callback() -> EventCallback {
    let thinking_buf = std::sync::Mutex::new(String::new());
    let in_thinking = std::sync::Mutex::new(false);
    const W: usize = 60;

    Box::new(move |event: &LoopEvent| {
        match event {
            LoopEvent::ThinkingDelta { text } => {
                if text.is_empty() {
                    eprintln!();
                    *in_thinking.lock().unwrap() = true;
                    thinking_buf.lock().unwrap().clear();
                } else {
                    thinking_buf.lock().unwrap().push_str(text);
                }
            }
            LoopEvent::TextDelta(text) => {
                flush_thinking(&thinking_buf, &in_thinking, W);
                use std::io::Write;
                print!("{}", text);
                std::io::stdout().flush().ok();
            }
            LoopEvent::ToolStart { tool_name, input, .. } => {
                flush_thinking(&thinking_buf, &in_thinking, W);
                if tool_name == "bash" {
                    let cmd = input["command"].as_str().unwrap_or("?");
                    let risk = classify_bash_risk(cmd);
                    let icon = match risk {
                        BashRisk::Safe => "🟢", BashRisk::Low => "🟡",
                        BashRisk::Medium => "🟠", BashRisk::High => "🔴", BashRisk::Critical => "⛔",
                    };
                    print_box(&format!("{} Bash: {}", icon, truncate_str(cmd, 40)), W);
                } else {
                    let title = tool_title(tool_name, input);
                    print_box(&title, W);
                }
            }
            LoopEvent::ToolEnd { tool_name, result, is_error, duration_ms, .. } => {
                let status = if *is_error { "✗" } else { "✓" };
                let header = format!("{} {} ({}ms)", status, tool_name, duration_ms);
                print_result_box(&header, result, if tool_name == "bash" { 300 } else { 200 }, W);
            }
            LoopEvent::TurnComplete { turn } => {
                print_box(&format!("Turn {}", turn), W);
            }
            LoopEvent::Error(msg) => {
                print_box(&format!("Error: {}", msg), W);
            }
        }
    })
}

pub async fn execute_turn(
    provider: &dyn crate::core::provider::Provider,
    tools: &ToolRegistry,
    cache: &GlobalCache,
    store: &Arc<UnifiedStore>,
    session_id: &str,
    user_input: &str,
    model: &str,
    system_prompt: &str,
    event_callback: Option<EventCallback>,
    images: Option<Vec<crate::core::provider::ContentBlock>>,
    thinking: bool,
    reasoning_effort: &str,
) -> crate::Result<String> {
    let history = store.get_transcripts(session_id).await.unwrap_or_default();

    // Append user message to transcript
    let user_msg_id = Uuid::new_v4().to_string();
    let now_str = Utc::now().to_rfc3339();
    store
        .append_transcript(&UnifiedTranscriptEntry {
            id: user_msg_id.clone(),
            session_id: session_id.to_string(),
            parent_id: history.last().map(|e| e.id.clone()),
            role: "user".to_string(),
            content: user_input.to_string(),
            tool_calls: None,
            timestamp: now_str,
        })
        .await
        .ok();

    let event_cb = event_callback.unwrap_or_else(create_cli_event_callback);

    // 思考深度映射到 max_output_tokens
    let max_output_tokens = match reasoning_effort {
        "low" => 2048,
        "medium" => 4096,
        "high" => 8192,
        "max" => 16384,
        "xhigh" => 32768,
        _ => 4096,
    };

    let loop_config = crate::core::r#loop::SimpleLoopConfig {
        model: model.to_string(),
        system_prompt: system_prompt.to_string(),
        max_turns: 20,
        max_tool_calls: 30,
        token_budget: 128_000,
        agent_id: "main".into(),
        session_id: session_id.to_string(),
        model_caps: crate::core::r#loop::ModelCaps {
            thinking,
            prompt_cache: false,
            max_output_tokens,
        },
        exec_mode: crate::core::exec_mode::ExecMode::default(),
    };
    let outcome = run_simple_loop(
        provider, tools, cache, &loop_config, user_input,
        crate::core::r#loop::SimpleLoopContext {
            event_callback: Some(event_cb),
            images,
            ..Default::default()
        },
    ).await;

    let response = match outcome {
        LoopOutcome::Completed { message, usage } => {
            tracing::info!(input = usage.input_tokens, output = usage.output_tokens, "Turn completed");
            message
        }
        LoopOutcome::MaxTurnsReached { message, .. } => format!("[Max turns] {}", message),
        LoopOutcome::Error { message } => format!("[Error] {}", message),
        LoopOutcome::BudgetExceeded { .. } => "[Budget exceeded]".to_string(),
        LoopOutcome::GuardrailDenied { reason } => format!("[Guardrail] {}", reason),
    };

    // Append assistant response to transcript (parent is the user message just appended)
    let now_str2 = Utc::now().to_rfc3339();
    store
        .append_transcript(&UnifiedTranscriptEntry {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            parent_id: Some(user_msg_id),
            role: "assistant".to_string(),
            content: response.clone(),
            tool_calls: None,
            timestamp: now_str2,
        })
        .await
        .ok();

    // Update session turn count
    if let Ok(Some(session)) = store.get_session(session_id).await {
        store
            .update_session_stats(session_id, session.turn_count + 1, session.tool_call_count, session.total_tokens)
            .await
            .ok();
    }

    Ok(response)
}

fn flush_thinking(buf: &std::sync::Mutex<String>, active: &std::sync::Mutex<bool>, w: usize) {
    if *active.lock().unwrap() {
        let content = buf.lock().unwrap().clone();
        if !content.is_empty() {
            let header = " Thinking ";
            let pad = "─".repeat(w.saturating_sub(header.len() + 2).max(0));
            eprintln!("╭─{}{}╮", header, pad);
            for para in content.split("\n\n") {
                let trimmed = para.trim();
                if !trimmed.is_empty() {
                    eprintln!("│ {:<width$} │", truncate_str(trimmed, w - 4), width = w - 2);
                }
            }
            eprintln!("╰{}╯", "─".repeat(w));
        }
        *active.lock().unwrap() = false;
        *buf.lock().unwrap() = String::new();
    }
}

fn print_box(title: &str, w: usize) {
    let header = format!(" {} ", title);
    let pad = "─".repeat(w.saturating_sub(header.len() + 2).max(0));
    eprintln!("\n╭─{}{}╮", header, pad);
    eprintln!("╰{}╯", "─".repeat(w));
}

fn print_result_box(title: &str, content: &str, max_chars: usize, w: usize) {
    let header = format!(" {} ", title);
    let pad = "─".repeat(w.saturating_sub(header.len() + 2).max(0));
    eprintln!("╭─{}{}╮", header, pad);
    let output = truncate_str(content, max_chars);
    for line in output.lines() {
        eprintln!("│ {:<width$} │", truncate_str(line, w - 4), width = w - 2);
    }
    eprintln!("╰{}╯", "─".repeat(w));
}
