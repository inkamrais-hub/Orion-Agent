//! 三段式系统提示词构建器 — 优化 Prompt Cache 命中率
//!
//! 将系统提示拆分为三段:
//! 1. Static Block (角色/原则/编码规范) — 极少变更, 缓存命中率最高
//! 2. Tool Block (工具 Schema 描述) — 工具增减时变更
//! 3. Dynamic Block (上下文/Memory/项目信息) — 每轮可能变更
//!
//! API 级 Prompt Cache (Anthropic/OpenAI) 基于前缀匹配,
//! 静态部分放在最前可最大化缓存复用。

/// 三段式提示词段
#[derive(Debug, Clone)]
pub struct PromptSection {
    /// 段名 (用于调试/日志)
    pub name: &'static str,
    /// 段内容
    pub content: String,
    /// 是否可缓存 (Static/Tool = true, Dynamic = false)
    pub cacheable: bool,
}

/// 三段式提示词构建器
pub struct PromptBuilder {
    /// 静态块: 角色定义、核心原则、编码规范
    static_block: String,
    /// 工具块: 工具描述/Schema (可选, 有些场景不需要)
    tool_block: Option<String>,
    /// 动态块: 上下文、Memory、项目信息
    dynamic_block: String,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            static_block: String::new(),
            tool_block: None,
            dynamic_block: String::new(),
        }
    }

    /// 设置静态块 (角色 + 原则 + 编码规范)
    pub fn static_block(mut self, content: impl Into<String>) -> Self {
        self.static_block = content.into();
        self
    }

    /// 设置工具块
    pub fn tool_block(mut self, content: impl Into<String>) -> Self {
        self.tool_block = Some(content.into());
        self
    }

    /// 设置动态块 (Memory + 上下文)
    pub fn dynamic_block(mut self, content: impl Into<String>) -> Self {
        self.dynamic_block = content.into();
        self
    }

    /// 追加动态内容 (不覆盖已有)
    pub fn append_dynamic(mut self, content: &str) -> Self {
        if !self.dynamic_block.is_empty() {
            self.dynamic_block.push('\n');
        }
        self.dynamic_block.push_str(content);
        self
    }

    /// 构建完整提示词 (三段拼接)
    pub fn build(&self) -> String {
        let mut parts = Vec::new();

        if !self.static_block.is_empty() {
            parts.push(self.static_block.clone());
        }

        if let Some(ref tools) = self.tool_block {
            if !tools.is_empty() {
                parts.push(tools.clone());
            }
        }

        if !self.dynamic_block.is_empty() {
            parts.push(self.dynamic_block.clone());
        }

        parts.join("\n\n")
    }

    /// 构建分段列表 (供 Anthropic cache_control 标记使用)
    pub fn build_sections(&self) -> Vec<PromptSection> {
        let mut sections = Vec::new();

        if !self.static_block.is_empty() {
            sections.push(PromptSection {
                name: "static",
                content: self.static_block.clone(),
                cacheable: true,
            });
        }

        if let Some(ref tools) = self.tool_block {
            if !tools.is_empty() {
                sections.push(PromptSection {
                    name: "tools",
                    content: tools.clone(),
                    cacheable: true,
                });
            }
        }

        if !self.dynamic_block.is_empty() {
            sections.push(PromptSection {
                name: "dynamic",
                content: self.dynamic_block.clone(),
                cacheable: false,
            });
        }

        sections
    }

    /// 估算静态块的 token 数 (粗略: 4字符 ≈ 1 token)
    pub fn static_token_estimate(&self) -> usize {
        self.static_block.len() / 4
    }

    /// 估算总 token 数
    pub fn total_tokens_estimate(&self) -> usize {
        let total_len = self.static_block.len()
            + self.tool_block.as_ref().map(|s| s.len()).unwrap_or(0)
            + self.dynamic_block.len();
        total_len / 4
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 从现有完整提示词中分离动态部分 (兼容旧代码)
///
/// 查找 `[Learned from previous sessions]` 或 `[Project Context]` 标记,
/// 将其后的内容视为动态块。
pub fn split_existing_prompt(full_prompt: &str) -> (String, String) {
    let dynamic_markers = [
        "[Learned from previous sessions]",
        "[Project Context]",
        "[Session Memory]",
        "[Dynamic Context]",
    ];

    for marker in &dynamic_markers {
        if let Some(pos) = full_prompt.find(marker) {
            let static_part = full_prompt[..pos].trim().to_string();
            let dynamic_part = full_prompt[pos..].trim().to_string();
            return (static_part, dynamic_part);
        }
    }

    // 未找到动态标记, 全部视为静态
    (full_prompt.to_string(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_builder_basic() {
        let prompt = PromptBuilder::new()
            .static_block("You are Orion, a coding assistant.")
            .dynamic_block("[Project] Rust project with Cargo")
            .build();

        assert!(prompt.contains("You are Orion"));
        assert!(prompt.contains("[Project]"));
    }

    #[test]
    fn test_prompt_builder_sections() {
        let builder = PromptBuilder::new()
            .static_block("Role: assistant")
            .tool_block("Tools: read, write, bash")
            .dynamic_block("Memory: user prefers edit tool");

        let sections = builder.build_sections();
        assert_eq!(sections.len(), 3);
        assert!(sections[0].cacheable);
        assert!(sections[1].cacheable);
        assert!(!sections[2].cacheable);
    }

    #[test]
    fn test_split_existing_prompt() {
        let full = "You are a helpful assistant.\n\n[Learned from previous sessions]\n- User prefers Rust";
        let (static_part, dynamic_part) = split_existing_prompt(full);
        assert!(static_part.contains("helpful assistant"));
        assert!(dynamic_part.contains("Learned from previous sessions"));
    }

    #[test]
    fn test_split_no_dynamic() {
        let full = "You are a helpful assistant. Be concise.";
        let (static_part, dynamic_part) = split_existing_prompt(full);
        assert_eq!(static_part, full);
        assert!(dynamic_part.is_empty());
    }
}
