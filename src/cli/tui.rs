//! TUI (Terminal User Interface) for Orion Agent
//!
//! A full-featured terminal interface built with ratatui, providing:
//! - Scrollable conversation pane
//! - Single-line input with cursor
//! - Status bar with model/workspace/session info
//! - Streaming AI response display
//! - Tool call/result visualization in bordered boxes
//! - Basic markdown rendering (bold, code blocks, headers)

use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::cli::execute::{build_system_prompt_static, execute_turn};
use crate::config::OrionConfig;
use crate::core::cache::GlobalCache;
use crate::core::provider::Provider;
use crate::core::providers;
use crate::core::r#loop::{LoopEvent, EventCallback};
use crate::session::UnifiedStore;
use crate::tools::registry::ToolRegistry;

// ============================================================
//  Types
// ============================================================

/// Events sent from the agent loop callback to the TUI
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// Incremental AI text
    TextDelta(String),
    /// Incremental thinking text
    ThinkingDelta(String),
    /// Tool call started
    ToolCallStart { name: String, input: String },
    /// Tool call completed
    ToolCallEnd { output: String, is_error: bool },
    /// Full turn completed
    TurnComplete(String),
    /// Error occurred
    Error(String),
}

/// A single message in the conversation history
#[derive(Debug, Clone)]
pub enum ChatMessage {
    User(String),
    Assistant(String),
    ToolCall {
        name: String,
        input: String,
        output: String,
        is_error: bool,
    },
    System(String),
}

/// Display state for a currently executing tool
#[derive(Debug, Clone)]
pub struct ToolDisplay {
    pub name: String,
    pub input_summary: String,
    pub output: Option<String>,
}

/// Main TUI application state
pub struct TuiApp {
    /// Terminal backend
    terminal: Terminal<CrosstermBackend<Stdout>>,

    /// Conversation history
    messages: Vec<ChatMessage>,
    /// Current input buffer
    input: String,
    /// Cursor position in input
    cursor_pos: usize,
    /// Scroll offset in conversation pane
    scroll_offset: usize,
    /// Current model name
    model: String,
    /// Workspace path
    workspace: String,
    /// Current session ID
    session_id: String,
    /// Whether AI is currently streaming
    is_streaming: bool,
    /// Accumulated streaming text
    streaming_text: String,
    /// Currently executing tool
    current_tool: Option<ToolDisplay>,
    /// Whether TUI should keep running
    running: bool,
    /// Receiver for TUI events from agent loop
    event_rx: mpsc::UnboundedReceiver<TuiEvent>,
    /// Sender clone for agent loop callbacks
    event_tx: mpsc::UnboundedSender<TuiEvent>,

    // Agent infrastructure (Option so we can take ownership during execution)
    provider: Option<Box<dyn Provider>>,
    tools: Option<ToolRegistry>,
    cache: Option<GlobalCache>,
    store: Option<Arc<UnifiedStore>>,
    system_prompt: String,
    thinking: bool,
    reasoning_effort: String,
}

// ============================================================
//  Entry Point
// ============================================================

/// Launch the TUI application
///
/// This is the main entry point for the TUI mode. It:
/// 1. Initializes crossterm (raw mode, alternate screen)
/// 2. Sets up the TUI app state (loads config, creates provider, registers tools)
/// 3. Enters the main event loop
/// 4. Cleans up terminal on exit
pub async fn run_tui(config: OrionConfig, working_dir: Option<String>) -> crate::Result<()> {
    // Initialize terminal
    enable_raw_mode().map_err(|e| crate::Error::Io(e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| crate::Error::Io(e))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| crate::Error::Config(format!("Terminal init failed: {}", e)))?;
    terminal.clear().map_err(|e| crate::Error::Config(format!("Terminal clear failed: {}", e)))?;

    // Set up panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    // Initialize agent infrastructure
    let model_config = config.active_model();
    let workspace = working_dir.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into())
    });

    // API Key validation
    let api_key = model_config.api_key.clone()
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("LLM_API_KEY").ok())
        .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
        .unwrap_or_default();

    if api_key.is_empty() {
        // Restore terminal before returning error
        disable_raw_mode().ok();
        execute!(io::stdout(), LeaveAlternateScreen).ok();
        return Err(crate::Error::Config(
            "API Key not set. Configure models[].api_key in ~/.orion/config.yaml, or set LLM_API_KEY".into()
        ));
    }

    // Create provider
    let mut mc = model_config.clone();
    mc.api_key = Some(api_key);
    let provider: Box<dyn Provider> = providers::create_provider(&mc);

    // Register tools
    let mut tools = ToolRegistry::new();
    crate::tools::register_default_tools(&mut tools);
    tools.register(crate::tools::multi_shell::MultiShellTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotHistoryTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRollbackTool);
    tools.register(crate::tools::code_intelligence::file_snapshot::SnapshotRiskyTool);
    let search_proxy = model_config.proxy.clone()
        .unwrap_or_else(|| std::env::var("HTTP_PROXY").unwrap_or_default());
    if search_proxy.is_empty() {
        tools.register(crate::tools::web_search::WebSearchTool::new());
    } else {
        tools.register(crate::tools::web_search::WebSearchTool::with_proxy(&search_proxy));
    }

    // Connect MCP servers
    crate::tools::mcp_init::init_mcp_tools(&config, &mut tools).await;

    let cache = GlobalCache::from_config(&config.cache);
    let system_prompt = build_system_prompt_static();

    // Create session
    let store = UnifiedStore::open().await?;
    let session_id = crate::session::store::generate_session_id();
    let now = chrono::Utc::now().to_rfc3339();
    let _ = store.create_session(&crate::session::unified::SessionMeta {
        session_id: session_id.clone(),
        agent_name: "main".into(),
        model: model_config.name.clone(),
        working_dir: workspace.clone(),
        status: crate::session::unified::SessionStatus::Active,
        created_at: now.clone(),
        updated_at: now,
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    }).await;

    // Event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    let thinking = model_config.thinking;

    let mut app = TuiApp {
        terminal,
        messages: vec![ChatMessage::System(format!(
            "Orion Agent v{} | Model: {} | Session: {}...",
            env!("CARGO_PKG_VERSION"),
            model_config.name,
            &session_id[..8.min(session_id.len())]
        ))],
        input: String::new(),
        cursor_pos: 0,
        scroll_offset: 0,
        model: model_config.name.clone(),
        workspace,
        session_id,
        is_streaming: false,
        streaming_text: String::new(),
        current_tool: None,
        running: true,
        event_rx,
        event_tx,
        provider: Some(provider),
        tools: Some(tools),
        cache: Some(cache),
        store: Some(store),
        system_prompt,
        thinking,
        reasoning_effort: "medium".into(),
    };

    // Run the main event loop
    let result = app.run().await;

    // Restore terminal
    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();

    result
}

// ============================================================
//  Main Event Loop
// ============================================================

impl TuiApp {
    /// Main event loop: poll events, handle input, render UI
    async fn run(&mut self) -> crate::Result<()> {
        while self.running {
            // Drain any pending TUI events from the agent loop
            self.drain_events();

            // Pre-render scroll clamping (to avoid mutable borrow in closure)
            self.clamp_scroll_offset();

            // Render the UI
            let messages = &self.messages;
            let streaming_text = &self.streaming_text;
            let current_tool = &self.current_tool;
            let is_streaming = self.is_streaming;
            let scroll_offset = self.scroll_offset;
            let model = &self.model;
            let workspace = &self.workspace;
            let session_id = &self.session_id;
            let input = &self.input;
            let cursor_pos = self.cursor_pos;

            self.terminal.draw(|f| {
                render_ui(
                    f,
                    messages,
                    streaming_text,
                    current_tool,
                    is_streaming,
                    scroll_offset,
                    model,
                    workspace,
                    session_id,
                    input,
                    cursor_pos,
                )
            }).map_err(|e| crate::Error::Io(e))?;

            // Poll for crossterm events with a short timeout
            // This allows us to refresh the UI during streaming
            let timeout = if self.is_streaming {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(100)
            };

            if event::poll(timeout).map_err(|e| crate::Error::Io(e))? {
                if let Event::Key(key) = event::read().map_err(|e| crate::Error::Io(e))? {
                    self.handle_key(key).await;
                }
            }
        }
        Ok(())
    }

    /// Clamp scroll offset to valid range
    fn clamp_scroll_offset(&mut self) {
        let total_lines = self.conversation_line_count();
        // Estimate visible height (conservative)
        let visible_height = 40; // Will be adjusted by actual terminal size
        let max_scroll = total_lines.saturating_sub(visible_height);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
    }

    /// Drain all pending events from the agent loop channel
    fn drain_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                TuiEvent::TextDelta(text) => {
                    self.streaming_text.push_str(&text);
                }
                TuiEvent::ThinkingDelta(_text) => {
                    // Thinking is shown in the status area during streaming
                    // For now, we don't display it separately in the TUI
                }
                TuiEvent::ToolCallStart { name, input } => {
                    self.current_tool = Some(ToolDisplay {
                        name: name.clone(),
                        input_summary: input.clone(),
                        output: None,
                    });
                }
                TuiEvent::ToolCallEnd { output, is_error } => {
                    if let Some(tool) = self.current_tool.take() {
                        self.messages.push(ChatMessage::ToolCall {
                            name: tool.name,
                            input: tool.input_summary,
                            output,
                            is_error,
                        });
                    }
                }
                TuiEvent::TurnComplete(full_text) => {
                    // The streaming text becomes the final assistant message
                    let text = if full_text.is_empty() {
                        self.streaming_text.clone()
                    } else {
                        full_text
                    };
                    if !text.is_empty() {
                        self.messages.push(ChatMessage::Assistant(text));
                    }
                    self.streaming_text.clear();
                    self.is_streaming = false;
                    self.current_tool = None;
                    // Auto-scroll to bottom
                    self.scroll_to_bottom();
                }
                TuiEvent::Error(msg) => {
                    self.messages.push(ChatMessage::System(format!("Error: {}", msg)));
                    self.is_streaming = false;
                    self.streaming_text.clear();
                    self.current_tool = None;
                }
            }
        }
    }

    /// Handle keyboard input
    async fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.is_streaming {
                // Cancel streaming
                self.is_streaming = false;
                self.streaming_text.clear();
                self.current_tool = None;
                self.messages.push(ChatMessage::System("Turn cancelled.".into()));
            } else {
                self.running = false;
            }
            return;
        }

        // During streaming, only Ctrl+C is handled
        if self.is_streaming {
            return;
        }

        match key.code {
            // Enter: submit input
            KeyCode::Enter => {
                let input = self.input.trim().to_string();
                if !input.is_empty() {
                    self.messages.push(ChatMessage::User(input.clone()));
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.scroll_to_bottom();
                    // Execute the turn
                    self.execute_turn(&input).await;
                }
            }
            // Backspace: delete character
            KeyCode::Backspace => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                }
            }
            // Delete: delete character at cursor
            KeyCode::Delete => {
                if self.cursor_pos < self.input.len() {
                    self.input.remove(self.cursor_pos);
                }
            }
            // Left: move cursor left
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
            }
            // Right: move cursor right
            KeyCode::Right => {
                if self.cursor_pos < self.input.len() {
                    self.cursor_pos += 1;
                }
            }
            // Home: move to start
            KeyCode::Home => {
                self.cursor_pos = 0;
            }
            // End: move to end
            KeyCode::End => {
                self.cursor_pos = self.input.len();
            }
            // Up: scroll conversation up
            KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            // Down: scroll conversation down
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            // PageUp: scroll up faster
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }
            // PageDown: scroll down faster
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
            }
            // Regular character input
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
            }
            _ => {}
        }
    }

    /// Execute a turn via the agent loop
    async fn execute_turn(&mut self, user_input: &str) {
        // Check if we have the required infrastructure
        let (provider, tools, cache, store) = match (
            self.provider.as_ref(),
            self.tools.as_ref(),
            self.cache.as_ref(),
            self.store.as_ref(),
        ) {
            (Some(p), Some(t), Some(c), Some(s)) => (p, t, c, s),
            _ => {
                self.messages.push(ChatMessage::System("Error: Agent not initialized".into()));
                return;
            }
        };

        self.is_streaming = true;
        self.streaming_text.clear();

        // Create event callback that sends events to our channel
        let tx = self.event_tx.clone();
        let event_callback: EventCallback = Box::new(move |event: &LoopEvent| {
            let tui_event = match event {
                LoopEvent::TextDelta(text) => TuiEvent::TextDelta(text.clone()),
                LoopEvent::ThinkingDelta { text } => TuiEvent::ThinkingDelta(text.clone()),
                LoopEvent::ToolStart { tool_name, input, .. } => {
                    let input_summary = match tool_name.as_str() {
                        "bash" => input["command"].as_str().unwrap_or("?").to_string(),
                        "read" | "write" | "edit" => input["path"].as_str().unwrap_or("?").to_string(),
                        _ => truncate_input(input),
                    };
                    TuiEvent::ToolCallStart {
                        name: tool_name.clone(),
                        input: input_summary,
                    }
                }
                LoopEvent::ToolEnd { result, is_error, .. } => {
                    TuiEvent::ToolCallEnd {
                        output: result.clone(),
                        is_error: *is_error,
                    }
                }
                LoopEvent::TurnComplete { turn: _ } => {
                    TuiEvent::TurnComplete(String::new())
                }
                LoopEvent::Error(msg) => TuiEvent::Error(msg.clone()),
            };
            let _ = tx.send(tui_event);
        });

        // Run the turn
        let result = execute_turn(
            provider.as_ref(),
            tools,
            cache,
            store,
            &self.session_id,
            user_input,
            &self.model,
            &self.system_prompt,
            Some(event_callback),
            None,
            self.thinking,
            &self.reasoning_effort,
            Some(&self.workspace),
        ).await;

        match result {
            Ok(response) => {
                // If TurnComplete event wasn't received, add the message now
                if self.is_streaming {
                    let text = if self.streaming_text.is_empty() {
                        response
                    } else {
                        self.streaming_text.clone()
                    };
                    if !text.is_empty() {
                        self.messages.push(ChatMessage::Assistant(text));
                    }
                    self.streaming_text.clear();
                    self.is_streaming = false;
                    self.current_tool = None;
                    self.scroll_to_bottom();
                }
            }
            Err(e) => {
                self.messages.push(ChatMessage::System(format!("Error: {}", e)));
                self.is_streaming = false;
                self.streaming_text.clear();
                self.current_tool = None;
            }
        }
    }

    /// Scroll to the bottom of the conversation
    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Calculate the total number of visible lines in the conversation
    fn conversation_line_count(&self) -> usize {
        let mut count = 0;
        for msg in &self.messages {
            count += message_line_count(msg);
            count += 1; // spacing
        }
        // Add streaming text if present
        if self.is_streaming && !self.streaming_text.is_empty() {
            count += count_display_lines(&self.streaming_text, 80);
            count += 1;
        }
        // Add current tool if present
        if self.current_tool.is_some() {
            count += 4; // tool box height
            count += 1;
        }
        count
    }
}

// ============================================================
//  Rendering
// ============================================================

/// Render the entire TUI layout (standalone function to avoid borrow issues)
fn render_ui(
    f: &mut Frame,
    messages: &[ChatMessage],
    streaming_text: &str,
    current_tool: &Option<ToolDisplay>,
    is_streaming: bool,
    scroll_offset: usize,
    model: &str,
    workspace: &str,
    session_id: &str,
    input: &str,
    cursor_pos: usize,
) {
    let size = f.area();

    // Main layout: status bar (1) + conversation (flex) + input (3)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),   // Status bar
            Constraint::Min(5),     // Conversation pane
            Constraint::Length(3),  // Input area
        ])
        .split(size);

    render_status_bar(f, chunks[0], model, workspace, session_id);
    render_conversation(f, chunks[1], messages, streaming_text, current_tool, is_streaming, scroll_offset);
    render_input(f, chunks[2], input, cursor_pos);
}

/// Render the status bar at the top
fn render_status_bar(f: &mut Frame, area: Rect, model: &str, workspace: &str, session_id: &str) {
    let session_short = if session_id.len() >= 8 {
        &session_id[..8]
    } else {
        session_id
    };

    let workspace_short = truncate_path(workspace, 20);

    let status_text = format!(
        " Orion Agent | model: {} | workspace: {} | session: {} ",
        model, workspace_short, session_short
    );

    let status = Paragraph::new(Line::from(vec![
        Span::styled(status_text, Style::default().fg(Color::White).bg(Color::DarkGray)),
    ]));

    f.render_widget(status, area);
}

/// Render the conversation pane
fn render_conversation(
    f: &mut Frame,
    area: Rect,
    messages: &[ChatMessage],
    streaming_text: &str,
    current_tool: &Option<ToolDisplay>,
    is_streaming: bool,
    scroll_offset: usize,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Render all messages
    for msg in messages {
        let msg_lines = render_message(msg, area.width.saturating_sub(2) as usize);
        lines.extend(msg_lines);
        lines.push(Line::from("")); // spacing
    }

    // Render current tool if executing
    if let Some(ref tool) = current_tool {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("[{}] ", tool.name), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(tool.input_summary.clone(), Style::default().fg(Color::Gray)),
        ]));
        if tool.output.is_some() {
            lines.push(Line::from(vec![
                Span::styled("    Output: ", Style::default().fg(Color::DarkGray)),
                Span::styled(tool.output.clone().unwrap(), Style::default().fg(Color::Gray)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("    Running...", Style::default().fg(Color::Yellow)),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Render streaming text
    if is_streaming && !streaming_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  Assistant: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]));
        let stream_lines = render_markdown(streaming_text, area.width.saturating_sub(4) as usize);
        lines.extend(stream_lines);
        lines.push(Line::from(""));
    } else if is_streaming {
        lines.push(Line::from(vec![
            Span::styled("  Assistant: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("...", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));
    }

    // Apply scroll offset
    let total_lines = lines.len();
    let visible_height = area.height as usize;
    let start = total_lines.saturating_sub(visible_height + scroll_offset);
    let end = start.saturating_add(visible_height).min(total_lines);
    let visible_lines: Vec<Line> = lines.into_iter().skip(start).take(end - start).collect();

    let conversation = Paragraph::new(visible_lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });

    f.render_widget(conversation, area);
}

/// Render the input area at the bottom
fn render_input(f: &mut Frame, area: Rect, input: &str, cursor_pos: usize) {
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" orion> ");

    // Show input with cursor
    let mut cursor_spans = Vec::new();
    let input_chars: Vec<char> = input.chars().collect();

    if cursor_pos < input_chars.len() {
        // Before cursor
        if cursor_pos > 0 {
            let before: String = input_chars[..cursor_pos].iter().collect();
            cursor_spans.push(Span::raw(before));
        }
        // At cursor (highlighted)
        let at_cursor: String = input_chars[cursor_pos..=cursor_pos].iter().collect();
        cursor_spans.push(Span::styled(
            at_cursor,
            Style::default().bg(Color::White).fg(Color::Black),
        ));
        // After cursor
        if cursor_pos + 1 < input_chars.len() {
            let after: String = input_chars[cursor_pos + 1..].iter().collect();
            cursor_spans.push(Span::raw(after));
        }
    } else {
        // Cursor at end
        cursor_spans.push(Span::raw(input.to_string()));
        cursor_spans.push(Span::styled(" ", Style::default().bg(Color::White)));
    }

    let input_line = Paragraph::new(Line::from(cursor_spans))
        .block(input_block)
        .wrap(Wrap { trim: false });

    f.render_widget(input_line, area);
}

// ============================================================
//  Message Rendering
// ============================================================

/// Render a single chat message into display lines
fn render_message(msg: &ChatMessage, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let indent = "  ";

    match msg {
        ChatMessage::User(text) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{}User: ", indent), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(text.clone(), Style::default().fg(Color::White)),
            ]));
        }
        ChatMessage::Assistant(text) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{}Assistant: ", indent), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]));
            let md_lines = render_markdown(text, width.saturating_sub(4));
            for md_line in md_lines {
                let mut combined = vec![Span::styled(format!("{}  ", indent), Style::default())];
                combined.extend(md_line.spans);
                lines.push(Line::from(combined));
            }
        }
        ChatMessage::ToolCall { name, input, output, is_error } => {
            // Tool call box
            let status_icon = if *is_error { "x" } else { "+" };
            let status_color = if *is_error { Color::Red } else { Color::Green };

            lines.push(Line::from(vec![
                Span::styled(format!("{}  [{} ", indent, status_icon), Style::default().fg(status_color)),
                Span::styled(name.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("] ", Style::default().fg(status_color)),
                Span::styled(truncate_str(input, 50), Style::default().fg(Color::Gray)),
            ]));

            // Tool output (indented, dimmed)
            if !output.is_empty() {
                let output_lines: Vec<&str> = output.lines().take(5).collect();
                for ol in output_lines {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}    | ", indent), Style::default().fg(Color::DarkGray)),
                        Span::styled(truncate_str(ol, width.saturating_sub(8)), Style::default().fg(Color::DarkGray)),
                    ]));
                }
                if output.lines().count() > 5 {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}    | ... ({} more lines)", indent, output.lines().count() - 5), Style::default().fg(Color::DarkGray)),
                    ]));
                }
            }
        }
        ChatMessage::System(text) => {
            lines.push(Line::from(vec![
                Span::styled(format!("{}* ", indent), Style::default().fg(Color::Magenta)),
                Span::styled(text.clone(), Style::default().fg(Color::Magenta)),
            ]));
        }
    }

    lines
}

/// Count the number of display lines a message will take
fn message_line_count(msg: &ChatMessage) -> usize {
    match msg {
        ChatMessage::User(text) => {
            1 + count_display_lines(text, 70)
        }
        ChatMessage::Assistant(text) => {
            1 + count_display_lines(text, 70)
        }
        ChatMessage::ToolCall { output, .. } => {
            1 + output.lines().take(5).count() + if output.lines().count() > 5 { 1 } else { 0 }
        }
        ChatMessage::System(text) => {
            1 + count_display_lines(text, 70)
        }
    }
}

// ============================================================
//  Basic Markdown Rendering
// ============================================================

/// Render basic markdown to styled lines
///
/// Supports:
/// - Headers (# ## ###)
/// - Bold (**text**)
/// - Inline code (`code`)
/// - Code blocks (``` ... ```)
/// - Lists (- item)
fn render_markdown(text: &str, _width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Code block toggle
        if trimmed.starts_with("```") {
            if in_code_block {
                // End of code block
                lines.push(Line::from(vec![
                    Span::styled("  ```", Style::default().fg(Color::DarkGray)),
                ]));
                in_code_block = false;
                code_lang.clear();
            } else {
                // Start of code block
                code_lang = trimmed[3..].trim().to_string();
                let lang_display = if code_lang.is_empty() { "code" } else { &code_lang };
                lines.push(Line::from(vec![
                    Span::styled(format!("  ``` {}", lang_display), Style::default().fg(Color::DarkGray)),
                ]));
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            // Inside code block - render as monospace/dimmed
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", line), Style::default().fg(Color::Yellow)),
            ]));
            continue;
        }

        // Headers
        if trimmed.starts_with("### ") {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", &trimmed[4..]), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            ]));
            continue;
        }
        if trimmed.starts_with("## ") {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", &trimmed[3..]), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            ]));
            continue;
        }
        if trimmed.starts_with("# ") {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}", &trimmed[2..]), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            ]));
            continue;
        }

        // List items
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            lines.push(Line::from(vec![
                Span::styled("    * ", Style::default().fg(Color::Blue)),
                Span::raw(trimmed[2..].to_string()),
            ]));
            continue;
        }

        // Numbered lists
        if trimmed.len() > 2 && trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            if let Some(dot_pos) = trimmed.find(". ") {
                if dot_pos < 4 {
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {}", &trimmed[..=dot_pos]), Style::default().fg(Color::Blue)),
                        Span::raw(trimmed[dot_pos + 2..].to_string()),
                    ]));
                    continue;
                }
            }
        }

        // Regular line - apply inline formatting
        let styled_line = parse_inline_markdown(trimmed);
        let mut combined = vec![Span::styled("  ", Style::default())];
        combined.extend(styled_line);
        lines.push(Line::from(combined));
    }

    lines
}

/// Parse inline markdown elements (bold, code)
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Look for inline code
        if let Some(code_start) = remaining.find('`') {
            // Add text before code
            if code_start > 0 {
                let before = &remaining[..code_start];
                spans.extend(parse_bold(before));
            }

            // Find closing backtick
            let after_open = &remaining[code_start + 1..];
            if let Some(code_end) = after_open.find('`') {
                let code = &after_open[..code_end];
                spans.push(Span::styled(
                    code.to_string(),
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                ));
                remaining = &after_open[code_end + 1..];
            } else {
                // No closing backtick, treat as regular text
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
        } else {
            // No more code blocks, parse bold
            spans.extend(parse_bold(remaining));
            break;
        }
    }

    spans
}

/// Parse bold markdown (**text**)
fn parse_bold(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if let Some(bold_start) = remaining.find("**") {
            // Add text before bold
            if bold_start > 0 {
                spans.push(Span::raw(remaining[..bold_start].to_string()));
            }

            // Find closing **
            let after_open = &remaining[bold_start + 2..];
            if let Some(bold_end) = after_open.find("**") {
                let bold_text = &after_open[..bold_end];
                spans.push(Span::styled(
                    bold_text.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                remaining = &after_open[bold_end + 2..];
            } else {
                // No closing **, treat as regular text
                spans.push(Span::raw(remaining.to_string()));
                break;
            }
        } else {
            spans.push(Span::raw(remaining.to_string()));
            break;
        }
    }

    spans
}

// ============================================================
//  Utility Functions
// ============================================================

/// Count the number of display lines for text with a given width
fn count_display_lines(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.lines().count();
    }
    text.lines()
        .map(|line| {
            let len = line.chars().count();
            (len + width - 1) / width // ceiling division
        })
        .sum::<usize>()
        .max(1)
}

/// Truncate a string to a max length, adding "..." if truncated
fn truncate_str(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Truncate a path for display
fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        let truncated = &path[path.len() - max_len + 3..];
        format!("...{}", truncated)
    }
}

/// Truncate tool input JSON for display
fn truncate_input(input: &serde_json::Value) -> String {
    let s = input.to_string();
    if s.len() > 50 {
        format!("{}...", &s[..47])
    } else {
        s
    }
}
