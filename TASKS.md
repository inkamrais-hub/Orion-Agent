# Orion Agent 框架化改造 — 任务拆解

> 基于文档 b 的方向，结合源码实况的细腻拆解。
> 每个任务可独立执行，有明确验收标准。

## 状态总览 (2026-06-11)

**所有任务已完成。**

| 任务 | 状态 | 提交 |
|------|------|------|
| T1: Agent 结构体 + Builder | ✅ | 早期提交 |
| T2: AgentEvent 枚举 + 事件桥接 | ✅ | 早期提交 |
| T3: lib.rs 导出 | ✅ | 早期提交 |
| T4: execute_turn 统一 | ✅ | 早期提交 |
| T5: Gateway 去重 | ✅ | 早期提交 |
| T6: cli/commands.rs 清理 | ✅ | 已删除 |
| T7: MCP 配置集成 | ✅ | 早期提交 |
| T8: Gateway 自动 MCP | ✅ | 早期提交 |
| T9: Coordinator + TaskPlan | ✅ | 早期提交 |
| T10: 删除重复 ExecutionPlan | ✅ | 早期提交 |
| T11: GuardrailChain 集成 | ✅ | 早期提交 |
| T12: 代码清理 | ✅ | caa25d1 |

---

---

## 阶段一：P0 — Agent 结构体（核心）

### T1: 创建 Agent 结构体 + Builder

**目标**: `src/core/agent.rs`，让 Agent 成为自包含的可执行单元。

**输入（需读的文件）**:
- `src/core/loop.rs` L392-438 — SimpleLoopConfig + SimpleLoopContext + run_simple_loop 签名
- `src/agent/runtime.rs` — AgentRuntime（数据容器，无 run 方法）
- `src/cli/execute.rs` L83-213 — execute_turn 展示了如何拼装参数调用 run_simple_loop
- `src/gateway/commands.rs` — run 命令展示了另一种拼装方式

**输出**:
新建 `src/core/agent.rs`，包含：

```rust
/// Agent 配置（面向用户的简洁配置）
pub struct AgentConfig {
    pub name: String,
    pub model: String,
    pub system_prompt: String,
    pub max_turns: u64,         // 默认 20
    pub max_tool_calls: u64,    // 默认 30
    pub token_budget: u64,      // 默认 128_000
    pub thinking: bool,         // 默认 false
    pub reasoning_effort: String, // 默认 "medium"
}

/// Agent 构建器
pub struct AgentBuilder { ... }

/// Agent 执行体
pub struct Agent {
    config: AgentConfig,
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    cache: GlobalCache,
    hook_engine: Option<HookEngine>,
    exec_policy: Option<ExecPolicy>,
    registry: Option<Arc<AgentRegistry>>,
}
```

**方法**:
- `Agent::builder() -> AgentBuilder`
- `AgentBuilder::model() / system_prompt() / tools() / provider() / thinking() / ...` — 链式调用
- `AgentBuilder::build() -> Result<Agent>` — 校验必填字段
- `Agent::chat(&self, input: &str) -> Result<String>` — 内部构造 SimpleLoopConfig + SimpleLoopContext，调用 run_simple_loop
- `Agent::chat_stream(&self, input: &str) -> Result<mpsc::UnboundedReceiver<AgentEvent>>` — 用 channel 桥接 EventCallback

**关键约束**:
- Agent 内部调用现有的 `run_simple_loop()`，不重写循环逻辑
- Agent 持有 `Arc<dyn Provider>`（不是 Box），支持跨线程共享
- Builder 要能从 `OrionConfig` + model name 自动创建 Provider

**验收**:
```rust
// 这段代码能编译并跑通
let agent = Agent::builder()
    .model("deepseek-chat")
    .system_prompt("你是一个助手")
    .tools(register_default_tools())
    .provider(my_provider)
    .build()?;
let answer = agent.chat("你好").await?;
assert!(!answer.is_empty());
```

**依赖**: 无（纯新增文件）

---

### T2: AgentEvent 枚举 + 事件桥接

**目标**: 定义统一的 Agent 事件类型，替代直接使用 LoopEvent。

**输入**:
- `src/core/loop.rs` L21-44 — LoopEvent 定义
- `src/core/loop.rs` L47 — EventCallback 类型

**输出**:
在 `src/core/agent.rs` 中新增：

```rust
/// Agent 事件（面向用户的统一事件类型）
pub enum AgentEvent {
    /// 思考内容
    Thinking(String),
    /// 文本增量
    Text(String),
    /// 工具调用开始
    ToolStart { name: String, input: String },
    /// 工具调用结束
    ToolEnd { name: String, result: String, success: bool, duration_ms: u64 },
    /// 轮次完成
    TurnComplete { turn: u64 },
    /// Agent 完成
    Done { message: String, total_tokens: u64 },
    /// 错误
    Error(String),
}
```

**桥接逻辑**: `chat_stream()` 内部创建 `mpsc::unbounded_channel::<AgentEvent>`，EventCallback 闭包把 LoopEvent 转换为 AgentEvent 发送到 channel。

**验收**: `chat_stream()` 返回的 receiver 能收到完整的事件流。

**依赖**: T1

---

### T3: 将 Agent 注册到 lib.rs 导出

**目标**: 让外部可以通过 `orion_agent::core::agent::Agent` 使用。

**输入**:
- `src/lib.rs` — 当前模块导出
- `src/core/mod.rs` — 核心模块索引

**输出**:
- `src/core/mod.rs` 添加 `pub mod agent;`
- `src/lib.rs` 的 `prelude` 添加 `Agent`, `AgentConfig`, `AgentBuilder`, `AgentEvent`
- `src/core/agent.rs` 标注 `#[cfg(feature = "openai-compat")]` 如果依赖了 Provider 实现

**验收**: `cargo build --lib` 通过，外部能 `use orion_agent::core::agent::Agent;`

**依赖**: T1

---

## 阶段二：P1 — 清理与统一

### T4: 消除 execute_turn 与 run_simple_loop 的重复

**目标**: `cli/execute.rs` 的 `execute_turn()` 内部改为调用 `Agent::chat()`。

**输入**:
- `src/cli/execute.rs` L83-213 — 当前实现
- T1 产出的 Agent 结构体

**输出**:
- `execute_turn()` 内部创建 Agent，调用 `agent.chat_stream()` 
- UI 渲染代码（print_box/print_result_box/flush_thinking）保留在 execute_turn 的 EventCallback 中
- 删除 execute_turn 中手动构造 SimpleLoopConfig 的重复代码

**验收**: `cargo build` 通过，CLI 对话功能不变。

**依赖**: T1, T2

---

### T5: 消除 gateway 重复代码

**目标**: `gateway/mod.rs` 的 `run_onlyrun()` 和 `gateway/commands.rs` 的 `run` 命令约 130 行重复代码合并。

**输入**:
- `src/gateway/mod.rs` — run_onlyrun()
- `src/gateway/commands.rs` — run 命令 handler

**输出**:
- 提取公共函数 `run_task_once(task: &str, config: &OrionConfig, images: Option<...>) -> Result<String>`
- 两处都调用这个公共函数
- 修复 run_gateway() 中 "tui" 路由 Bug（改为直接调用 `crate::cli::repl::run()`）

**验收**: `cargo build` 通过，`orion-agent --onlyrun "task"` 和 `orion-agent` → 选 CLI 都能正常工作。

**依赖**: 无

---

### T6: 清理 cli/commands.rs 中的死代码

**目标**: 删除 SlashRegistry 中永远不会执行到的占位命令。

**输入**:
- `src/cli/commands.rs` — SlashRegistry 中的 /model, /new, /resume, /history, /memory 占位实现
- `src/cli/repl.rs` — handle_command() 已优先匹配这些命令

**输出**:
- 删除 commands.rs 中 `/model`, `/new`, `/resume` 的占位实现（repl.rs 已覆盖）
- 保留 `/history`, `/memory` 的占位但标记 `// TODO: 实现`
- 或者直接从 SlashRegistry 中移除这些未使用的注册

**验收**: `cargo build` 通过，REPL 所有命令正常工作。

**依赖**: 无

---

## 阶段三：P2 — MCP 集成

### T7: MCP 配置集成到 config.yaml

**目标**: 让 MCP server 通过配置文件声明，启动时自动加载。

**输入**:
- `src/config.rs` — OrionConfig 结构体
- `src/tools/mcp.rs` — McpServerConfig + connect_mcp_server()
- `src/tools/mcp.rs` L153-174 — connect_mcp_server 完整实现

**输出**:
- `OrionConfig` 新增 `mcp_servers: Vec<McpServerConfig>` 字段（serde default 空 vec）
- config.yaml 支持：
  ```yaml
  mcp_servers:
    - name: "filesystem"
      command: "npx"
      args: ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
  ```
- `McpServerConfig` 已在 mcp.rs 中定义且有 Deserialize，直接复用

**验收**: 配置文件中有 mcp_servers 时，`OrionConfig::load()` 能正确解析。

**依赖**: 无

---

### T8: Gateway 启动时自动连接 MCP 并注入 ToolRegistry

**目标**: MCP 工具在 Agent 启动时自动可用。

**输入**:
- T7 产出的配置
- `src/tools/mcp.rs` L153-174 — connect_mcp_server()
- `src/gateway/commands.rs` — 工具注册流程
- `src/cli/repl.rs` — 工具注册流程

**输出**:
- 提取公共函数 `init_tools_from_config(config: &OrionConfig) -> (ToolRegistry, Vec<McpClient>)`
- 在 gateway 和 repl 的启动流程中调用此函数
- MCP 工具注册到同一个 ToolRegistry，与内置工具无差别使用
- 返回 McpClient 列表，用于退出时 shutdown

**验收**: 配置一个 MCP server（如 `@modelcontextprotocol/server-filesystem`），Agent 能通过 MCP 工具读写文件。

**依赖**: T7

---

## 阶段四：P3 — Coordinator 接入 Plan

### T9: Coordinator 调用 TaskPlan 拆解任务

**目标**: 让 Coordinator 真正使用 plan.rs 的任务分解能力。

**输入**:
- `src/orchestrator/coordinator.rs` L98-106 — 当前 execute() 只创建单个 Worker
- `src/orchestrator/plan.rs` — TaskPlan + PLANNING_SYSTEM_PROMPT + next_executable()
- `src/orchestrator/worker.rs` — Worker::execute()

**输出**:
修改 `Coordinator::execute()`:

```rust
pub async fn execute(&self, task: &Task) -> crate::Result<String> {
    // 1. 调用 LLM 生成 TaskPlan
    let plan_json = self.call_llm_for_plan(task).await?;
    let mut plan = TaskPlan::from_json(&plan_json)?;
    
    // 2. 循环执行可执行任务
    while !plan.is_complete() {
        let Some(subtask) = plan.next_executable().cloned() else {
            break; // 没有可执行的任务（可能有依赖卡住）
        };
        
        let worker = self.create_worker(&subtask.id).await;
        match worker.execute(&subtask.description, &plan.completed_summary()).await {
            Ok(result) => plan.mark_completed(&subtask.id, result),
            Err(e) => plan.mark_failed(&subtask.id, e.to_string()),
        }
    }
    
    // 3. 汇总
    Ok(plan.completed_summary())
}

async fn call_llm_for_plan(&self, task: &Task) -> crate::Result<String> {
    // 用 self.provider + PLANNING_SYSTEM_PROMPT 调用一次 LLM
    // 返回 JSON 字符串
}
```

**验收**: 给 Coordinator 一个复杂任务（如 "创建一个 Rust web server 并写测试"），能看到被拆解成多个子任务顺序执行。

**依赖**: 无

---

### T10: 删除 coordinator.rs 中的重复 ExecutionPlan

**目标**: coordinator.rs 和 plan.rs 有两套并行的计划数据结构，统一为一套。

**输入**:
- `src/orchestrator/coordinator.rs` L30-77 — Task + ExecutionPlan + PlanTask + PlanTaskStatus
- `src/orchestrator/plan.rs` — SubTask + TaskPlan + TaskStatus

**输出**:
- 删除 coordinator.rs 中的 `Task`, `ExecutionPlan`, `PlanTask`, `PlanTaskStatus`
- Coordinator 统一使用 plan.rs 的 `TaskPlan` + `SubTask`
- 更新 Coordinator::execute() 的签名，接收 `&str`（任务描述）而不是 `&Task`

**验收**: `cargo build` 通过，无重复数据结构。

**依赖**: T9

---

## 阶段五：P4 — 护栏集成

### T11: 将 GuardrailChain 集成到 run_simple_loop

**目标**: 让护栏系统真正生效，而不只是存在。

**输入**:
- `src/core/guardrail.rs` — GuardrailChain + check_pre_tool()
- `src/core/loop.rs` L636+ — 工具执行部分

**输出**:
- `SimpleLoopContext` 新增 `guardrails: Option<&'a GuardrailChain>` 字段
- 在 run_simple_loop 的工具执行前，调用 `guardrails.check_pre_tool()`
- 如果返回 `Deny(reason)`，跳过工具执行，返回错误结果给 LLM
- 如果返回 `Skip`，静默跳过

**验收**: 配置一个 PermissionGuardrail 禁止 bash 工具，Agent 尝试调用 bash 时被拦截。

**依赖**: 无

---

## 阶段六：清理与文档

### T12: 清理未使用的模块和 import

**目标**: 删除确认未使用的代码。

**输入**:
- `src/agent/runtime.rs` — AgentRuntime（被 Agent 取代后可能成为死代码）
- `src/tools/spec.rs` — ToolSpecRegistry（未与 ToolRegistry 集成）
- `src/tools/category.rs` — CategoryRegistry（未与 ToolRegistry 集成）
- `src/agent/lanes.rs` — LaneManager（从未被调用）
- `src/agent/protocol.rs` — A2AMessage（仅在 examples 中使用）

**输出**:
- 评估每个模块是否还有价值：
  - AgentRuntime → 保留但标注 deprecated，Agent 是新入口
  - ToolSpecRegistry → 保留（未来工具暴露控制需要）
  - CategoryRegistry → 保留（system prompt 生成已使用）
  - LaneManager → 保留（未来并行执行需要）
  - A2AMessage → 保留（examples 使用）
- 删除真正无用的 import 和 dead code
- `cargo build` 无 warning

**验收**: `cargo build 2>&1 | grep "warning"` 无输出。

**依赖**: T1-T11 全部完成

---

## 执行顺序建议

```
并行组 1: T1 → T2 → T3 (Agent 核心，串行依赖)
并行组 2: T5, T6, T7, T9, T11 (互相独立，可并行)
并行组 3: T4 (依赖 T1-T3), T8 (依赖 T7), T10 (依赖 T9)
最终: T12 (依赖全部)
```

最优路径（最快完成）:
1. T1 → T2 → T3 (Agent 核心)
2. 同时: T5 + T6 + T7 + T9 + T11 (5个独立任务并行)
3. 同时: T4 + T8 + T10
4. T12 (清理)
