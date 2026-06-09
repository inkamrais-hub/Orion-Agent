English | [中文](README.md)

# Orion Agent Framework

A modular AI Agent framework built in Rust. The goal is to provide a privately deployable, high-performance, and security-controllable Agent platform.

> Current status: Early development (v0.1.0). Core engine is functional, platform layer is under construction.

## What It Can Do

**Implemented (working):**

- **Agent Chat** — Create agents via `Agent::builder()`, supports `chat()` for single-turn and `chat_stream()` for streaming
- **Multi-model Support** — OpenAI-compatible API (DeepSeek/Qwen/Ollama etc.) + Anthropic Claude, switchable via config
- **17+ Built-in Tools** — File read/write, code editing, shell execution, regex search, symbol search, web search, sub-agent, A2A communication, etc.
- **MCP Protocol** — Connect to any MCP Server via stdio, with connection pooling (one subprocess per server name)
- **Context Management** — 7 compaction strategies (Micro/Snip/Chunked/Auto/Reactive/Collapse), circuit breaker for consecutive failures
- **Three-layer Cache** — L1 tool result cache + L2 context snapshot cache + file cache (mtime-aware)
- **Safety Guardrails** — Permission ACL + Token budget + Bash risk classification (Safe→Critical) + Hook interceptors
- **REST API + SSE** — `POST /api/chat` streams Agent events, `/api/agents` CRUD for configuration
- **Session Management** — SQLite persistence, JSONL transcription, file rollback snapshots
- **Audit Logging** — 9 event types, automatic sensitive information redaction

**Partially Implemented (runs but incomplete):**

- **Multi-Agent Orchestration** — Coordinator can decompose tasks into DAG via LLM and execute subtasks sequentially, but parallel/collaborative modes are not implemented
- **REPL Interaction** — 17 slash commands (/model, /think, /think-level, etc.), but the UI is minimal
- **Code Indexing** — Incremental indexing, symbol search, call chain analysis, but only for some languages

**Not Implemented (stubs or design only):**

- Web UI frontend
- Multi-tenancy / RBAC
- Docker sandbox execution mode
- Dynamic tool discovery (for reducing Token consumption)

## Tech Stack

| Layer | Technology | Description |
|---|---|---|
| Language | Rust 2021 | Main framework |
| Async Runtime | Tokio (full features) | Async I/O, task scheduling |
| HTTP Client | reqwest (optional) | OpenAI-compatible API calls |
| Web Framework | Axum (optional) | REST API + SSE |
| Database | rusqlite (bundled) | Session/Agent config persistence |
| Cache | moka + DashMap | High-performance concurrent cache |
| Logging | tracing + tracing-subscriber | Structured logging |
| Error Handling | thiserror | Unified error types |
| Serialization | serde + serde_json + serde_yaml | Config and data exchange |

## Quick Start

```bash
# Clone
git clone https://github.com/inkamrais-hub/Orion-Agent.git
cd Orion-Agent

# Configure API Key
cp .env.example .env
# Edit .env, fill in your LLM_API_KEY

# CLI mode
cargo run

# REST API mode
cargo run --features api -- serve

# One-shot task
cargo run -- --onlyrun "Write an HTTP server in Rust"
```

## Project Structure

```
src/
├── core/               # Core engine
│   ├── agent.rs        # Agent struct + Builder + AgentEvent
│   ├── loop.rs         # Core execution loop (streaming LLM + tool execution)
│   ├── provider.rs     # Provider trait (LLM abstraction)
│   ├── providers/      # OpenAI-compatible + Anthropic implementations
│   ├── context.rs      # Context management + 7 compaction strategies
│   ├── cache.rs        # Three-layer cache system
│   ├── guardrail.rs    # Permission + budget guardrails
│   ├── hooks.rs        # YAML-configured Hook interceptors
│   ├── execpolicy.rs   # Command execution whitelist policy
│   ├── goal.rs         # Goal state machine + auto-steering
│   ├── workspace.rs    # Workspace security guard
│   └── audit.rs        # Low-level audit logging
├── tools/              # Tool system
│   ├── mod.rs          # Read/Write/Bash core tools
│   ├── registry.rs     # Tool registry + AOP path interception
│   ├── mcp.rs          # MCP client + connection pool
│   ├── edit.rs         # Precise string replacement
│   ├── grep_tool.rs    # Regex content search
│   ├── glob_tool.rs    # Filename search
│   ├── multi_shell.rs  # Multi-terminal tool
│   ├── web_search.rs   # Web search
│   ├── agent_tool.rs   # Sub-agent creation
│   └── code_intelligence/  # Symbol search, call chain, project map
├── agent/              # Agent runtime
│   ├── runtime.rs      # AgentRuntime data container
│   ├── registry.rs     # Agent registry (A2A communication)
│   ├── store.rs        # Agent config SQLite persistence + rollback snapshots
│   ├── lanes.rs        # Execution lanes (resource contention prevention)
│   └── protocol.rs     # A2A protocol messages
├── orchestrator/       # Multi-agent orchestration
│   ├── coordinator.rs  # Coordinator (LLM task decomposition + DAG scheduling)
│   ├── plan.rs         # TaskPlan (dependency resolution + state tracking)
│   └── worker.rs       # Worker (subtask execution)
├── session/            # Session management
│   ├── store.rs        # SQLite persistence (4 tables + indexes)
│   ├── manager.rs      # JSONL transcription + JSON indexing
│   ├── memory.rs       # Cross-session memory system
│   ├── files.rs        # Directory structure management (soft delete/restore)
│   └── rollout.rs      # JSONL event stream (immutable audit)
├── api/                # REST API (feature-gated)
├── cli/                # REPL + command handling
├── gateway/            # Entry routing
├── config.rs           # YAML config + environment variable substitution
├── model/              # Model registry
├── audit/              # High-level audit log management
├── logging/            # Logging subsystem + sensitive info redaction
└── index/              # Code indexing engine
```

## Architecture

Core execution flow:

```
User Input
  ↓
Agent::chat_stream(input)
  ↓
run_simple_loop()
  ├── Provider.stream() → Streaming LLM call
  ├── Tool execution (parallel readonly / serial write)
  │   ├── ExecPolicy command whitelist check
  │   ├── GuardrailChain guardrail check
  │   ├── HookEngine before/after interception
  │   ├── ToolRegistry AOP path normalization
  │   └── StepObserver retry/replan judgment
  ├── ContextManager context compression
  ├── GlobalCache cache hit check
  ├── AuditLogger audit recording
  └── RolloutRecorder event stream recording
  ↓
AgentEvent → SSE streaming output
```

## Current Limitations (Honest Assessment)

1. **No Web UI** — Only REST API available, frontend needs to be built separately
2. **Limited Orchestration** — Coordinator only supports sequential execution, parallel/collaborative modes are stubs
3. **No Dynamic Tool Discovery** — Every conversation sends full schemas for all tools, resulting in higher Token consumption
4. **Insufficient Test Coverage** — 68 unit tests exist, but lacks integration and end-to-end tests
5. **Documentation Gaps** — Code has comments, but lacks API documentation and usage tutorials
6. **Some Modules Are Stubs** — `src/events/`, `src/plugins/` directories are empty

## Roadmap

- [ ] Web UI frontend (React/Vue)
- [ ] MCP dynamic tool discovery (reduce initial Token consumption)
- [ ] Coordinator parallel execution mode
- [ ] Docker sandbox execution
- [ ] Better test coverage
- [ ] API documentation (OpenAPI)

## License

AGPL-3.0-only
