# Orion Node System — 设计笔记

> 日期: 2026-06-11 脑暴记录
> 状态: 概念设计阶段，尚未动手

---

## 一、核心思路

Agent 框架工作流化，对标 ComfyUI 的节点编排模式。

**关键洞察：**
- Tool 天然就是节点（输入 → 处理 → 输出）
- Loop/Model 不是节点，是**调度器**（决定调哪些节点）
- Sub-agent 的工具范围由**连线**决定，不是配置项
- 连线即权限，断开即禁止——小学生都能看懂

**商业化路径：** 卖工作流。一个 JSON 文件 = 一个可复用的 agent 工作流。

---

## 二、节点定义规范

每个节点携带四样东西：

```
NodeDefinition {
    // 1. 端口定义（数据流契约）
    input_ports: [{name, type, required}],
    output_ports: [{name, type}],

    // 2. 执行逻辑（纯数据流，与 LLM 无关）
    execute: (inputs) → outputs,

    // 3. 系统提示片段（仅 tool 类节点需要）
    sys_prompt_fragment: Option<String>,

    // 4. 启用开关（双保险，防止未连接的工具被调用）
    enabled: bool,
}
```

**工作流 JSON 格式（参考 ComfyUI API Format）：**

```json
{
    "1": {
        "class_type": "ReadTool",
        "inputs": { "path": "/src/main.rs" }
    },
    "2": {
        "class_type": "GrepTool",
        "inputs": {
            "pattern": "fn main",
            "path": ["1", 0]          // ← 连线：节点1的第1个输出
        }
    },
    "3": {
        "class_type": "AgentNode",
        "inputs": {
            "model": ["model_1", 0],
            "tools": ["2", "5", "7"],  // ← 连接的工具节点 ID 列表
            "task": "分析这段代码的架构"
        }
    }
}
```

---

## 三、节点分类体系

### 3.1 Tool 节点（计算节点）

每个 tool 独立一个节点。是工作流的原子操作单元。

| 类别 | 节点名 | 输入 | 输出 | 有 sys_prompt |
|------|--------|------|------|:---:|
| **文件操作** | ReadTool | path: STRING | content: STRING | ✅ |
| | WriteTool | path, content: STRING | success: BOOL | ✅ |
| | EditTool | path, old_string, new_string: STRING | success: BOOL | ✅ |
| **搜索** | GlobTool | pattern: STRING, path?: STRING | files: STRING_LIST | ✅ |
| | GrepTool | pattern: STRING, path?: STRING | matches: STRING | ✅ |
| | WebSearchTool | query: STRING | results: STRING | ✅ |
| **代码智能** | SymbolSearchTool | query: STRING | symbols: STRING | ✅ |
| | FindCallersTool | symbol: STRING | callers: STRING | ✅ |
| | ProjectMapTool | root?: STRING | tree: STRING | ✅ |
| | SkeletonTool | path: STRING | skeleton: STRING | ✅ |
| **Shell** | BashTool | command: STRING | stdout, stderr: STRING, exit: INT | ✅ |
| | MultiShellTool | commands: STRING_LIST | results: STRING | ✅ |
| **快照** | SnapshotHistoryTool | path?: STRING | history: STRING | ✅ |
| | SnapshotRollbackTool | path, snapshot_id: STRING | success: BOOL | ✅ |
| | SnapshotRiskyTool | (none) | risky_files: STRING | ✅ |
| **通信** | AskUserTool | question: STRING | answer: STRING | ✅ |
| | SendMessageTool | target, message: STRING | reply?: STRING | ✅ |
| | ListPeersTool | (none) | peers: STRING | ✅ |
| **子代理** | SubAgentTool | task: STRING, max_turns?: INT | result: STRING | ✅ |
| **元工具** | LoadTool | tool_name: STRING | success: BOOL | ❌ |
| | ListCategoriesTool | (none) | categories: STRING | ❌ |
| **MCP 代理** | McpProxyTool_* | (动态) | (动态) | ✅(动态) |

### 3.2 Agent 节点（调度节点）

Agent 节点本身不执行具体操作，它调度 tool 节点。

```
AgentNode {
    input_ports: [
        {name: "model", type: "MODEL", required: true},
        {name: "tools", type: "NODE_REF_LIST", required: true},   // 连接的 tool 节点 ID
        {name: "task", type: "STRING", required: true},
        {name: "system_prompt", type: "STRING", required: false},
        {name: "context", type: "STRING", required: false},       // 上游节点输出作为上下文
    ],
    output_ports: [
        {name: "result", type: "STRING"},
        {name: "usage", type: "USAGE"},
    ],
    // Agent 节点不携带 sys_prompt_fragment
    // 它的 sys prompt = base_prompt + 所有已连接且 enabled 的 tool 节点的 sys_prompt_fragment
}
```

**Sub-agent 工具限制：** Agent 节点只能使用连线连到它身上的 tool 节点。没连线的 tool？模型根本不知道它存在。连线即权限。

### 3.3 Model 节点（资源节点）

```
ModelNode {
    input_ports: [
        {name: "provider", type: "STRING", required: true},     // "openai" | "anthropic"
        {name: "endpoint", type: "STRING", required: true},
        {name: "api_key", type: "STRING", required: true},
        {name: "model_name", type: "STRING", required: true},
    ],
    output_ports: [
        {name: "model", type: "MODEL"},
    ],
    // Model 节点无 sys_prompt_fragment
}
```

### 3.4 Orchestrator 节点（高级调度）

| 节点 | 用途 |
|------|------|
| CoordinatorNode | DAG 任务分解 + 按依赖执行 |
| MapReduceNode | 并行扇出 + 汇总 |
| SequentialNode | 顺序执行子图 |
| ParallelNode | 并行执行子图 |

### 3.5 数据节点（常量/变量）

| 节点 | 用途 |
|------|------|
| TextInput | 用户输入常量 |
| FileInput | 从文件读取常量 |
| Variable | 命名变量（可被多处引用） |
| Output | 工作流最终输出 |

### 3.6 特殊节点（需要谨慎设计）

| 节点 | 说明 | 谨慎原因 |
|------|------|----------|
| MCP 代理节点 | 动态加载外部 MCP server 的工具 | 输入输出类型动态，无法静态校验 |
| Skill 节点 | 加载技能包 | 技能包可能包含多个子节点，需要展开机制 |
| Hook 节点 | 前置/后置拦截器 | 需要明确触发时机 |
| Guardrail 节点 | 安全检查链 | 需要强制执行，不能被绕过 |

---

## 四、MCP / Skill 的特殊考虑

**MCP 节点：** MCP server 提供的是动态 tool 集合。一个 MCP server 可能暴露 1-N 个工具。

方案选择：
- **A. 一个 MCP Server 节点** → 内部包含所有该 server 的工具，不透明
- **B. 每个 MCP tool 独立节点** → 启动时动态展开，每个 tool 变成独立节点
- **C. MCP 适配器节点** → 一个固定节点，接受 tool_name 参数，路由到对应 MCP tool

推荐 **B**（展开为独立节点），保持节点图的透明性和组合性。但需要在协议层支持动态端口声明。

**Skill 节点：** 技能包本质上是一组 tool + prompt 模板。

方案选择：
- **A. Skill = 子图** → 展开后变成多个 tool 节点 + 一个 Agent 节点
- **B. Skill = 单一复合节点** → 黑箱，不透明

推荐 **A**（展开为子图），保持一致性。

---

## 五、执行引擎映射

Orion 现有组件 → 节点系统执行引擎：

| ComfyUI 概念 | Orion 对应 |
|-------------|-----------|
| Workflow JSON | 新增 `WorkflowDefinition` struct |
| class_type → node class | `NodeRegistry` 按 class_type 查找 `NodeDefinition` |
| [node_id, output_index] 连线 | `DataLink { source: NodeId, output: usize, target: NodeId, input: String }` |
| ExecutionList (拓扑调度) | `Coordinator` 的 DAG 执行（已有） |
| CacheSet (输出缓存) | `GlobalCache`（已有） |
| /prompt API | axum `POST /workflow/execute`（已有 axum 基础） |
| WebSocket 进度 | axum WebSocket（已有基础） |
| Node validation | `validate_inputs()` 检查类型匹配 + 必填项 |

---

## 六、前端方案

**路线 A（推荐先行）：ComfyUI 寄生**

- 用 LiteGraph.js（ComfyUI 前端用的同一个图库）搭建编辑器
- 后端用 axum 提供节点列表 API + 工作流执行 API + WebSocket 进度
- Python 桥接层（可选）：让工作流也能在 ComfyUI 里跑

**路线 B（远期）：自建前端**

- React/Vue + 自研节点编辑器
- 更精细的交互设计

---

## 七、落地路径建议

### Phase 0: 记录（当前）
- [x] 脑暴记录
- [x] 组件盘点
- [x] 节点分类

### Phase 1: 协议层
- [ ] 定义 `NodeDefinition` trait / struct
- [ ] 定义端口类型系统（STRING, INT, BOOL, STRING_LIST, MODEL, NODE_REF...）
- [ ] 定义工作流 JSON schema
- [ ] 为每个现有 Tool 生成 NodeDefinition（自动 or 手动）
- [ ] sys_prompt_fragment 从现有 tool description 提取

### Phase 2: 执行适配
- [ ] `NodeRegistry` — 按 class_type 查找节点定义
- [ ] `WorkflowExecutor` — 解析 JSON → 构建 DAG → 调用 Coordinator
- [ ] `DataLink` 解析 — [node_id, output_index] → 实际数据传递
- [ ] Agent 节点的 sys_prompt 动态组装

### Phase 3: API 层
- [ ] `GET /nodes` — 返回所有节点定义（class_type, ports, category）
- [ ] `POST /workflow/execute` — 提交工作流 JSON
- [ ] WebSocket 进度推送 — 节点开始/完成/失败
- [ ] `GET /workflow/{id}/status` — 查询状态

### Phase 4: 前端
- [ ] LiteGraph.js 集成
- [ ] 节点渲染（从 /nodes API 动态生成）
- [ ] 连线 + 数据流可视化
- [ ] 执行进度动画

---

## 八、现有组件完整清单（83个）

### Tools (22)
ReadTool, WriteTool, EditTool, BashTool, MultiShellTool, GlobTool, GrepTool,
SymbolSearchTool, FindCallersTool, ProjectMapTool, SkeletonTool,
WebSearchTool, SubAgentTool, AskUserTool, SendMessageTool, ListPeersTool,
LoadTool, ListCategoriesTool,
SnapshotHistoryTool, SnapshotRollbackTool, SnapshotRiskyTool,
MCP 动态代理 (运行时数量不定)

### Providers (2+factory)
AnthropicProvider, OpenAICompatProvider, create_provider() factory

### Orchestrator (4)
Coordinator, Worker, MapReduceOrchestrator, TaskPlan

### Core (16)
SimpleLoop, ContextManager (6 compaction strategies), GlobalCache (L1+L2),
ExecPolicy, HookEngine, GoalManager, GuardrailChain, PermissionBroker,
ExecMode, PromptBuilder, ToolExecutor, Workspace, OrionIgnore, ModelCaps

### Session (7)
SessionMemory, GitSandbox, RolloutRecorder, UnifiedStore, SessionStore,
SessionBackend, SessionFileManager

### Other (12+)
AgentRegistry, A2A Protocol, Lanes, OrionConfig, ModelConfig, ModelRegistry,
CodeIndex, SkeletonExtractor, EventBus, AuditLogger, GatewayContext, CommandRegistry

---

## 九、工具聚类 + Prompt 缓存优化（B+C 合并方案）

> 2026-06-11 后续讨论补充

### 9.1 问题背景

当前 22 个 tool 各自独立，如果每个 tool 都带 `sys_prompt_fragment` 动态注入 Agent 节点的 system prompt，会导致：

1. **Prompt 缓存命中率低** — 每次工作流不同，tool 组合不同，sys_prompt 变化频繁，缓存基本打不中
2. **模型容易误调用** — 22 个工具描述堆在 prompt 里，模型容易选错工具
3. **sys_prompt 膨胀** — 全量 tool 描述加起来很长，浪费 token

### 9.2 方案 B 和 C 合并

原方案 B（静态 prompt + 连线拒绝）和方案 C（工具聚类）应该合在一起做：

**第一步：工具聚类（缩小节点数 + 缩小 sys_prompt）**

将 22 个 tool 按功能聚合成 5-6 个大类：

| 聚类名 | 包含工具 | sys_prompt 大小估算 |
|--------|---------|-------------------|
| `file_ops` | Read, Write, Edit, Glob, Grep | 中（5个工具描述合并） |
| `search` | WebSearch, SymbolSearch, FindCallers, ProjectMap, Skeleton | 中 |
| `shell` | Bash, MultiShell | 小 |
| `snapshot` | SnapshotHistory, SnapshotRollback, SnapshotRisky | 小 |
| `communication` | AskUser, SendMessage, ListPeers | 小 |
| `sub_agent` | SubAgentTool | 小 |

聚类后，节点图里的"tool 节点"变成"tool cluster 节点"，一个 cluster 节点代表一类工具。用户连线时连的是 cluster，不是单个 tool。

**第二步：静态 prompt + 连线拒绝（保证缓存命中）**

- 每个 cluster 的 `sys_prompt_fragment` 是**固定不变**的（不是动态拼出来的）
- Agent 节点的 system prompt = `base_prompt` + 已连接 cluster 的固定 fragment
- 如果模型调用了未连接的 tool → 运行时直接拒绝（返回错误提示）
- 因为 prompt 是静态组合，同样的工作流 → 同样的 prompt → **缓存命中**

### 9.3 主 Agent vs Sub-Agent 的权限模型

这是整个设计最精妙的地方：

**主 Agent（MainAgent 节点）：**
- 拥有全部工具能力，不做限制
- sys_prompt 包含所有 cluster 的 fragment
- 这是合理的——主 agent 就是全能管家

**Sub-Agent（ToolSubAgent 节点）：**
- 用户创建 SubAgent 节点时，**自行指定挂载哪些 tool cluster**
- 每个 SubAgent 节点可以有不同工具范围
- 例如：一个 SubAgent 只有 `file_ops` + `search`，另一个只有 `shell` + `snapshot`
- 工具范围由**用户拖拽连线**决定，不是代码里硬编码的

```
// ToolSubAgent 节点定义
ToolSubAgentNode {
    input_ports: [
        {name: "model", type: "MODEL", required: true},
        {name: "task", type: "STRING", required: true},
        // 以下 cluster 端口均为 optional，用户按需连接
        {name: "file_ops", type: "CLUSTER_REF", required: false},
        {name: "search", type: "CLUSTER_REF", required: false},
        {name: "shell", type: "CLUSTER_REF", required: false},
        {name: "snapshot", type: "CLUSTER_REF", required: false},
        {name: "communication", type: "CLUSTER_REF", required: false},
    ],
    output_ports: [
        {name: "result", type: "STRING"},
    ],
    // sys_prompt = base_sub_agent_prompt + 已连接的 cluster fragments
    // 未连接的 cluster → 模型完全不知道那些工具存在
}
```

**为什么这样更好：**
- 用户可以在画布上创建**多个** SubAgent 节点，每个有不同的工具组合
- 主 agent 通过 `sub_agent` cluster 调用 sub-agent，sub-agent 的能力范围一目了然
- 连线即权限，断开即禁止——可视化 + 直觉化

### 9.4 Prompt 缓存命中率对比

| 方案 | 主 Agent prompt 稳定性 | Sub-Agent prompt 稳定性 | 缓存命中率 |
|------|----------------------|------------------------|-----------|
| A. 动态插拔（当前） | 低（每次组合不同） | 低 | ~20-30% |
| B. 静态+拒绝 | 高（固定全量） | 高（固定子集） | ~80-90% |
| B+C（本方案） | 高（固定全量 cluster） | 高（固定 cluster 子集） | ~80-90%，且 prompt 更短 |

B+C 比纯 B 更好的原因：cluster 粒度的 fragment 比逐 tool 的 fragment 更短、更稳定，缓存效率相同但 token 消耗更少。

### 9.5 对现有代码的影响

实现这个方案需要重构的部分：

1. **Tool 注册** — 现有 `ToolRegistry` 按单个 tool 注册，需改为按 cluster 注册
2. **sys_prompt 生成** — 现有 `PromptBuilder` 逐 tool 拼 description，需改为按 cluster 拼
3. **SubAgentTool** — 当前只有一个 SubAgentTool 且硬编码工具列表，需改为 `ToolSubAgentNode` 由连线决定
4. **工具调用拦截** — 在 `ToolExecutor` 层增加"该 tool 是否属于已连接 cluster"的检查
5. **节点定义** — 每个 cluster 需要一个 `NodeDefinition`，包含端口、execute、sys_prompt_fragment

不需要重构的部分：单个 tool 的 execute 逻辑、Provider、Model、Orchestrator、Session 等全部保持不变。
