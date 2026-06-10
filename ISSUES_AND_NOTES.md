# Orion Agent — 问题注意事项与当前状态

**日期**: 2026-06-11
**分支**: main @ 8a2fc11
**编译状态**: 通过 (0 errors, 0 warnings in orion-agent crate)
**测试状态**: 158 tests passed, 0 failed

---

## 提交历史 (本轮)

| Commit | 内容 |
|--------|------|
| 576c0c4 | feat: sandbox mode, gateway refactor, ExecPolicy hardening |
| 0ba9738 | fix: config provider default, OnceLock cache, clone fixes |
| 6bb70af | fix: enforce token budget termination + WorkspaceGuard init |
| 8cdcb1f | docs: update ISSUES_AND_NOTES with stress test #2 results |
| 8a2fc11 | feat: UX improvements, compaction optimization, deprecated API cleanup |

---

## 本轮完成的全部改进

### 安全与稳定性
- **无网络沙箱模式**: ExecPolicy sandbox 字段，拦截网络/VCS 写操作
- **Token Budget 强制终止**: 首次 critical 立即压缩，连续 3 轮 critical 终止返回 BudgetExceeded
- **WorkspaceGuard 初始化**: run_task_once() 中初始化，修复 fail-close 拒绝所有 bash 命令的问题
- **Config provider 默认值**: 防止缺少 provider 字段导致整个配置被丢弃

### 模型体验 (System Prompt)
- **动态上下文**: 注入 OS、Shell、CWD 信息 (之前是死代码)
- **项目类型检测**: 自动检测 Rust/Node.js/Python 项目并注入构建/测试提示
- **Rust 保留字提示**: 避免 `pub mod core` 遮蔽内置 core crate

### 上下文管理
- **压缩阈值降低**: 50 → 20 条消息，更早触发避免膨胀
- **定期压缩**: 每 10 轮强制压缩一次，不依赖消息数阈值
- **tokens_freed 估算改进**: 基于内容长度 (4字符≈1token) 而非固定 100/消息

### 存储集成
- **UnifiedStore 注入**: build_main_agent() 接受 store 参数，ToolRegistry 正确设置
- 消除 "No UnifiedStore available for snapshot" 警告

### 代码质量
- **Deprecated API 清理**: 57 → 0 warnings
  - runtime.rs 迁移到 crate::audit (新 sync API)
  - 移除 core::audit 模块声明
  - session/manager.rs 添加 #[allow(deprecated)]
- **使用体验**: BudgetExceeded/MaxTurnsReached 返回中文可操作提示
- **启动信息**: run_onlyrun() 显示模型名、预算、任务预览、总耗时

---

## 已知未解决问题

### P2 — 中优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 1 | Context Compaction 实际释放效果待验证 | 待压测 | 估算改进 + 阈值降低 + 定期压缩，需压测验证 |
| 2 | Prompt Cache 在 --onlyrun 路径未完整集成 | 部分修复 | prompt_cache 正确传递，PromptBuilder 三段式待验证 |
| 3 | 模型使用 Linux 路径 (`/workspace/`) | 未修复 | System Prompt 已加 CWD，待压测验证效果 |
| 4 | `get_tool_calls_for_turn` dead code | 已 suppress | #[allow(dead_code)]，后续可删除 |
| 5 | Docker 沙箱 Windows 兼容性 | 待验证 | Docker Desktop daemon 未运行时的降级逻辑 |

---

## 分支状态

| 分支 | 最新 Commit | 内容 |
|------|-------------|------|
| main | 8a2fc11 | 沙箱 + budget 终止 + UX + compaction + 0 warnings |
| beta | d16295d | 架构改进 + UnifiedStore + 死代码清理 (大部分已 cherry-pick 到 main) |

---

## 下一步计划

1. **重跑压测**: 验证所有改进的实际效果 (budget 终止、compaction、system prompt)
2. **合并 beta 分支**: 如果还有未合入的改动
3. **Context Compaction 调优**: 根据压测结果调整阈值和间隔
4. **Prompt Cache 集成**: 在 run_task_once 路径集成 PromptBuilder 三段式 prompt
