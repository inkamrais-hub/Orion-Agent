[English](README_EN.md) | 中文

# Orion Agent Framework

一个用 Rust 构建的模块化 AI Agent 框架。可私有化部署、高性能、安全可控。

> 当前版本：v0.1.0-beta。核心引擎 + CLI + REST API 完整可用。

## 核心能力

**Agent 对话** — `Agent::builder()` 创建 Agent，`chat()` 单次对话 / `chat_stream()` SSE 流式对话。三种执行模式：Assist（每步确认）、Auto（安全工具自动执行）、Plan（只规划不执行）。

**多模型支持** — OpenAI 兼容 API（DeepSeek v4-flash/v4-pro、Qwen、Ollama 等）+ Anthropic Claude。配置文件中 `models[]` 列表一键切换，支持 thinking mode（推理链回传）。

**17+ 内置工具** — 文件读写、代码编辑（diff+rollback）、Shell 执行（多终端隔离）、正则搜索、符号搜索（跨语言 AST）、Web 搜索、子 Agent 嵌套、A2A 跨进程通信、MCP 协议接入外部工具。

**统一安全模型** — PermissionBroker 统一决策点，整合 ExecPolicy（命令白名单）+ GuardrailChain（ACL + Token 预算）+ orionignore（敏感文件拦截）。Bash 命令六级风险分级（Safe → Critical）。

**REST API + SSE** — Axum 框架，`POST /api/chat` 流式输出 Agent 事件，`/api/agents` CRUD 配置，`/api/sessions/{id}/rollback` 一键回滚。内置 API Key 认证中间件 + per-IP 限流中间件。

**UnifiedStore 存储** — 单一 SQLite 数据库替代原有的 3 套存储（JSONL + AgentStore + SessionStore），6 张表覆盖 Agent 配置、Session 元数据、对话转录、工具调用记录、文件快照。SessionBackend async trait 抽象，未来可换 PostgreSQL。

**Prompt 缓存** — 三段式 Prompt 构建器（Static → Tool → Dynamic），最大化 prefix-match 缓存命中率。DeepSeek 自动前缀缓存、Anthropic cache_control。

**多 Agent 编排** — Coordinator 通过 LLM 拆解任务为 DAG，顺序执行子任务并自动重试失败项。MapReduce 支持 token 追踪。

**上下文管理** — 7 种压缩策略（Micro/Snip/Chunked/Auto/Reactive/Collapse），断路器防连续失败。三层缓存（L1 工具结果 + L2 上下文快照 + 文件 mtime 感知）。

**项目维度记忆** — 跨 Session 记忆按项目隔离，带时间衰减自动清理。

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 语言 | Rust 2021 | 主体框架 |
| 异步运行时 | Tokio (full) | 异步 I/O、任务调度 |
| HTTP 客户端 | reqwest (可选) | LLM API 调用 + SSE 流式 |
| Web 框架 | Axum 0.7 (可选) | REST API + SSE + 中间件 |
| 数据库 | rusqlite (bundled) | UnifiedStore 统一存储 |
| 缓存 | moka + DashMap | 高性能并发缓存 |
| 日志 | tracing + tracing-subscriber | 结构化日志 + JSON 输出 |
| 错误处理 | thiserror | 10 种统一错误变体 |
| 序列化 | serde + serde_json + serde_yaml | 配置和数据交换 |

## 快速开始

```bash
# 克隆
git clone https://github.com/inkamrais-hub/Orion-Agent.git
cd Orion-Agent

# 配置 API Key (二选一)
# 方式 1: .env 文件
cp .env.example .env
# 编辑 .env，填入 LLM_API_KEY=sk-xxx

# 方式 2: 配置文件 ~/.orion/config.yaml
# 参见下方配置说明

# 运行
cargo run                         # CLI 交互模式
cargo run --features api -- serve # REST API 模式
cargo run -- --onlyrun "任务描述"  # 一次性执行
```

### 配置示例 (~/.orion/config.yaml)

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

  - name: deepseek-chat-thinking
    endpoint: https://api.deepseek.com
    api_key: sk-your-key-here
    max_tokens: 8192
    thinking: true
    reasoning_effort: high

cache:
  l1_capacity: 1024
  l2_capacity: 256
  file_cache_enabled: true
```

## 项目结构

```
src/
├── core/               # 核心引擎
│   ├── agent.rs        # Agent 结构体 + Builder + AgentEvent
│   ├── loop.rs         # 核心执行循环（流式 LLM + 工具执行）
│   ├── provider.rs     # Provider trait（LLM 抽象层）
│   ├── providers/      # OpenAI 兼容 + Anthropic 实现
│   ├── prompt.rs       # 三段式 Prompt 构建器
│   ├── permission_broker.rs  # 统一安全决策点
│   ├── exec_mode.rs    # 执行模式 (Assist/Auto/Plan)
│   ├── execpolicy.rs   # 命令执行白名单策略
│   ├── guardrail.rs    # 权限 ACL + Token 预算护栏
│   ├── hooks.rs        # YAML 配置式 Hook 拦截器
│   ├── context.rs      # 上下文管理 + 7 种压缩策略
│   ├── cache.rs        # 三层缓存系统
│   ├── goal.rs         # Goal 状态机 + 自动续命
│   ├── workspace.rs    # 工作区安全守卫
│   ├── orionignore.rs  # 敏感文件检测 + 忽略规则
│   └── audit.rs        # 底层审计日志
├── tools/              # 工具系统
│   ├── mod.rs          # Read/Write/Bash 核心工具
│   ├── registry.rs     # 工具注册表 + AOP 路径拦截
│   ├── mcp.rs          # MCP 客户端 + 连接池
│   ├── edit.rs         # 精确字符串替换
│   ├── grep_tool.rs    # 正则内容搜索
│   ├── glob_tool.rs    # 文件名搜索
│   ├── multi_shell.rs  # 多终端隔离执行
│   ├── web_search.rs   # Web 搜索（多语言优化）
│   ├── agent_tool.rs   # 子 Agent 创建 + A2A 通信
│   ├── category.rs     # 工具分类注册（延迟装载）
│   └── code_intelligence/  # 符号搜索、调用链、依赖图、项目概览
├── agent/              # Agent 间通信
│   ├── protocol.rs     # A2A 协议（correlation_id + TaskLifecycle）
│   ├── registry.rs     # Agent 注册表
│   ├── runtime.rs      # AgentMessage + MessageHandler trait
│   └── lanes.rs        # Lane 常量 + LaneToken
├── orchestrator/       # 多 Agent 编排
│   ├── coordinator.rs  # Coordinator（LLM DAG 拆解 + 调度 + 重试）
│   ├── plan.rs         # TaskPlan（依赖解析 + markdown fence 剥离）
│   ├── map_reduce.rs   # MapReduce（token 追踪）
│   └── worker.rs       # Worker（子任务执行）
├── session/            # 持久化存储
│   ├── unified.rs      # UnifiedStore（单一 SQLite，6 张表）
│   ├── backend.rs      # SessionBackend async trait（16 个方法）
│   ├── memory.rs       # 项目维度记忆（时间衰减 + 自动清理）
│   ├── store.rs        # Session SQLite（turn 级记录）
│   ├── files.rs        # 目录结构管理
│   ├── rollout.rs      # JSONL 事件流（不可变审计）
│   └── sandbox.rs      # 沙箱配置
├── api/                # REST API + 认证 + 限流（feature-gated）
├── cli/                # CLI 交互（chat/ 子模块）
├── gateway/            # 入口路由 + 命令系统
├── config.rs           # YAML 配置 + ${ENV_VAR} 替换
├── model/              # 模型配置 + 路由器
├── audit/              # 高层审计日志管理
├── logging/            # 日志子系统 + 敏感信息脱敏
├── index/              # 代码增量索引引擎
└── ui/                 # CLI UI 组件（进度条、报告）
```

## 架构流程

```
用户输入
  ↓
Gateway → CLI / WebUI / --onlyrun
  ↓
Agent::chat_stream(input)
  ↓
run_simple_loop()
  ├── Provider.stream() → 流式 LLM 调用 (SSE)
  ├── PermissionBroker → 安全决策
  │   ├── ExecPolicy 命令白名单
  │   ├── GuardrailChain ACL + 预算
  │   └── orionignore 敏感文件拦截
  ├── 工具执行（只读并行 / 写入串行）
  │   ├── ToolRegistry AOP 路径拦截
  │   ├── HookEngine before/after 拦截
  │   └── StepObserver 重试/Replan 判断
  ├── PromptBuilder → 三段式 prompt (cache-friendly)
  ├── ContextManager 上下文压缩
  ├── GlobalCache 缓存命中检查
  └── UnifiedStore 持久化 (transcript + snapshots)
  ↓
AgentEvent → SSE / CLI 流式输出
```

## REST API

启动 API 服务后可通过 HTTP 调用。支持 `X-API-Key` 或 `Authorization: Bearer` 认证。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/health` | 健康检查 |
| GET | `/api/agents` | 列出所有 Agent 配置 |
| POST | `/api/agents` | 创建 Agent 配置 |
| GET | `/api/agents/{id}` | 获取单个 Agent |
| PUT | `/api/agents/{id}` | 更新 Agent 配置 |
| DELETE | `/api/agents/{id}` | 删除 Agent 配置 |
| GET | `/api/tools` | 列出可用工具 |
| POST | `/api/chat` | SSE 流式对话 |
| POST | `/api/sessions/{id}/rollback` | Session 回滚 |

### 快速调用示例

```bash
# 健康检查
curl http://localhost:3000/api/health

# 流式对话
curl -N -X POST http://localhost:3000/api/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "用 Rust 写一个快速排序", "agent_id": "main"}'
```

## 代码示例

### 基本对话

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
        .system_prompt("你是一个 Rust 专家。")
        .provider(provider)
        .max_turns(10)
        .build()
        .unwrap();

    // 单次对话
    let reply = agent.chat("什么是 ownership?").await.unwrap();
    println!("{}", reply);
}
```

### 流式对话 + 工具

```rust
let agent = Agent::builder()
    .name("coder")
    .model("deepseek-v4-flash")
    .system_prompt("你是一个编码助手。")
    .provider(provider)
    .add_tool(orion_agent::tools::ReadFileTool)
    .add_tool(orion_agent::tools::WriteFileTool)
    .add_tool(orion_agent::tools::BashTool)
    .build()
    .unwrap();

let mut rx = agent.chat_stream("读取 Cargo.toml 并分析依赖", None)?;
while let Some(event) = rx.recv().await {
    match event {
        AgentEvent::Text(text) => print!("{}", text),
        AgentEvent::ToolStart { name, .. } => eprintln!("[tool] {}", name),
        AgentEvent::Done { message, .. } => println!("\n{}", message),
        _ => {}
    }
}
```

### 启用 Thinking Mode

```rust
let agent = Agent::builder()
    .model("deepseek-v4-flash")
    .thinking(true)
    .reasoning_effort("high")  // low / medium / high / max
    .provider(provider)
    .build()
    .unwrap();
```

## 测试

```bash
cargo test                              # 全量单元测试 (149 tests)
cargo test --test deepseek_integration  # DeepSeek API 集成测试
cargo clippy --all-targets              # 零 clippy 警告
```

## 已知限制

- **无 Web UI 前端** — REST API 完备，但前端需要自行搭建
- **编排能力** — Coordinator 只支持顺序 DAG 执行，并行/协作模式待实现
- **工具动态发现** — `lazy_tools()` 已实现元工具模式，但未在 CLI/API 路径中启用
- **代码索引** — 支持 Rust/Python/JavaScript/Go/TypeScript，其他语言需扩展
- **单进程** — 目前不支持多实例水平扩展（SQLite 限制，SessionBackend trait 已预留）

## 路线图

- [ ] Web UI 前端
- [ ] Coordinator 并行执行 + 协作模式
- [ ] Docker 沙箱执行
- [ ] PostgreSQL SessionBackend 实现
- [ ] OpenAPI 文档
- [ ] 多租户 / RBAC

## 许可证

AGPL-3.0-only
