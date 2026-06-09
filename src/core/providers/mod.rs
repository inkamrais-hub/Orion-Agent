//! Provider 实现集合
//! 每个 Provider 是一个独立积木, 通过 feature flag 控制编译

#[cfg(feature = "openai-compat")]
pub mod openai_compat;

pub mod anthropic;
