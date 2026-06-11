# Orion Agent — 问题注意事项与当前状态

**日期**: 2026-06-11
**分支**: main @ caa25d1
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

---

## 框架化改造任务 (TASKS.md) 状态

**全部 12 项任务已完成。**

- T1-T3: Agent 核心结构体 + Builder + 事件桥接
- T4: execute_turn 统一调用 Agent 接口
- T5: Gateway 代码去重 (run_task_once 公共函数)
- T6: cli/commands.rs 死代码清理
- T7-T8: MCP 配置集成 + Gateway 自动连接
- T9-T10: Coordinator + TaskPlan DAG 执行，删除重复 ExecutionPlan
- T11: GuardrailChain 集成到 ToolExecutor
- T12: Clippy 代码质量清理 (75 → 12 warnings)

---

## 已知未解决问题

### P2 — 中优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 1 | Context Compaction 端到端压测 | 待压测 | token-based compaction + budget reset 需完整验证 |
| 2 | Prompt Cache 在 --onlyrun 路径未完整集成 | 部分修复 | prompt_cache 正确传递，PromptBuilder 三段式待验证 |
| 3 | 模型使用 Linux 路径 (`/workspace/`) | 未修复 | System Prompt 已加 CWD，待压测验证效果 |
| 4 | DeepSeek API TLS 握手间歇性失败 | 未修复 | 网络层问题，http1_only() 部分缓解 |
| 5 | Docker 沙箱 Windows 兼容性 | 待验证 | Docker Desktop daemon 未运行时的降级逻辑 |

---

## 下一步计划

1. **端到端压测**: 验证 token-based compaction + budget reset 完整流程
2. **Prompt Cache 集成**: 在 run_task_once 路径集成 PromptBuilder 三段式 prompt
3. **合并 beta 分支**: 如果还有未合入的改动
