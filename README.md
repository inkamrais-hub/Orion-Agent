[English](README_EN.md) | 中文

# Orion Agent Framework

一个用 Rust 构建的模块化 AI Agent 框架。目标是提供一个可私有化部署、高性能、安全可控的 Agent 平台。

> 当前状态：早期开发中 (v0.1.0)。核心引擎可用，平台层正在建设。

## 它能做什么

**已实现（可正常工作）：**

- **Agent 对话** — 基于 `Agent::builder()` 创建 Agent，支持 `chat()` 单次对话和 `chat_stream()` 流式对话
- **多模型支持** — OpenAI 兼容 API（DeepSeek/Qwen/Ollama 等）+ Anthropic Claude，配置文件切换
- **17+ 内置工具** — 文件读写、代码编辑、Shell 执行、正则搜索、符号搜索、Web 搜索、子 Agent、A2A 通信等
- **MCP 协议** — 通过 stdio 连接任意 MCP Server，连接池复用（同名 server 只启动一个子进程）
- **上下文管理** — 7 种压缩策略（Micro/Snip/Chunked/Auto/Reactive/Collapse），断路器防连续失败
- **三层缓存** — L1 工具结果缓存 + L2 上下文快照缓存 + 文件缓存（mtime 感知）
- **安全护栏** — 权限 ACL + Token 预算 + Bash 风险分级（Safe→Critical）+ Hook 拦截器
- **REST API + SSE** — `POST /api/chat` 流式输出 Agent 事件，`/api/agents` CRUD 配置
- **Session 管理** — SQLite 持久化，JSONL 转录，文件回滚快照
- **审计日志** — 9 种事件类型，敏感信息自动脱敏

**部分实现（能跑但不完善）：**

- **多 Agent 编排** — Coordinator 能通过 LLM 拆解任务为 DAG 并顺序执行子任务，但并行/协作模式未实现
- **REPL 交互** — 17 个斜杠命令（/model、/think、/think-level 等），但 UI 比较简陋
- **代码索引** — 增量索引、符号搜索、调用链分析，但只支持部分语言

**未实现（只有空壳或设计）：**

- Web UI 前端
- 多租户 / RBAC
- Docker 沙箱执行模式
- 工具动态发现（减少 Token 消耗）

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 语言 | Rust 2021 | 主体框架 |
| 异步运行时 | Tokio (full features) | 异步 I/O、任务调度 |
| HTTP 客户端 | reqwest (可选) | OpenAI 兼容 API 调用 |
| Web 框架 | Axum (可选) | REST API + SSE |
| 数据库 | rusqlite (bundled) | Session/Agent 配置持久化 |
| 缓存 | moka + DashMap | 高性能并发缓存 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 错误处理 | thiserror | 统一错误类型 |
| 序列化 | serde + serde_json + serde_yaml | 配置和数据交换 |

## 快速开始

```bash
# 克隆
git clone https://github.com/inkamrais-hub/Orion-Agent.git
cd Orion-Agent

# 配置 API Key
cp .env.example .env
# 编辑 .env，填入你的 LLM_API_KEY

# CLI 模式
cargo run

# REST API 模式
cargo run --features api -- serve

# 一次性任务
cargo run -- --onlyrun "用 Rust 写一个 HTTP server"
```

## 项目结构

```
src/
├── core/               # 核心引擎
│   ├── agent.rs        # Agent 结构体 + Builder + AgentEvent
│   ├── loop.rs         # 核心执行循环（流式 LLM 调用 + 工具执行）
│   ├── provider.rs     # Provider trait（LLM 抽象层）
│   ├── providers/      # OpenAI 兼容 + Anthropic 实现
│   ├── context.rs      # 上下文管理 + 7 种压缩策略
│   ├── cache.rs        # 三层缓存系统
│   ├── guardrail.rs    # 权限 + 预算护栏
│   ├── hooks.rs        # YAML 配置式 Hook 拦截器
│   ├── execpolicy.rs   # 命令执行白名单策略
│   ├── goal.rs         # Goal 状态机 + 自动续命
│   ├── workspace.rs    # 工作区安全守卫
│   └── audit.rs        # 底层审计日志
├── tools/              # 工具系统
│   ├── mod.rs          # Read/Write/Bash 核心工具
│   ├── registry.rs     # 工具注册表 + AOP 路径拦截
│   ├── mcp.rs          # MCP 客户端 + 连接池
│   ├── edit.rs         # 精确字符串替换
│   ├── grep_tool.rs    # 正则内容搜索
│   ├── glob_tool.rs    # 文件名搜索
│   ├── multi_shell.rs  # 多终端工具
│   ├── web_search.rs   # Web 搜索
│   ├── agent_tool.rs   # 子 Agent 创建
│   └── code_intelligence/  # 符号搜索、调用链、项目概览
├── agent/              # Agent 运行时
│   ├── runtime.rs      # AgentRuntime 数据容器
│   ├── registry.rs     # Agent 注册表（A2A 通信）
│   ├── store.rs        # Agent 配置 SQLite 持久化 + 回滚快照
│   ├── lanes.rs        # 执行车道（防资源竞争）
│   └── protocol.rs     # A2A 协议消息
├── orchestrator/       # 多 Agent 编排
│   ├── coordinator.rs  # Coordinator（LLM 任务拆解 + DAG 调度）
│   ├── plan.rs         # TaskPlan（依赖解析 + 状态追踪）
│   └── worker.rs       # Worker（子任务执行）
├── session/            # Session 管理
│   ├── store.rs        # SQLite 持久化（4 张表 + 索引）
│   ├── manager.rs      # JSONL 转录 + JSON 索引
│   ├── memory.rs       # 跨 Session 记忆系统
│   ├── files.rs        # 目录结构管理（软删除/恢复）
│   └── rollout.rs      # JSONL 事件流（不可变审计）
├── api/                # REST API（feature-gated）
├── cli/                # REPL + 命令处理
├── gateway/            # 入口路由
├── config.rs           # YAML 配置 + 环境变量替换
├── model/              # 模型注册表
├── audit/              # 高层审计日志管理
├── logging/            # 日志子系统 + 敏感信息脱敏
└── index/              # 代码索引引擎
```

## 架构设计

核心执行流程：

```
用户输入
  ↓
Agent::chat_stream(input)
  ↓
run_simple_loop()
  ├── Provider.stream() → 流式 LLM 调用
  ├── 工具执行（并行只读 / 串行写入）
  │   ├── ExecPolicy 命令白名单检查
  │   ├── GuardrailChain 护栏检查
  │   ├── HookEngine before/after 拦截
  │   ├── ToolRegistry AOP 路径规范化
  │   └── StepObserver 重试/Replan 判断
  ├── ContextManager 上下文压缩
  ├── GlobalCache 缓存命中检查
  ├── AuditLogger 审计记录
  └── RolloutRecorder 事件流记录
  ↓
AgentEvent → SSE 流式输出
```

## 当前不足（诚实说明）

1. **没有 Web UI** — 只有 REST API，前端需要自己搭
2. **编排能力有限** — Coordinator 只支持顺序执行，并行/协作模式是空壳
3. **工具动态发现未实现** — 每次对话都发送所有工具的完整 Schema，Token 消耗较高
4. **测试覆盖不足** — 有 68 个单元测试，但缺乏集成测试和端到端测试
5. **文档欠缺** — 代码注释较多，但缺乏 API 文档和使用教程
6. **部分模块是空壳** — `src/events/`、`src/plugins/` 目录为空

## 路线图

- [ ] Web UI 前端（React/Vue）
- [ ] MCP 工具动态发现（减少初始 Token 消耗）
- [ ] Coordinator 并行执行模式
- [ ] Docker 沙箱执行
- [ ] 更完善的测试覆盖
- [ ] API 文档 (OpenAPI)

## 许可证

AGPL-3.0-only
