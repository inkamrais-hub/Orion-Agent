English | [中文](README.md)

# Orion Agent Framework

A modular AI Agent framework built in Rust. Privately deployable, high-performance, security-controllable.

> Version: v0.1.0-beta. Core engine + CLI + REST API fully operational.

## Core Capabilities

**Agent Conversations** — `Agent::builder()` to create agents, `chat()` for single-turn / `chat_stream()` for SSE streaming. Three execution modes: Assist (confirm each step), Auto (safe tools auto-execute), Plan (text-only, no tools).

**Multi-Model Support** — OpenAI-compatible API (DeepSeek v4-flash/v4-pro, Qwen, Ollama, etc.) + Anthropic Claude. Switch models via `models[]` in config. Supports thinking mode (reasoning chain pass-through).

**17+ Built-in Tools** — File read/write, code editing (diff+rollback), shell execution (multi-terminal isolation), regex search, symbol search (cross-language AST), web search, nested sub-agents, A2A cross-process messaging, MCP protocol for external tools.

**Unified Security Model** — PermissionBroker as single decision point: ExecPolicy (command whitelist) + GuardrailChain (ACL + token budget) + orionignore (sensitive file blocking). Bash commands rated on a 6-level risk scale (Safe → Critical).

**REST API + SSE** — Axum framework, `POST /api/chat` streaming agent events, `/api/agents` CRUD, `/api/sessions/{id}/rollback`. Built-in API Key auth middleware + per-IP rate limiter.

**UnifiedStore** — Single SQLite database replacing 3 legacy backends (JSONL + AgentStore + SessionStore). 6 tables covering agent configs, session metadata, transcripts, tool call records, file snapshots. SessionBackend async trait for future PostgreSQL migration.

**Prompt Caching** — Three-section PromptBuilder (Static → Tool → Dynamic) maximizing prefix-match cache hits. DeepSeek automatic prefix caching, Anthropic cache_control.

**Multi-Agent Orchestration** — Coordinator uses LLM to decompose tasks into DAGs, sequential execution with automatic retry on failure. MapReduce with token tracking.

**Context Management** — 7 compaction strategies (Micro/Snip/Chunked/Auto/Reactive/Collapse), circuit breaker for consecutive failures. Three-layer cache (L1 tool results + L2 context snapshots + file mtime-aware).

**Project-Scoped Memory** — Cross-session memory isolated by project, with time-decay auto-pruning.

## Tech Stack

| Layer | Technology | Notes |
|---|---|---|
| Language | Rust 2021 | Core framework |
| Async Runtime | Tokio (full) | Async I/O, task scheduling |
| HTTP Client | reqwest (optional) | LLM API calls + SSE streaming |
| Web Framework | Axum 0.7 (optional) | REST API + SSE + middleware |
| Database | rusqlite (bundled) | UnifiedStore persistence |
| Cache | moka + DashMap | High-performance concurrent cache |
| Logging | tracing + tracing-subscriber | Structured logging + JSON output |
| Error Handling | thiserror | 10 unified error variants |
| Serialization | serde + serde_json + serde_yaml | Config and data exchange |

## Quick Start

```bash
git clone https://github.com/inkamrais-hub/Orion-Agent.git
cd Orion-Agent

# Set API Key (choose one)
# Option 1: .env file
cp .env.example .env
# Edit .env, add LLM_API_KEY=sk-xxx

# Option 2: Config file ~/.orion/config.yaml

# Run
cargo run                          # CLI interactive mode
cargo run --features api -- serve  # REST API mode
cargo run -- --onlyrun "task"      # One-shot execution
```

### Config Example (~/.orion/config.yaml)

```yaml
default_model: deepseek-chat

models:
  - name: deepseek-chat
    endpoint: https://api.deepseek.com
    api_key: sk-your-key-here
    max_tokens: 4096
    max_input_tokens: 128000
    thinking: false
    prompt_cache: true
```

## Project Structure

```
src/
├── core/               # Core engine
│   ├── agent.rs        # Agent struct + Builder + AgentEvent
│   ├── loop.rs         # Core execution loop (streaming LLM + tool exec)
│   ├── provider.rs     # Provider trait (LLM abstraction)
│   ├── providers/      # OpenAI-compat + Anthropic implementations
│   ├── prompt.rs       # Three-section PromptBuilder
│   ├── permission_broker.rs  # Unified security decision point
│   ├── exec_mode.rs    # Execution modes (Assist/Auto/Plan)
│   ├── execpolicy.rs   # Command whitelist policy
│   ├── guardrail.rs    # ACL + token budget guardrails
│   ├── hooks.rs        # YAML-configured hook interceptors
│   ├── context.rs      # Context management + 7 compaction strategies
│   ├── cache.rs        # Three-layer cache system
│   ├── goal.rs         # Goal state machine + auto-continue
│   ├── orionignore.rs  # Sensitive file detection + ignore rules
│   └── audit.rs        # Low-level audit logging
├── tools/              # Tool system
│   ├── registry.rs     # Tool registry + AOP path interception
│   ├── mcp.rs          # MCP client + connection pooling
│   ├── multi_shell.rs  # Multi-terminal isolated execution
│   ├── web_search.rs   # Web search (multilingual)
│   ├── agent_tool.rs   # Sub-agent creation + A2A messaging
│   ├── category.rs     # Tool category registry (lazy loading)
│   └── code_intelligence/  # Symbol search, call chains, dep graph
├── agent/              # Inter-agent communication
│   ├── protocol.rs     # A2A protocol (correlation_id + TaskLifecycle)
│   ├── registry.rs     # Agent registry
│   ├── runtime.rs      # AgentMessage + MessageHandler trait
│   └── lanes.rs        # Lane constants + LaneToken
├── orchestrator/       # Multi-agent orchestration
│   ├── coordinator.rs  # LLM-based DAG decomposition + retry
│   ├── plan.rs         # TaskPlan (dependency resolution)
│   ├── map_reduce.rs   # MapReduce with token tracking
│   └── worker.rs       # Subtask execution
├── session/            # Persistence
│   ├── unified.rs      # UnifiedStore (single SQLite, 6 tables)
│   ├── backend.rs      # SessionBackend async trait (16 methods)
│   ├── memory.rs       # Project-scoped memory (decay + pruning)
│   ├── store.rs        # Session SQLite (turn-level records)
│   ├── rollout.rs      # JSONL event stream (immutable audit)
│   └── files.rs        # Directory structure management
├── api/                # REST API + auth + rate limiting (feature-gated)
├── cli/                # CLI interactive (chat/ submodule)
├── gateway/            # Entry routing + command system
├── config.rs           # YAML config + ${ENV_VAR} substitution
├── model/              # Model config + router
├── audit/              # High-level audit management
├── logging/            # Logging subsystem + PII redaction
├── index/              # Incremental code indexing engine
└── ui/                 # CLI UI components (progress, reports)
```

## Architecture Flow

```
User Input
  ↓
Gateway → CLI / WebUI / --onlyrun
  ↓
Agent::chat_stream(input)
  ↓
run_simple_loop()
  ├── Provider.stream() → Streaming LLM call (SSE)
  ├── PermissionBroker → Security decision
  │   ├── ExecPolicy command whitelist
  │   ├── GuardrailChain ACL + budget
  │   └── orionignore sensitive file blocking
  ├── Tool Execution (read-only parallel / write serial)
  │   ├── ToolRegistry AOP path interception
  │   ├── HookEngine before/after hooks
  │   └── StepObserver retry/replan detection
  ├── PromptBuilder → Three-section prompt (cache-friendly)
  ├── ContextManager context compaction
  ├── GlobalCache cache hit check
  └── UnifiedStore persistence (transcripts + snapshots)
  ↓
AgentEvent → SSE / CLI streaming output
```

## REST API

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/health` | Health check |
| GET | `/api/agents` | List all agent configs |
| POST | `/api/agents` | Create agent config |
| GET | `/api/agents/{id}` | Get agent config |
| PUT | `/api/agents/{id}` | Update agent config |
| DELETE | `/api/agents/{id}` | Delete agent config |
| GET | `/api/tools` | List available tools |
| POST | `/api/chat` | SSE streaming conversation |
| POST | `/api/sessions/{id}/rollback` | Session rollback |

## Code Example

```rust
use orion_agent::prelude::*;
use orion_agent::core::providers::openai_compat::OpenAICompatProvider;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let provider = Arc::new(OpenAICompatProvider::from_env());

    let agent = Agent::builder()
        .name("my-agent")
        .model("deepseek-v4-flash")
        .system_prompt("You are a Rust expert.")
        .provider(provider)
        .max_turns(10)
        .build()
        .unwrap();

    let reply = agent.chat("What is ownership?").await.unwrap();
    println!("{}", reply);
}
```

## Testing

```bash
cargo test                              # All unit tests (149 tests)
cargo test --test deepseek_integration  # DeepSeek API integration tests
cargo clippy --all-targets              # Zero clippy warnings
```

## Known Limitations

- **No Web UI frontend** — REST API is complete, frontend must be built separately
- **Orchestration** — Coordinator supports sequential DAG only; parallel/collaborative modes are planned
- **Tool discovery** — `lazy_tools()` implements meta-tool mode but is not yet wired into CLI/API paths
- **Code indexing** — Supports Rust/Python/JavaScript/Go/TypeScript; other languages need extension
- **Single process** — No horizontal scaling (SQLite limitation; SessionBackend trait reserved for future)

## Roadmap

- [ ] Web UI frontend
- [ ] Coordinator parallel execution + collaborative mode
- [ ] Docker sandbox execution
- [ ] PostgreSQL SessionBackend implementation
- [ ] OpenAPI documentation
- [ ] Multi-tenancy / RBAC

## License

AGPL-3.0-only
