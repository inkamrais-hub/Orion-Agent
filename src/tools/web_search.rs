//! Web Search Tool — Agent 联网搜索工具
//!
//! 三层搜索策略:
//!   1. API 模式 (企业): Google Custom Search API / Bing Search API
//!   2. 代理模式 (个人): Google 爬虫 + 配置代理 URL
//!   3. 兜底模式: DuckDuckGo (无需配置)

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use web_search::{SearchManager, SearchOptions, SearchApiConfig, ApiProvider, SearchApiEngine};
use web_search::engines::google::GoogleEngine;
use web_search::engines::duckduckgo::DuckDuckGoEngine;

use crate::tools::{Tool, ToolContext, ToolResult};

/// 搜索模式
#[derive(Debug, Clone)]
pub enum SearchMode {
    /// API 模式 (企业)
    Api {
        config: SearchApiConfig,
    },
    /// 代理模式 (个人)
    Proxy {
        proxy_url: String,
    },
    /// 直连模式 (兜底)
    Direct,
}

/// Web Search 工具
pub struct WebSearchTool {
    mode: SearchMode,
    /// 缓存的 HTTP Client，避免每次搜索重建连接
    client: Option<Arc<reqwest::Client>>,
}

impl Default for WebSearchTool {
    fn default() -> Self { Self::new() }
}

impl WebSearchTool {
    /// 创建直连模式 (DuckDuckGo)
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()
            .map(Arc::new);
        Self {
            mode: SearchMode::Direct,
            client,
        }
    }

    /// 创建代理模式
    pub fn with_proxy(proxy_url: &str) -> Self {
        let proxy = reqwest::Proxy::all(proxy_url).ok();
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36");
        if let Some(ref p) = proxy {
            builder = builder.proxy(p.clone());
        }
        let client = builder.build().ok().map(Arc::new);
        Self {
            mode: SearchMode::Proxy {
                proxy_url: proxy_url.to_string(),
            },
            client,
        }
    }

    /// 创建 API 模式
    pub fn with_api(config: SearchApiConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()
            .map(Arc::new);
        Self {
            mode: SearchMode::Api { config },
            client,
        }
    }

    /// 构建 SearchManager
    fn build_manager(&self) -> Option<SearchManager> {
        let mut manager = SearchManager::new();

        match &self.mode {
            SearchMode::Api { config } => {
                manager.register(Arc::new(SearchApiEngine::new(config.clone())));
                let name = match config.provider {
                    ApiProvider::Google => "google_api",
                    ApiProvider::Bing => "bing_api",
                };
                let _ = manager.set_default_engine(name);
            }
            SearchMode::Proxy { .. } => {
                let client = self.client.as_ref()?;
                manager.register(Arc::new(GoogleEngine::with_client(client.as_ref().clone())));
                manager.register(Arc::new(DuckDuckGoEngine::with_client(client.as_ref().clone())));
                let _ = manager.set_default_engine("google");
            }
            SearchMode::Direct => {
                manager.register(Arc::new(DuckDuckGoEngine::new()));
                let _ = manager.set_default_engine("duckduckgo");
            }
        }

        Some(manager)
    }

    /// 检测文本是否包含中文
    fn has_chinese(text: &str) -> bool {
        text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
    }

    /// 生成多语言查询变体
    fn multilingual_queries(query: &str) -> Vec<String> {
        let mut queries = vec![query.to_string()];

        if Self::has_chinese(query) {
            let english_words: String = query.chars().map(|c| {
                if c.is_ascii_alphanumeric() || c == ' ' { c } else { ' ' }
            }).collect::<String>();
            let english_words = english_words.trim();
            if !english_words.is_empty() && english_words != query {
                queries.push(english_words.to_string());
            }
        }

        queries
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Supports multiple search modes:\n\
         - API mode (enterprise): Google/Bing Search API for best quality\n\
         - Proxy mode (personal): Google crawler with configurable proxy\n\
         - Direct mode (fallback): DuckDuckGo, no configuration needed\n\
         Automatically searches in multiple languages and merges results."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query. Can be in any language."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 5)",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Tool("Missing required parameter: query".into()))?;

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        let manager = match self.build_manager() {
            Some(m) => m,
            None => return Ok(ToolResult {
                content: "HTTP client not available".into(),
                is_error: true,
                metadata: None,
            }),
        };
        let queries = Self::multilingual_queries(query);
        let mut all_results = Vec::new();
        let mut seen_urls = std::collections::HashSet::new();
        let mut engines_used = Vec::new();

        for q in &queries {
            let options = SearchOptions {
                max_results,
                ..SearchOptions::default()
            };

            match manager.search_all(q, &options).await {
                Ok(response) => {
                    if !engines_used.contains(&response.engine) {
                        engines_used.push(response.engine.clone());
                    }
                    for result in response.results {
                        let normalized_url = normalize_url(&result.url);
                        if seen_urls.insert(normalized_url) {
                            all_results.push(result);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Search failed for '{}': {}", q, e);
                }
            }
        }

        all_results.truncate(max_results);

        if all_results.is_empty() {
            return Ok(ToolResult {
                content: format!("No results found for: \"{}\"", query),
                is_error: false,
                metadata: None,
            });
        }

        let mode_str = match &self.mode {
            SearchMode::Api { config } => format!("API ({})", match config.provider {
                ApiProvider::Google => "Google",
                ApiProvider::Bing => "Bing",
            }),
            SearchMode::Proxy { .. } => "Google (proxy)".to_string(),
            SearchMode::Direct => "DuckDuckGo (direct)".to_string(),
        };

        let mut output = format!("Search results for: \"{}\"\n", query);
        output.push_str(&format!("Mode: {} | Engines: {} | {} results\n", mode_str, engines_used.join(", "), all_results.len()));
        if queries.len() > 1 {
            output.push_str(&format!("Multi-language queries: {}\n", queries.join(" | ")));
        }
        output.push_str(&format!("{:-<60}\n", ""));

        for (i, result) in all_results.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, result.title));
            output.push_str(&format!("   URL: {}\n", result.url));
            if !result.snippet.is_empty() {
                output.push_str(&format!("   {}\n", result.snippet));
            }
            output.push('\n');
        }

        Ok(ToolResult {
            content: output,
            is_error: false,
            metadata: None,
        })
    }
}

/// 标准化 URL 用于去重
fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let url = url.trim_end_matches('/');
    let url = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")).unwrap_or(url);
    url.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_chinese() {
        assert!(WebSearchTool::has_chinese("你好世界"));
        assert!(WebSearchTool::has_chinese("Rust 编程"));
        assert!(!WebSearchTool::has_chinese("Rust programming"));
    }

    #[test]
    fn test_multilingual_queries_chinese() {
        let queries = WebSearchTool::multilingual_queries("Rust 编程语言");
        assert!(queries.len() >= 1);
        assert_eq!(queries[0], "Rust 编程语言");
    }

    #[test]
    fn test_multilingual_queries_english() {
        let queries = WebSearchTool::multilingual_queries("Rust programming");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0], "Rust programming");
    }

    #[test]
    fn test_normalize_url() {
        assert_eq!(normalize_url("https://Example.com/path/"), "example.com/path");
        assert_eq!(normalize_url("http://example.com/path"), "example.com/path");
    }
}
