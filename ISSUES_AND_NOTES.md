# Orion Agent — 问题注意事项与当前状态

**日期**: 2026-06-11
**分支**: main @ 7a92ce6
**编译状态**: 通过 (0 errors, 0 warnings)
**Clippy 状态**: 12 warnings (均为 API 设计级建议，非代码缺陷)
**测试状态**: 158 tests passed, 0 failed

---

## 提交历史 (本轮)

| Commit | 内容 |
|--------|------|
| 8a2fc11 | feat: UX improvements, compaction optimization, deprecated API cleanup |
| e01f0be | refactor: token-based compaction instead of message count |
| d56fb4b | refactor: token-based compaction with budget reset |
| f8b154b | Add AGPL v3.0 license |
| caa25d1 | refactor: clippy cleanup — 75 warnings reduced to 12 |
| 3e5a4c8 | docs: TASKS.md + ISSUES_AND_NOTES.md update |
| 7a92ce6 | feat: multi-agent orchestration — prompt cache, parallel DAG, A2A request-response |

---

## 本轮多 Agent 编排开发 (7a92ce6)

### 已完成功能

1. **Prompt Cache 端到端打通**
   - `AgentConfig.prompt_cache` → `ModelCaps.prompt_cache` → `ProviderRequest.enable_prompt_cache` → API 请求体
   - `loop.rs` 在消息列表 ≥3 条时自动标记 cache breakpoint（倒数第二条消息）
   - 缓存命中/创建 token 数的日志输出

2. **Agent 多轮对话 (`chat_with_history`)**
   - 新 API: `agent.chat_with_history(input, &mut history)` 
   - 自动管理消息历史，通过 `initial_messages` 注入到 loop
   - 事件回调重建 tool call 消息，保持完整对话上下文

3. **Coordinator 并行 DAG 执行**
   - `TaskPlan::next_executable_batch()` — 获取所有无依赖/依赖已完成的待处理任务
   - `Coordinator::execute()` 重写 — 单任务直接执行，多任务通过 `JoinSet` 并行执行
   - 死循环保护 (max 20 iterations) + DAG 卡死检测

4. **Orchestrator 顶层分发**
   - 新增 `Orchestrator` struct，统一入口 `run(task)` 
   - Sequential/Parallel → Coordinator (DAG 模式)
   - Collaborative → MapReduceOrchestrator (Map-Reduce 模式)
   - LLM 驱动的自动任务分解 (`decompose_for_collaborative`)

5. **SubAgent 工具重写**
   - 可配置工具集: `readonly` (read/glob/grep/skeleton), `full` (全部), `search` (glob/grep)
   - 结果缓存: LRU 64 entries, 5 分钟 TTL, 基于 task+tool_set 哈希
   - 继承父级 `prompt_cache` 和 `compaction_ratio` 配置
   - 只读模式系统提示，防止子 Agent 意外写文件

6. **A2A 请求-响应机制**
   - `AgentRegistry::send_and_wait()` — 发送消息并阻塞等待回复 (correlation ID + oneshot channel)
   - `AgentRegistry::deliver_reply()` — 目标 Agent 投递回复
   - `SendMessageTool` 新增 `wait_for_reply` 参数 (默认 false 保持兼容)
   - 60 秒超时保护

7. **Prelude 导出完善**
   - 新增: `SimpleLoopConfig`, `SimpleLoopContext`, `LoopOutcome`, `LoopEvent`, `ModelCaps`
   - 新增: `Provider`, `Message`, `Role`, `ContentBlock`, `GlobalCache`
   - 新增: `Tool`, `ToolContext`, `ToolRegistry`
   - 新增: `Orchestrator`, `OrchestratorConfig`, `OrchestratorMode`, `OrchestratorResult`
   - 新增: `Coordinator`, `CoordinatorConfig`, `TaskPlan`, `PlanSubTask`, `TaskStatus`
   - 新增: `MapReduceOrchestrator`, `SwarmSummary`
   - 新增: `AgentRegistry`, `A2AMessage`, `AgentMessage`
   - 更新模块文档树结构图

### 改动统计
- 10 files changed, +852 / -85 lines
- 编译: 0 errors, 0 warnings
- Clippy: 12 warnings (均为已有代码的 API 设计建议)
- 测试: 158 passed, 0 failed
