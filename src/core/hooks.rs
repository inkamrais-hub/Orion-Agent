//! Hook 系统 (Layer 4)
//!
//! 工具执行前后的拦截/修改/记录机制
//!
//! 设计原则:
//! - 声明式配置 (YAML)，无需写代码
//! - 防死循环 (每工具每轮最多触发 N 次)
//! - 日志同步 (所有 Hook 执行记录到审计 + 日志)
//! - 积木化 (每个 Hook 独立，可按需组合)
//!
//! Hook 配置格式 (.orion/hooks.yaml):
//! ```yaml
//! hooks:
//!   - name: block-dangerous
//!     point: BeforeTool
//!     match:
//!       tool: "bash"
//!       pattern: "rm -rf"
//!     action: Block("危险命令被拦截")
//!
//!   - name: audit-writes
//!     point: AfterTool
//!     match:
//!       tool: ["write", "edit"]
//!     action: Continue
//!     log: true
//!
//!   - name: retry-on-error
//!     point: OnError
//!     match:
//!       tool: "bash"
//!     action: Retry
//!     max_retries: 2
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::collections::HashMap;

// ── Hook 配置类型 ──────────────────────────────────────────

/// Hook 触发点
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum HookPoint {
    /// 工具执行前 (可拦截/修改输入)
    BeforeTool,
    /// 工具执行后 (可修改结果/记录)
    AfterTool,
    /// 工具出错时 (可重试/降级)
    OnError,
}

/// Hook 匹配条件
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookMatch {
    /// 工具名 (单个或多个)
    #[serde(default)]
    pub tool: MatchValue,
    /// 输入模式匹配 (正则)
    #[serde(default)]
    pub pattern: Option<String>,
}

/// 匹配值 (支持单个字符串或数组)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MatchValue {
    Single(String),
    Multiple(Vec<String>),
    None,
}

impl Default for MatchValue {
    fn default() -> Self {
        MatchValue::None
    }
}

impl MatchValue {
    pub fn matches(&self, value: &str) -> bool {
        match self {
            MatchValue::Single(s) => s == value || s == "*",
            MatchValue::Multiple(v) => v.iter().any(|s| s == value || s == "*"),
            MatchValue::None => true, // 未指定 = 匹配所有
        }
    }
}

/// Hook 动作
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookAction {
    /// 继续执行
    Continue,
    /// 阻断执行，返回原因
    Block(String),
    /// 重试 (仅 OnError 有效)
    Retry,
}

/// Hook 配置文件格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfigRaw {
    pub name: String,
    pub point: HookPoint,
    #[serde(default)]
    pub match_config: Option<HookMatch>,
    pub action: String,
    #[serde(default)]
    pub action_message: Option<String>,
    #[serde(default)]
    pub log: bool,
    #[serde(default)]
    pub max_retries: Option<u32>,
}

/// Hook 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Hook 名称
    pub name: String,
    /// 触发点
    pub point: HookPoint,
    /// 匹配条件
    #[serde(default)]
    pub match_: HookMatch,
    /// 动作 (Continue / Block / Retry)
    pub action: String,
    /// 动作消息 (Block 时的原因)
    #[serde(default)]
    pub action_message: Option<String>,
    /// 是否记录到审计日志
    #[serde(default)]
    pub log: bool,
    /// 最大重试次数 (仅 OnError 有效)
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_retries() -> u32 {
    2
}

/// Hook 配置文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksFile {
    pub hooks: Vec<HookConfig>,
}

// ── Hook 执行结果 ──────────────────────────────────────────

/// Hook 执行结果
#[derive(Debug, Clone)]
pub enum HookResult {
    /// 继续执行
    Continue,
    /// 阻断执行
    Block(String),
    /// 请求重试
    Retry,
}

// ── Hook 引擎 ──────────────────────────────────────────────

/// Hook 引擎
pub struct HookEngine {
    /// 已加载的 Hook 配置
    hooks: Vec<HookConfig>,
    /// 执行计数器 (防止死循环): (hook_name, tool_name) -> count
    counters: HashMap<(String, String), u32>,
    /// 每工具每轮最大 Hook 触发次数
    max_per_round: u32,
}

impl HookEngine {
    /// 创建空的 Hook 引擎
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            counters: HashMap::new(),
            max_per_round: 5,
        }
    }

    /// 从配置文件加载 Hook
    pub fn load_from_file(path: &PathBuf) -> Self {
        let mut engine = Self::new();

        if !path.exists() {
            log_debug!("hook", "Hook 配置文件不存在: {}", path.display());
            return engine;
        }

        match std::fs::read_to_string(path) {
            Ok(content) => {
                match serde_yaml::from_str::<HooksFile>(&content) {
                    Ok(file) => {
                        engine.hooks = file.hooks;
                        log_info!("hook", "加载 {} 个 Hook 配置", engine.hooks.len());
                    }
                    Err(e) => {
                        log_warn!("hook", "Hook 配置解析失败: {}", e);
                    }
                }
            }
            Err(e) => {
                log_warn!("hook", "Hook 配置读取失败: {}", e);
            }
        }

        engine
    }

    /// 从默认路径加载 (.orion/hooks.yaml)
    pub fn load_default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let path = cwd.join(".orion").join("hooks.yaml");
        Self::load_from_file(&path)
    }

    /// 执行 BeforeTool Hook
    pub fn run_before(&mut self, tool_name: &str, input: &str) -> HookResult {
        self.run_hooks(&HookPoint::BeforeTool, tool_name, input)
    }

    /// 执行 AfterTool Hook
    pub fn run_after(&mut self, tool_name: &str, output: &str) -> HookResult {
        self.run_hooks(&HookPoint::AfterTool, tool_name, output)
    }

    /// 执行 OnError Hook
    pub fn run_on_error(&mut self, tool_name: &str, error: &str) -> HookResult {
        self.run_hooks(&HookPoint::OnError, tool_name, error)
    }

    /// 重置计数器 (每轮对话开始时调用)
    pub fn reset_counters(&mut self) {
        self.counters.clear();
    }

    /// 获取已加载的 Hook 数量
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    // ── 内部方法 ────────────────────────────────────────

    fn run_hooks(&mut self, point: &HookPoint, tool_name: &str, data: &str) -> HookResult {
        let matching: Vec<HookConfig> = self.hooks.iter()
            .filter(|h| h.point == *point)
            .filter(|h| h.match_.tool.matches(tool_name))
            .filter(|h| {
                // 模式匹配
                if let Some(ref pattern) = h.match_.pattern {
                    if let Ok(re) = regex::Regex::new(pattern) {
                        re.is_match(data)
                    } else {
                        true // 正则无效则跳过匹配
                    }
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        for hook in matching {
            // 检查防死循环
            let key = (hook.name.clone(), tool_name.to_string());
            let count = self.counters.entry(key).or_insert(0);
            *count += 1;

            if *count > hook.max_retries.max(self.max_per_round) {
                log_warn!("hook", "Hook '{}' 触发次数超限 ({})，跳过", hook.name, count);
                continue;
            }

            // 记录到审计日志
            if hook.log {
                log_info!("hook", "Hook '{}' 触发: {} on {}", hook.name, point_str(&hook.point), tool_name);
            }

            // 执行动作
            match hook.action.as_str() {
                "Continue" => {
                    if hook.log {
                        log_debug!("hook", "Hook '{}': Continue", hook.name);
                    }
                    // 继续检查下一个 Hook
                }
                "Block" => {
                    let reason = hook.action_message.clone().unwrap_or_else(|| "被 Hook 拦截".into());
                    log_warn!("hook", "Hook '{}' 阻断: {}", hook.name, reason);
                    return HookResult::Block(reason);
                }
                "Retry" => {
                    if *point == HookPoint::OnError {
                        log_info!("hook", "Hook '{}' 请求重试 (第 {} 次)", hook.name, count);
                        return HookResult::Retry;
                    } else {
                        log_warn!("hook", "Hook '{}' 的 Retry 动作仅在 OnError 点有效", hook.name);
                    }
                }
                _ => {
                    log_warn!("hook", "Hook '{}': 未知动作 '{}'", hook.name, hook.action);
                }
            }
        }

        HookResult::Continue
    }
}

impl Default for HookEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn point_str(point: &HookPoint) -> &'static str {
    match point {
        HookPoint::BeforeTool => "BeforeTool",
        HookPoint::AfterTool => "AfterTool",
        HookPoint::OnError => "OnError",
    }
}

// ── 依赖 ──────────────────────────────────────────────────

use crate::{log_info, log_warn, log_debug};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_engine_empty() {
        let engine = HookEngine::new();
        assert!(engine.is_empty());
        assert_eq!(engine.len(), 0);
    }

    #[test]
    fn test_match_value() {
        assert!(MatchValue::Single("*".into()).matches("anything"));
        assert!(MatchValue::Single("bash".into()).matches("bash"));
        assert!(!MatchValue::Single("bash".into()).matches("write"));
        assert!(MatchValue::Multiple(vec!["bash".into(), "write".into()]).matches("bash"));
        assert!(MatchValue::None.matches("anything"));
    }

    #[test]
    fn test_hook_block() {
        let mut engine = HookEngine::new();
        engine.hooks.push(HookConfig {
            name: "test-block".into(),
            point: HookPoint::BeforeTool,
            match_: HookMatch {
                tool: MatchValue::Single("bash".into()),
                pattern: None,
            },
            action: "Block".into(),
            action_message: Some("blocked".into()),
            log: false,
            max_retries: 1,
        });

        let result = engine.run_before("bash", "rm -rf /");
        assert!(matches!(result, HookResult::Block(_)));
    }

    #[test]
    fn test_hook_continue() {
        let mut engine = HookEngine::new();
        engine.hooks.push(HookConfig {
            name: "test-continue".into(),
            point: HookPoint::BeforeTool,
            match_: HookMatch {
                tool: MatchValue::Single("bash".into()),
                pattern: None,
            },
            action: "Continue".into(),
            action_message: None,
            log: false,
            max_retries: 1,
        });

        let result = engine.run_before("bash", "ls");
        assert!(matches!(result, HookResult::Continue));
    }
}
