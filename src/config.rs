//! 统一配置系统
//!
//! 唯一配置源: ~/.orion/config.yaml
//! 支持: ${ENV_VAR} 环境变量替换、默认值填充、模型能力声明

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use crate::model::ModelConfig;

// ============================================================
//  顶层配置 (唯一配置源)
// ============================================================

/// Orion Agent 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrionConfig {
    // ── 模型 ──
    /// 默认使用的模型名称
    pub default_model: String,
    /// 已注册的模型列表
    pub models: Vec<ModelConfig>,

    // ── 子系统 ──
    pub cache: CacheConfig,
    pub orchestrator: OrchestratorConfig,
    pub agent: AgentLoopConfig,
    pub audit: AuditConfig,
    pub cli: CliConfig,
    pub gateway: GatewayConfig,

    // ── Docker 沙箱 ──
    #[serde(default)]
    pub docker: DockerConfig,

    // ── MCP ──
    #[serde(default)]
    pub mcp_servers: Vec<crate::tools::mcp::McpServerConfig>,
}

impl Default for OrionConfig {
    fn default() -> Self {
        Self {
            default_model: "deepseek-chat".into(),
            models: vec![ModelConfig::default()],
            cache: CacheConfig::default(),
            orchestrator: OrchestratorConfig::default(),
            agent: AgentLoopConfig::default(),
            audit: AuditConfig::default(),
            cli: CliConfig::default(),
            gateway: GatewayConfig::default(),
            docker: DockerConfig::default(),
            mcp_servers: Vec::new(),
        }
    }
}

impl OrionConfig {
    /// 从默认路径加载 (~/.orion/config.yaml)
    pub fn load() -> Self {
        if let Some(path) = config_file_path() {
            if path.exists() {
                match Self::load_from(&path) {
                    Ok(config) => {
                        tracing::info!(path = %path.display(), "配置加载成功");
                        return config;
                    }
                    Err(e) => {
                        tracing::error!(path = %path.display(), error = %e, "配置解析失败，使用默认配置");
                    }
                }
            }
        }
        Self::default()
    }

    /// 缓存版本: 只加载一次，后续直接返回引用
    pub fn load_cached() -> &'static OrionConfig {
        static CONFIG_CACHE: std::sync::OnceLock<OrionConfig> = std::sync::OnceLock::new();
        CONFIG_CACHE.get_or_init(Self::load)
    }

    /// 从指定路径加载
    pub fn load_from(path: impl AsRef<Path>) -> crate::Result<Self> {
        let raw = std::fs::read_to_string(path.as_ref()).map_err(crate::Error::Io)?;
        let substituted = substitute_env(&raw);
        let config: OrionConfig = serde_yaml::from_str(&substituted)
            .map_err(|e| crate::Error::Config(format!("配置解析失败: {}", e)))?;
        Ok(config)
    }

    /// 获取当前活跃的模型配置
    pub fn active_model(&self) -> ModelConfig {
        self.models.iter()
            .find(|m| m.name == self.default_model)
            .cloned()
            .or_else(|| self.models.first().cloned())
            .unwrap_or_default()
    }

    /// 保存配置到默认路径
    pub fn save(&self) -> crate::Result<()> {
        if let Some(path) = config_file_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = serde_yaml::to_string(self)
                .map_err(|e| crate::Error::Config(format!("序列化失败: {}", e)))?;
            std::fs::write(path, content)?;
        }
        Ok(())
    }
}

/// 获取默认配置文件路径
/// 优先 ~/.config/orion/config.yaml，兼容旧路径 ~/.orion/config.yaml
pub fn config_file_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let new_path = PathBuf::from(&home).join(".config").join("orion").join("config.yaml");
    let old_path = PathBuf::from(&home).join(".orion").join("config.yaml");
    if new_path.exists() {
        Some(new_path)
    } else if old_path.exists() {
        Some(old_path)
    } else {
        Some(new_path) // 默认用新路径
    }
}

/// 获取数据目录路径 (sessions, memories, etc.)
/// 优先 ~/.config/orion/，兼容旧路径 ~/.orion/
pub fn data_dir_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    let new_path = PathBuf::from(&home).join(".config").join("orion");
    let old_path = PathBuf::from(&home).join(".orion");
    // 如果旧目录存在且新目录不存在，使用旧目录
    if old_path.exists() && !new_path.exists() {
        old_path
    } else {
        new_path
    }
}

// ============================================================
//  子配置模块
// ============================================================

/// 缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    pub l1_max_entries: u64,
    pub l1_ttl_secs: u64,
    pub l2_max_entries: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { l1_max_entries: 2048, l1_ttl_secs: 300, l2_max_entries: 128 }
    }
}

/// 编排器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    pub mode: String,
    pub max_workers: usize,
    pub max_rounds: usize,
    pub coordinator_model: Option<String>,
    pub worker_model: Option<String>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            mode: "sequential".into(),
            max_workers: 3,
            max_rounds: 10,
            coordinator_model: None,
            worker_model: None,
        }
    }
}

/// Agent 循环配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLoopConfig {
    pub max_turns: u64,
    pub max_tool_calls: u64,
    pub token_budget: u64,
    /// 压缩触发比例 (0.0-1.0)，当 token 使用量超过 context_window * compaction_ratio 时触发压缩
    #[serde(default = "default_compaction_ratio")]
    pub compaction_ratio: f64,
}

fn default_compaction_ratio() -> f64 {
    0.80
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            max_turns: 50,
            max_tool_calls: 30,
            token_budget: 128_000,
            compaction_ratio: default_compaction_ratio(),
        }
    }
}

/// 审计配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    pub enabled: bool,
    pub path: String,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self { enabled: true, path: "./audit.log".into() }
    }
}

/// CLI 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    pub prompt: String,
    pub auto_resume: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self { prompt: "orion> ".into(), auto_resume: true }
    }
}

/// Gateway 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    pub log_level: String,
    pub log_file: Option<String>,
    pub api_port: u16,
    pub max_agents: usize,
    pub session_dir: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            log_file: None,
            api_port: 8080,
            max_agents: 10,
            session_dir: ".orion/sessions".into(),
        }
    }
}

/// Docker 沙箱配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerConfig {
    pub enabled: bool,
    pub image: String,
    pub workdir: String,
    pub auto_pull: bool,
    /// 网络模式: "none", "host", "bridge"
    pub network: String,
    pub memory_limit: String,
    pub cpu_limit: String,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            image: "ubuntu:22.04".into(),
            workdir: "/workspace".into(),
            auto_pull: true,
            network: "none".into(),
            memory_limit: "512m".into(),
            cpu_limit: "1".into(),
        }
    }
}

// ============================================================
//  环境变量替换
// ============================================================

fn substitute_env(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next();
            let mut var_name = String::new();
            let mut found = false;
            for nc in chars.by_ref() {
                if nc == '}' { found = true; break; }
                var_name.push(nc);
            }
            if found {
                match std::env::var(&var_name) {
                    Ok(val) => result.push_str(&val),
                    Err(_) => result.push_str(&format!("${{{}}}", var_name)),
                }
            } else {
                result.push('$');
                result.push('{');
                result.push_str(&var_name);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // 序列化环境变量测试，防止并行冲突
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn substitute_basic_env_var() {
        let _lock = ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("ORION_TEST_KEY", "secret123"); }
        let result = substitute_env("api_key: ${ORION_TEST_KEY}");
        assert_eq!(result, "api_key: secret123");
        unsafe { std::env::remove_var("ORION_TEST_KEY"); }
    }

    #[test]
    fn substitute_missing_env_var_keeps_placeholder() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let result = substitute_env("key: ${NONEXISTENT_VAR_XYZ}");
        assert_eq!(result, "key: ${NONEXISTENT_VAR_XYZ}");
    }

    #[test]
    fn substitute_no_env_vars() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let result = substitute_env("plain text without vars");
        assert_eq!(result, "plain text without vars");
    }

    #[test]
    fn substitute_unclosed_brace() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let result = substitute_env("key: ${unclosed");
        assert_eq!(result, "key: ${unclosed");
    }

    #[test]
    fn substitute_multiple_vars() {
        let _lock = ENV_MUTEX.lock().unwrap();
        unsafe { std::env::set_var("ORION_A", "hello"); }
        unsafe { std::env::set_var("ORION_B", "world"); }
        let result = substitute_env("${ORION_A} ${ORION_B}");
        assert_eq!(result, "hello world");
        unsafe { std::env::remove_var("ORION_A"); }
        unsafe { std::env::remove_var("ORION_B"); }
    }

    #[test]
    fn substitute_dollar_sign_without_brace() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let result = substitute_env("cost is $5");
        assert_eq!(result, "cost is $5");
    }
}
