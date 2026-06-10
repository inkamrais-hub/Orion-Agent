# Orion Agent — 问题注意事项与当前状态

**日期**: 2026-06-11
**分支**: main @ 6bb70af
**编译状态**: 通过 (0 errors, 57 warnings)
**测试状态**: 158 tests passed, 0 failed (含 7 个新增 budget 测试)

---

## 提交历史 (本轮)

| Commit | 内容 |
|--------|------|
| 576c0c4 | feat: sandbox mode, gateway refactor, ExecPolicy hardening, code review fixes |
| 0ba9738 | fix: config provider default, OnceLock cache, clone fixes |
| 6bb70af | fix: enforce token budget termination + WorkspaceGuard init in run_task_once |

---

## 本次修复摘要

### 1. 无网络沙箱模式 (新增)

**动机**: 压测中发现模型可通过 bash 工具执行 `git push`、`curl` 等网络操作，存在安全风险。

**实现**:
- `ExecPolicy` 新增 `sandbox: bool` 字段
- 沙箱模式优先于用户规则检查，无条件拦截:
  - VCS 写操作: `git push/fetch/clone/pull/remote/submodule`
  - 网络工具: `curl`, `wget`, `ssh`, `scp`, `rsync`, `ftp`, `telnet` 等
  - PowerShell 网络 cmdlet: `Invoke-WebRequest`, `Invoke-RestMethod` 等
  - 嵌套 shell 命令: `bash -c "curl ..."` / `sh -c "git push ..."`
- 只允许 git 只读命令: `status`, `log`, `diff`, `branch`
- 默认 git 规则从 `add/commit` 缩减为只读 (`status/log/diff/branch`)

**使用方式**:
```bash
# CLI 单次任务 + 沙箱
orion-agent run "重构这个项目" --sandbox

# 兼容旧 --onlyrun 语法
orion-agent --onlyrun "重构这个项目" --sandbox

# Docker 沙箱 (硬件级隔离, 默认 network=none)
# 在 config.yaml 中启用 docker.enabled: true
```

**局限**:
- 软件级沙箱无法完全阻止所有网络绕过方式 (如自定义二进制文件)
- 企业级安全建议使用 Docker 沙箱或虚拟机隔离

### 2. Token Budget 强制终止 (新增)

**动机**: 压测 #2 中模型使用 657K tokens (预算 128K)，循环未终止，77% token 浪费。

**实现**:
- `LoopState` 新增 `budget_critical_streak: u32` 字段
- 分级响应策略:
  - 首次 Critical: 立即触发上下文压缩 (不受 50 消息阈值限制)
  - 连续 3 轮 Critical: 强制终止，返回 `LoopOutcome::BudgetExceeded`
  - 状态恢复 Ok/Warning: 重置 streak 计数器
- `ContextManager` 新增 `force_compact()` 方法 (绕过消息数阈值)
- 7 个新增单元测试覆盖 budget 逻辑

### 3. WorkspaceGuard 初始化修复

**动机**: `run_task_once()` 未初始化 WorkspaceGuard，导致所有 bash 命令被 fail-close 拒绝。

**修复**: 在 `run_task_once()` 开头调用 `init_workspace_guard(workspace_root).await`。

### 4. 其他修复

- Gateway 硬编码: `prompt_cache`/`max_output_tokens`/`max_turns` 改用配置值
- Config provider 默认值: 添加 `#[serde(default = "default_provider")]` 防止解析失败
- OnceLock config cache: `OrionConfig::load_cached()` 避免热路径重复加载
- ExecPolicy 安全加固: 路径匹配 + 参数匹配改进

---

## 压测 #2 结果 (DeepSeek v4-flash + 沙箱)

| 指标 | 压测 #1 (MiMo) | 压测 #2 (DeepSeek + 沙箱) |
|------|----------------|---------------------------|
| 总耗时 | 136s | 160s |
| 终止原因 | MaxTurnsReached (20) | 自然完成 |
| 产出文件 | 5 | 7 (Cargo.toml + 6 .rs) |
| 编译结果 | 16 errors + 6 warnings | 1 error + 2 warnings |
| 工具调用成功率 | ~70% | ~95% |
| Token 消耗 | ~395K | 657K (预算 128K, 5x 超限) |
| Sandbox 触发 | N/A | 0 次 (模型未尝试网络操作) |

**详细报告**: `F:\测试-agent\STRESS_TEST_REPORT_2.md`

---

## 已知未解决问题

### P1 — 高优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 1 | Context Compaction 效果差 | 未修复 | 52→1 条消息只释放 5K tokens，需改进压缩策略 |
| 2 | UnifiedStore 未在 run_task_once 初始化 | 未修复 | "No UnifiedStore available for snapshot, skipping" 警告 |
| 3 | Prompt Cache 在 --onlyrun 路径未集成 | 部分修复 | prompt_cache 正确传递，PromptBuilder 三段式待验证 |

### P2 — 中优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 4 | 57 个 deprecated API warnings | 未修复 | `core::audit` → `crate::audit`，`session::SessionEntry` → `UnifiedStore` |
| 5 | 模型使用 Linux 路径 (`/workspace/`) | 未修复 | 需要在 system prompt 中明确 OS 和工作目录 |
| 6 | `pub mod core` 与 Rust 内置 core 冲突 | 未修复 | LLM 生成质量问题 |
| 7 | `get_tool_calls_for_turn` dead code | 未修复 | session/store.rs:441 |
| 8 | Docker 沙箱 Windows 兼容性 | 待验证 | Docker Desktop daemon 未运行时的降级逻辑 |

---

## 分支状态

| 分支 | 最新 Commit | 内容 |
|------|-------------|------|
| main | 6bb70af | 沙箱模式 + gateway 重构 + budget 强制终止 + WorkspaceGuard 修复 |
| beta | d16295d | 架构改进 + UnifiedStore + 死代码清理 + 压测修复 |

---

## 下一步计划

1. **Context Compaction 优化**: 改进压缩策略，每 10 轮定期压缩而非仅在阈值时触发
2. **UnifiedStore 初始化**: 在 `run_task_once()` 中初始化 store 以支持快照/回滚
3. **合并 beta 分支**: 将 beta 分支的架构改进合入 main
4. **清理 deprecated warnings**: 57 → 0 (core::audit + session types)
5. **System prompt 改进**: 增加 OS/路径信息、Rust 保留字提示
6. **重跑压测**: 验证 budget 强制终止是否生效
