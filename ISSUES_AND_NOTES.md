# Orion Agent — 问题注意事项与当前状态

**日期**: 2026-06-11
**分支**: main (未提交改动)
**编译状态**: 通过 (0 errors, 57 warnings)
**测试状态**: 141 tests passed, 0 failed

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

### 2. Gateway 硬编码修复

- `prompt_cache: false` → 改为使用 `model_config.prompt_cache`
- `max_output_tokens: 4096` → 改为使用 `model_config.max_tokens.unwrap_or(4096)`
- `max_turns: 20` → 提高到 `50` (压测发现 20 轮不够)
- `run_task_once` 新增 `sandbox` 参数
- `--onlyrun` 参数恢复兼容

### 3. ExecPolicy 安全加固

- 新增 `default_policy` 字段 (默认 Allow，向后兼容)
- 路径匹配改进: 支持完整路径 (`/usr/bin/rm`) 的文件名提取
- 参数匹配改进: 逐个参数检查，防止 `rm -r -f /` 绕过 `["-rf", "/"]` 规则
- config.rs 测试安全: 添加 Mutex 序列化 + `unsafe` 块 (Rust 1.80+ set_var 变为 unsafe)

---

## 已知未解决问题

### P0 — 必须在下次压测前解决

无。上次 P0 问题 (config provider 默认值、WriteTool 父目录创建) 已在 beta 分支修复。

### P1 — 高优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 1 | Token Budget 超限不终止 | 未修复 | budget=32K 但实际用了 395K，循环继续 |
| 2 | Context Compaction 效果差 | 未修复 | 51→1 条消息只释放 5K tokens |
| 3 | Prompt Cache 在 --onlyrun 路径未集成 | 部分修复 | prompt_cache 现在正确传递，但 --onlyrun 路径是否使用 PromptBuilder 待验证 |
| 4 | Config 重复加载 (每次工具调用) | 未修复 | beta 分支的 OnceLock 修复尚未合入 main |

### P2 — 中优先级

| # | 问题 | 状态 | 备注 |
|---|------|------|------|
| 5 | 57 个 deprecated API warnings | 未修复 | `core::audit` → `crate::audit`，`session::SessionEntry` → `UnifiedStore` |
| 6 | Agent 倾向用 bash 替代 read/glob | 未修复 | 可能是 system prompt 引导不足 |
| 7 | `pub mod core` 与 Rust 内置 core 冲突 | 未修复 | LLM 生成质量问题，可在 system prompt 中提示 |
| 8 | `get_tool_calls_for_turn` dead code | 未修复 | session/store.rs:441 |
| 9 | Docker 沙箱 Windows 兼容性 | 待验证 | Docker Desktop daemon 未运行时的降级逻辑已实现 |

---

## 分支状态

| 分支 | 最新 Commit | 内容 |
|------|-------------|------|
| main | 34083e1 + 未提交改动 | code review 修复 + 沙箱模式 + gateway 重构 |
| beta | d16295d | 架构改进 + UnifiedStore + 死代码清理 + 压测修复 |

**注意**: beta 分支的 OnceLock config cache 修复尚未合入 main。下次合并时需注意冲突。

---

## 下一步计划

1. 提交 main 分支当前改动
2. 无网络沙箱模式下重跑压测 (DeepSeek v4-flash)
3. 根据压测结果修复 P1 问题 (Token Budget 强制终止、Context Compaction 优化)
4. 合并 beta 分支改动到 main
5. 清理 deprecated API (57 warnings → 0)
