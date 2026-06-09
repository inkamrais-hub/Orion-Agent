//! 模型抽象层
//!
//! 用户配置驱动的 Provider 管理

pub mod router;

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// 模型配置 (用户在 config.yaml 中定义)
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// 模型名称 (如 "deepseek-chat", "claude-3-opus")
    pub name: String,
    /// Provider 类型 (如 "openai", "anthropic", "deepseek")
    pub provider: String,
    /// API 端点
    pub endpoint: String,
    /// API Key (可选, 也可从环境变量读取)
    /// 跳过 Serialize 防止序列化泄露
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    /// 最大输出 token 数
    pub max_tokens: Option<u32>,
    /// 模型支持的模态: ["text"], ["text", "vision"], ["text", "vision", "audio"], ["omni"]
    #[serde(default = "default_modalities")]
    pub modalities: Vec<String>,
    /// 是否支持思考模式 (DeepSeek/Qwen/Kimi 等)
    #[serde(default)]
    pub thinking: bool,
    /// 是否支持 prompt caching
    #[serde(default)]
    pub prompt_cache: bool,
    /// 最大输入 token 数 (上下文窗口)
    pub max_input_tokens: Option<u64>,
    /// HTTP 代理地址 (可选)
    pub proxy: Option<String>,
    /// HTTP 超时秒数
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_modalities() -> Vec<String> { vec!["text".into()] }
fn default_timeout() -> u64 { 120 }

/// 手动实现 Debug，隐藏 api_key 防止泄露
impl std::fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelConfig")
            .field("name", &self.name)
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "***"))
            .field("max_tokens", &self.max_tokens)
            .field("modalities", &self.modalities)
            .field("thinking", &self.thinking)
            .field("prompt_cache", &self.prompt_cache)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("proxy", &self.proxy)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            name: "deepseek-chat".into(),
            provider: "openai".into(),
            endpoint: "https://api.deepseek.com".into(),
            api_key: None,
            max_tokens: Some(4096),
            modalities: default_modalities(),
            thinking: false,
            prompt_cache: false,
            max_input_tokens: Some(128_000),
            proxy: None,
            timeout_secs: 120,
        }
    }
}

impl ModelConfig {
    /// 默认模型配置 (用于 fallback)
    pub const DEFAULT: ModelConfig = ModelConfig {
        name: String::new(),
        provider: String::new(),
        endpoint: String::new(),
        api_key: None,
        max_tokens: None,
        modalities: vec![],
        thinking: false,
        prompt_cache: false,
        max_input_tokens: None,
        proxy: None,
        timeout_secs: 120,
    };

    /// 是否支持指定模态
    pub fn supports_modality(&self, modality: &str) -> bool {
        self.modalities.iter().any(|m| m == modality || m == "omni")
    }
    /// 是否支持视觉 (图片)
    pub fn supports_vision(&self) -> bool {
        self.supports_modality("vision")
    }
}

/// 模型注册表 - 纯配置驱动, 不做自动路由
pub struct ModelRegistry {
    configs: HashMap<String, ModelConfig>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self { configs: HashMap::new() }
    }

    /// 从配置加载
    pub fn from_config(models: Vec<ModelConfig>) -> Self {
        let mut registry = Self::new();
        for model in models {
            registry.configs.insert(model.name.clone(), model);
        }
        registry
    }

    /// 注册模型配置
    pub fn register(&mut self, config: ModelConfig) {
        self.configs.insert(config.name.clone(), config);
    }

    /// 获取模型配置
    pub fn get(&self, name: &str) -> Option<&ModelConfig> {
        self.configs.get(name)
    }

    /// 列出所有模型
    pub fn list(&self) -> Vec<&ModelConfig> {
        self.configs.values().collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
