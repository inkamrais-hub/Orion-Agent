use std::sync::Arc;

use crate::core::r#loop::{run_simple_loop, LoopOutcome, LoopEvent, EventCallback, tool_title, classify_bash_risk, BashRisk, truncate_str};
use crate::session::UnifiedStore;
use crate::session::unified::TranscriptEntry;
use crate::tools::registry::ToolRegistry;
use chrono::Utc;
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
pub fn create_cli_event_callback() -> EventCallback {
    let thinking_buf = std::sync::Mutex::new(String::new());
    let in_thinking = std::sync::Mutex::new(false);

    Arc::new(move |event: &LoopEvent| {
        match event {
            LoopEvent::ThinkingDelta { text } => {
                if text.is_empty() {
                    *in_thinking.lock().unwrap() = true;
                    thinking_buf.lock().unwrap().clear();
                } else {
                    thinking_buf.lock().unwrap().push_str(text);
                }
            }
            LoopEvent::TextDelta(text) => {
                flush_thinking(&thinking_buf, &in_thinking);
                use std::io::Write;
                print!("{}", text);
                std::io::stdout().flush().ok();
            }
            LoopEvent::ToolStart { tool_name, input, .. } => {
                flush_thinking(&thinking_buf, &in_thinking);
                if tool_name == "bash" {
                    let cmd = input["command"].as_str().unwrap_or("?");
                    let risk = classify_bash_risk(cmd);
                    let icon = match risk {
                        BashRisk::Safe => "🟢 [Safe]",
                        BashRisk::Low => "🟡 [Low Risk]",
                        BashRisk::Medium => "🟠 [Medium Risk]",
                        BashRisk::High => "🔴 [High Risk]",
                        BashRisk::Critical => "⛔ [Critical Risk]",
                    };
                    eprintln!("🛠️  运行 Bash ({}): {}", icon, cmd);
                } else {
                    let title = tool_title(tool_name, input);
                    eprintln!("🛠️  调用工具: {}", title);
                }
            }
            LoopEvent::ToolEnd { tool_name, result, is_error, duration_ms, .. } => {
                if *is_error {
                    eprintln!("✗ {} 执行失败 ({}ms)", tool_name, duration_ms);
                    if !result.trim().is_empty() {
                        let lines: Vec<&str> = result.lines().take(5).collect();
                        for line in lines {
                            eprintln!("  │ {}", truncate_str(line, 100));
                        }
                        if result.lines().count() > 5 {
                            eprintln!("  │ ... (output truncated)");
                        }
                    }
                } else {
                    eprintln!("✓ {} 执行成功 ({}ms)", tool_name, duration_ms);
                }
                eprintln!();
            }
            LoopEvent::TurnComplete { turn } => {
                eprintln!("─── 轮次 {} 完成 ───\n", turn);
            }
            LoopEvent::Error(msg) => {
                eprintln!("❌ 错误: {}\n", msg);
            }
        }
    })
}

pub async fn execute_turn(
    agent: &crate::core::agent::Agent,
    store: &Arc<UnifiedStore>,
    session_id: &str,
    user_input: &str,
    system_prompt: &str,
    event_callback: Option<EventCallback>,
    images: Option<Vec<crate::core::provider::ContentBlock>>,
    thinking: bool,
    reasoning_effort: &str,
) -> crate::Result<String> {
    let history = store.get_transcript(session_id).await.unwrap_or_default();
    store.append_transcript(session_id, &TranscriptEntry {
        id: Uuid::new_v4().to_string(),
        parent_id: history.last().map(|e| e.id.clone()),
        role: "user".to_string(),
        content: user_input.to_string(),
        tool_calls: None,
        timestamp: Utc::now(),
    }).await.ok();

    let event_cb = event_callback.unwrap_or_else(create_cli_event_callback);

    let max_output_tokens = match reasoning_effort {
        "low" => 2048,
        "medium" => 4096,
        "high" => 8192,
        "max" => 16384,
        "xhigh" => 32768,
        _ => 4096,
    };

    let loop_config = crate::core::r#loop::SimpleLoopConfig {
        model: agent.config().model.clone(),
        system_prompt: system_prompt.to_string(),
        max_turns: agent.config().max_turns,
        max_tool_calls: agent.config().max_tool_calls,
        token_budget: agent.config().token_budget,
        agent_id: agent.config().name.clone(),
        session_id: session_id.to_string(),
        model_caps: crate::core::r#loop::ModelCaps {
            thinking,
            prompt_cache: false,
            max_output_tokens,
        },
    };
    
    let outcome = run_simple_loop(
        agent.provider(),
        agent.tools(),
        agent.cache(),
        &loop_config,
        user_input,
        crate::core::r#loop::SimpleLoopContext {
            event_callback: Some(event_cb),
            hook_engine: agent.hook_engine(),
            exec_policy: agent.exec_policy(),
            registry: agent.registry(),
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

    store.append_transcript(session_id, &TranscriptEntry {
        id: Uuid::new_v4().to_string(),
        parent_id: history.last().map(|e| e.id.clone()),
        role: "assistant".to_string(),
        content: response.clone(),
        tool_calls: None,
        timestamp: Utc::now(),
    }).await.ok();

    // 更新 session 统计 (turn_count + 1)
    if let Ok(Some(meta)) = store.get_session(session_id).await {
        store.update_session_stats(
            session_id,
            meta.turn_count + 1,
            meta.tool_call_count,
            meta.total_tokens,
        ).await.ok();
    }

    Ok(response)
}

fn flush_thinking(buf: &std::sync::Mutex<String>, active: &std::sync::Mutex<bool>) {
    if *active.lock().unwrap() {
        let content = buf.lock().unwrap().clone();
        if !content.is_empty() {
            eprintln!("\n🧠 [Thinking]");
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    eprintln!("   │ {}", trimmed);
                }
            }
            eprintln!();
        }
        *active.lock().unwrap() = false;
        *buf.lock().unwrap() = String::new();
    }
}
