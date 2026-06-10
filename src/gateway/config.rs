//! Gateway 配置 — 重导出统一配置
//!
//! 所有配置统一从 crate::config 模块加载

pub use crate::config::{OrionConfig, GatewayConfig, config_file_path};
pub use crate::config::OrionConfig as Config;

use serde::{Deserialize, Serialize};

/// SubAgent 模型策略
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type")]
pub enum SubAgentModelPolicy {
    #[serde(rename = "inherit")]
    #[default]
    Inherit,
    #[serde(rename = "custom")]
    Custom {
        model: String,
        endpoint: String,
        api_key: Option<String>,
    },
}
