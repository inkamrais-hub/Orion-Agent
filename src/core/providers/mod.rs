//! Provider 实现集合
//! 每个 Provider 是一个独立积木, 通过 feature flag 控制编译

#[cfg(feature = "openai-compat")]
pub mod openai_compat;

pub mod anthropic;

use crate::core::provider::Provider;
use crate::model::ModelConfig;

/// 根据 ModelConfig 中的 provider 字段创建对应的 Provider 实例
///
/// 支持的 provider 类型:
/// - `"openai"` / `"openai-compat"` → OpenAICompatProvider
/// - `"anthropic"` → AnthropicProvider
///
/// API Key 优先级: config.api_key > LLM_API_KEY 环境变量
pub fn create_provider(config: &ModelConfig) -> Box<dyn Provider> {
    let api_key = config.api_key.as_deref()
        .filter(|k| !k.is_empty())
        .map(|s| s.to_string())
        .or_else(|| std::env::var("LLM_API_KEY").ok())
        .unwrap_or_default();
    let model = config.effective_model();
    match config.provider.as_str() {
        "anthropic" => Box::new(anthropic::AnthropicProvider::new(
            &config.endpoint,
            &api_key,
            model,
        )),
        // 默认走 OpenAI 兼容协议
        _ => Box::new(openai_compat::OpenAICompatProvider::new(
            &config.endpoint,
            &api_key,
            model,
        )),
    }
}
