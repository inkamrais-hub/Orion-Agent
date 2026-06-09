use moka::future::Cache as MokaCache;
use std::hash::Hash;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ToolCacheKey {
    pub tool_name: String,
    pub input_hash: u64,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub value: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub hit_count: u64,
}

pub struct L1ToolCache {
    inner: MokaCache<ToolCacheKey, CacheEntry>,
}

impl L1ToolCache {
    pub fn new(max_entries: u64, ttl_secs: u64) -> Self {
        Self {
            inner: MokaCache::builder()
                .max_capacity(max_entries)
                .time_to_live(Duration::from_secs(ttl_secs))
                .build(),
        }
    }
    pub async fn get(&self, key: &ToolCacheKey) -> Option<String> {
        self.inner.get(key).await.map(|entry| entry.value.clone())
    }
    pub async fn set(&self, key: ToolCacheKey, value: String) {
        self.inner.insert(key, CacheEntry {
            value, created_at: chrono::Utc::now(), hit_count: 0,
        }).await;
    }
    pub async fn invalidate(&self, key: &ToolCacheKey) {
        self.inner.invalidate(key).await;
    }
    pub fn invalidate_all(&self) { self.inner.invalidate_all(); }
    pub fn entry_count(&self) -> u64 { self.inner.entry_count() }

    /// 计算工具输入的标准化哈希 (路径标准化 + 输入去噪)
    pub fn compute_input_hash(tool_name: &str, input: &serde_json::Value) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        tool_name.hash(&mut hasher);

        // 标准化输入
        let normalized = Self::normalize_input(tool_name, input);
        let sorted = Self::sort_json_keys(&normalized);
        sorted.to_string().hash(&mut hasher);

        hasher.finish()
    }

    /// 标准化工具输入 (路径标准化 + 默认值补全)
    fn normalize_input(tool_name: &str, input: &serde_json::Value) -> serde_json::Value {
        let mut input = input.clone();

        // 路径标准化: read/write/edit/glob/grep 等工具的 path 字段
        if let Some(obj) = input.as_object_mut() {
            for key in &["path", "file_path", "dir", "directory"] {
                if let Some(serde_json::Value::String(path)) = obj.get_mut(*key) {
                    *path = Self::normalize_path(path);
                }
            }

            // 默认值补全: 避免 {"query":"rust"} 和 {"query":"rust","limit":10} 产生不同 hash
            match tool_name {
                "web_search" => {
                    obj.entry("max_results").or_insert(serde_json::Value::Number(5.into()));
                }
                "read" => {
                    obj.entry("encoding").or_insert(serde_json::Value::String("utf-8".into()));
                }
                "grep" => {
                    obj.entry("case_sensitive").or_insert(serde_json::Value::Bool(true));
                }
                _ => {}
            }

            // 移除无语义字段
            obj.remove("request_id");
            obj.remove("timestamp");
            obj.remove("_trace");
        }

        input
    }

    /// 路径标准化: 统一为绝对路径、小写、去掉尾部斜杠
    fn normalize_path(path: &str) -> String {
        let path = path.trim();

        // 尝试 canonicalize
        if let Ok(canonical) = std::fs::canonicalize(path) {
            return canonical.to_string_lossy().to_lowercase();
        }

        // canonicalize 失败时手动处理
        let path = path.replace('\\', "/");
        let path = path.trim_end_matches('/');

        // 去掉 ./ 前缀
        let path = path.strip_prefix("./").unwrap_or(path);

        path.to_lowercase()
    }

    /// JSON 键排序 (确保相同内容产生相同 hash)
    fn sort_json_keys(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted = serde_json::Map::new();
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                for key in keys {
                    sorted.insert(key.clone(), Self::sort_json_keys(&map[key]));
                }
                serde_json::Value::Object(sorted)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(Self::sort_json_keys).collect())
            }
            _ => value.clone(),
        }
    }
}

// ============================================================
//  缓存失效追踪
// ============================================================

/// 缓存失效向量
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheBreakVector {
    /// 系统提示变更
    SystemPromptChange,
    /// 工具定义变更
    ToolDefinitionChange,
    /// 新消息追加
    NewMessage,
    /// 工具结果变更
    ToolResultChange,
    /// 上下文压缩触发
    ContextCompaction,
    /// 模型切换
    ModelSwitch,
    /// 会话恢复
    SessionResume,
    /// 文件内容变更 (影响上下文)
    FileContentChange,
    /// 环境变量变更
    EnvVarChange,
    /// 用户偏好变更
    UserPreferenceChange,
}

/// 缓存失效追踪器
pub struct CacheBreakTracker {
    breaks: Vec<(CacheBreakVector, std::time::Instant)>,
}

impl CacheBreakTracker {
    pub fn new() -> Self {
        Self { breaks: Vec::new() }
    }

    /// 记录缓存失效事件
    pub fn record(&mut self, vector: CacheBreakVector) {
        self.breaks.push((vector, std::time::Instant::now()));
    }

    /// 获取最近的缓存失效事件
    pub fn recent_breaks(&self, within: std::time::Duration) -> Vec<&CacheBreakVector> {
        let cutoff = std::time::Instant::now() - within;
        self.breaks.iter()
            .filter(|(_, t)| *t > cutoff)
            .map(|(v, _)| v)
            .collect()
    }

    /// 检查是否需要重建缓存
    pub fn needs_rebuild(&self, since: std::time::Instant) -> bool {
        self.breaks.iter().any(|(_, t)| *t > since)
    }

    /// 清理旧记录
    pub fn cleanup(&mut self, older_than: std::time::Duration) {
        let cutoff = std::time::Instant::now() - older_than;
        self.breaks.retain(|(_, t)| *t > cutoff);
    }
}

impl Default for CacheBreakTracker {
    fn default() -> Self {
        Self::new()
    }
}

// Layer 2: Context snapshot cache
#[derive(Debug, Clone, Default)]
pub struct CacheBreakVectors {
    pub last_compaction: Option<chrono::DateTime<chrono::Utc>>,
    pub last_truncation: Option<chrono::DateTime<chrono::Utc>>,
    pub last_tool_call: Option<chrono::DateTime<chrono::Utc>>,
    pub file_reads: u64,
    pub agent_switches: u64,
    pub session_memory_updates: u64,
}

impl CacheBreakVectors {
    pub fn is_stale(&self, snapshot: &Self) -> bool {
        self.last_compaction != snapshot.last_compaction
            || self.last_truncation != snapshot.last_truncation
            || self.last_tool_call != snapshot.last_tool_call
            || self.file_reads != snapshot.file_reads
            || self.agent_switches != snapshot.agent_switches
            || self.session_memory_updates != snapshot.session_memory_updates
    }
}

#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    pub messages_count: usize,
    pub total_tokens: u64,
    pub vectors: CacheBreakVectors,
}

pub struct L2ContextCache {
    inner: MokaCache<String, ContextSnapshot>,
    /// 基于 system prompt hash 的上下文快照缓存
    prompt_cache: MokaCache<u64, ContextSnapshot>,
}

impl L2ContextCache {
    pub fn new(max_entries: u64) -> Self {
        Self {
            inner: MokaCache::builder()
                .max_capacity(max_entries)
                .time_to_live(Duration::from_secs(3600))
                .build(),
            prompt_cache: MokaCache::builder()
                .max_capacity(max_entries)
                .time_to_live(Duration::from_secs(3600))
                .build(),
        }
    }
    pub async fn get(&self, session_id: &str) -> Option<ContextSnapshot> {
        self.inner.get(session_id).await
    }
    pub async fn set(&self, session_id: String, snapshot: ContextSnapshot) {
        self.inner.insert(session_id, snapshot).await;
    }
    pub async fn invalidate(&self, session_id: &str) {
        self.inner.invalidate(session_id).await;
    }

    /// 按 system prompt hash 存储上下文快照
    pub async fn store_by_prompt(&self, prompt_hash: u64, snapshot: ContextSnapshot) {
        self.prompt_cache.insert(prompt_hash, snapshot).await;
    }

    /// 按 system prompt hash 获取缓存的上下文快照
    pub async fn get_by_prompt(&self, prompt_hash: u64) -> Option<ContextSnapshot> {
        self.prompt_cache.get(&prompt_hash).await
    }

    /// 使用 CacheBreakTracker 检查缓存是否需要失效
    ///
    /// 优化: 不再 invalidate_all，而是根据失效向量类型精准失效。
    pub async fn invalidate_if_stale(&self, tracker: &CacheBreakTracker, _snapshot_time: std::time::Instant) {
        let recent = tracker.recent_breaks(std::time::Duration::from_secs(300));
        for vector in recent {
            match vector {
                CacheBreakVector::SystemPromptChange => {
                    // System Prompt 变更: 只清空 prompt_cache，保留 session cache
                    self.prompt_cache.invalidate_all();
                }
                CacheBreakVector::ContextCompaction => {
                    // 上下文压缩: 只清空当前 session 的缓存
                    // (需要 session_id，这里清空全部 prompt_cache)
                    self.prompt_cache.invalidate_all();
                }
                CacheBreakVector::FileContentChange => {
                    // 文件变更: 只清空 prompt_cache (文件内容影响上下文)
                    self.prompt_cache.invalidate_all();
                }
                CacheBreakVector::ModelSwitch => {
                    // 模型切换: 清空所有缓存 (模型变了，所有上下文都失效)
                    self.inner.invalidate_all();
                    self.prompt_cache.invalidate_all();
                }
                _ => {
                    // 其他类型: 只清空 prompt_cache
                    self.prompt_cache.invalidate_all();
                }
            }
        }
    }
}

/// 计算 system prompt 的哈希值, 用于 L2 缓存 key
pub fn hash_prompt(prompt: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    prompt.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub l1_hits: u64,
    pub l1_misses: u64,
    pub l2_hits: u64,
    pub l2_misses: u64,
    pub l1_entries: u64,
    pub l2_entries: u64,
}

impl CacheStats {
    pub fn l1_hit_rate(&self) -> f64 {
        let total = self.l1_hits + self.l1_misses;
        if total == 0 { 0.0 } else { self.l1_hits as f64 / total as f64 }
    }
    pub fn l2_hit_rate(&self) -> f64 {
        let total = self.l2_hits + self.l2_misses;
        if total == 0 { 0.0 } else { self.l2_hits as f64 / total as f64 }
    }
    pub fn overall_hit_rate(&self) -> f64 {
        let total_hits = self.l1_hits + self.l2_hits;
        let total = self.l1_hits + self.l1_misses + self.l2_hits + self.l2_misses;
        if total == 0 { 0.0 } else { total_hits as f64 / total as f64 }
    }
}

// GlobalCache - shared across agents/sessions
pub struct GlobalCache {
    pub l1: Arc<L1ToolCache>,
    pub l2: Arc<L2ContextCache>,
    l1_hits: Arc<AtomicU64>,
    l1_misses: Arc<AtomicU64>,
    l2_hits: Arc<AtomicU64>,
    l2_misses: Arc<AtomicU64>,
    path_tracker: Arc<PathTracker>,
    /// API 级 prompt cache 命中次数
    prompt_cache_hits: Arc<AtomicU64>,
    /// API 级 prompt cache 节省的 tokens
    prompt_cache_tokens_saved: Arc<AtomicU64>,
}

struct PathTracker {
    paths: dashmap::DashMap<String, Vec<ToolCacheKey>>,
}

impl PathTracker {
    fn new() -> Self { Self { paths: dashmap::DashMap::new() } }
    fn track(&self, path: &str, key: ToolCacheKey) {
        self.paths.entry(path.to_string()).or_default().push(key);
    }
    fn get_keys(&self, path: &str) -> Vec<ToolCacheKey> {
        self.paths.get(path).map(|v| v.clone()).unwrap_or_default()
    }
    fn remove_path(&self, path: &str) { self.paths.remove(path); }
}

impl GlobalCache {
    pub fn new(l1_max: u64, l1_ttl: u64, l2_max: u64) -> Self {
        Self {
            l1: Arc::new(L1ToolCache::new(l1_max, l1_ttl)),
            l2: Arc::new(L2ContextCache::new(l2_max)),
            l1_hits: Arc::new(AtomicU64::new(0)),
            l1_misses: Arc::new(AtomicU64::new(0)),
            l2_hits: Arc::new(AtomicU64::new(0)),
            l2_misses: Arc::new(AtomicU64::new(0)),
            path_tracker: Arc::new(PathTracker::new()),
            prompt_cache_hits: Arc::new(AtomicU64::new(0)),
            prompt_cache_tokens_saved: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn from_config(config: &crate::config::CacheConfig) -> Self {
        Self::new(config.l1_max_entries, config.l1_ttl_secs, config.l2_max_entries)
    }

    pub async fn get_tool_result(&self, key: &ToolCacheKey) -> Option<String> {
        match self.l1.get(key).await {
            Some(val) => { self.l1_hits.fetch_add(1, Ordering::Relaxed); Some(val) }
            None => { self.l1_misses.fetch_add(1, Ordering::Relaxed); None }
        }
    }

    pub async fn set_tool_result(&self, key: ToolCacheKey, value: String, input: &serde_json::Value) {
        if let Some(path) = extract_path_from_input(&key.tool_name, input) {
            self.path_tracker.track(&path, key.clone());
        }
        self.l1.set(key, value).await;
    }

    pub async fn get_context_snapshot(&self, session_id: &str) -> Option<ContextSnapshot> {
        match self.l2.get(session_id).await {
            Some(val) => { self.l2_hits.fetch_add(1, Ordering::Relaxed); Some(val) }
            None => { self.l2_misses.fetch_add(1, Ordering::Relaxed); None }
        }
    }

    pub async fn set_context_snapshot(&self, session_id: String, snapshot: ContextSnapshot) {
        self.l2.set(session_id, snapshot).await;
    }

    /// 按 prompt hash 存储上下文快照
    pub async fn store_by_prompt(&self, prompt_hash: u64, snapshot: ContextSnapshot) {
        self.l2.store_by_prompt(prompt_hash, snapshot).await;
    }

    /// 按 prompt hash 获取缓存上下文
    pub async fn get_by_prompt(&self, prompt_hash: u64) -> Option<ContextSnapshot> {
        self.l2.get_by_prompt(prompt_hash).await
    }

    /// 使用 CacheBreakTracker 检查并失效缓存
    pub async fn invalidate_if_stale(&self, tracker: &CacheBreakTracker, snapshot_time: std::time::Instant) {
        self.l2.invalidate_if_stale(tracker, snapshot_time).await;
    }

    pub async fn invalidate_for_path(&self, path: &str) {
        let keys = self.path_tracker.get_keys(path);
        for key in keys {
            self.l1.invalidate(&key).await;
        }
        self.path_tracker.remove_path(path);
    }

    pub fn invalidate_all_l1(&self) { self.l1.invalidate_all(); }

    pub fn report(&self) -> CacheReport {
        CacheReport {
            l1_hits: self.l1_hits.load(Ordering::Relaxed),
            l1_misses: self.l1_misses.load(Ordering::Relaxed),
            l2_hits: self.l2_hits.load(Ordering::Relaxed),
            l2_misses: self.l2_misses.load(Ordering::Relaxed),
            l1_entries: self.l1.entry_count(),
            l2_entries: 0,
            tracked_paths: self.path_tracker.paths.len(),
            prompt_cache_hits: self.prompt_cache_hits.load(Ordering::Relaxed),
            prompt_cache_tokens_saved: self.prompt_cache_tokens_saved.load(Ordering::Relaxed),
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            l1_hits: self.l1_hits.load(Ordering::Relaxed),
            l1_misses: self.l1_misses.load(Ordering::Relaxed),
            l2_hits: self.l2_hits.load(Ordering::Relaxed),
            l2_misses: self.l2_misses.load(Ordering::Relaxed),
            l1_entries: self.l1.entry_count(),
            l2_entries: 0,
        }
    }

    /// 记录 API 级别的 prompt cache 命中
    pub fn record_prompt_cache(&self, hit: bool, tokens_saved: u64) {
        if hit {
            self.prompt_cache_hits.fetch_add(1, Ordering::Relaxed);
        }
        self.prompt_cache_tokens_saved.fetch_add(tokens_saved, Ordering::Relaxed);
    }

    /// 完整缓存报告 (含 API 级 + 工具级)
    pub fn full_report(&self) -> String {
        let report = self.report();
        let api_hits = self.prompt_cache_hits.load(Ordering::Relaxed);
        let api_saved = self.prompt_cache_tokens_saved.load(Ordering::Relaxed);
        format!(
            "{} | Prompt Cache: {} hits, {} tokens saved",
            report.format_summary(), api_hits, api_saved
        )
    }
}

impl Clone for GlobalCache {
    fn clone(&self) -> Self {
        Self {
            l1: Arc::clone(&self.l1),
            l2: Arc::clone(&self.l2),
            l1_hits: Arc::clone(&self.l1_hits),
            l1_misses: Arc::clone(&self.l1_misses),
            l2_hits: Arc::clone(&self.l2_hits),
            l2_misses: Arc::clone(&self.l2_misses),
            path_tracker: Arc::clone(&self.path_tracker),
            prompt_cache_hits: Arc::clone(&self.prompt_cache_hits),
            prompt_cache_tokens_saved: Arc::clone(&self.prompt_cache_tokens_saved),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheReport {
    pub l1_hits: u64,
    pub l1_misses: u64,
    pub l2_hits: u64,
    pub l2_misses: u64,
    pub l1_entries: u64,
    pub l2_entries: u64,
    pub tracked_paths: usize,
    pub prompt_cache_hits: u64,
    pub prompt_cache_tokens_saved: u64,
}

impl CacheReport {
    pub fn l1_hit_rate(&self) -> f64 {
        let total = self.l1_hits + self.l1_misses;
        if total == 0 { 0.0 } else { self.l1_hits as f64 / total as f64 }
    }
    pub fn prompt_cache_tokens_saved(&self) -> u64 {
        self.prompt_cache_tokens_saved
    }
    pub fn format_summary(&self) -> String {
        let l2_total = self.l2_hits + self.l2_misses;
        let l2_rate = if l2_total == 0 { 0.0 } else { self.l2_hits as f64 / l2_total as f64 * 100.0 };
        format!(
            "Cache: L1 {}/{} ({:.1}%), L2 {}/{} ({:.1}%), Paths: {}",
            self.l1_hits, self.l1_hits + self.l1_misses, self.l1_hit_rate() * 100.0,
            self.l2_hits, l2_total, l2_rate, self.tracked_paths,
        )
    }
}

fn extract_path_from_input(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    match tool_name {
        "read" | "write" => input.get("path").and_then(|v| v.as_str()).map(|s| s.to_string()),
        _ => None,
    }
}

pub fn compute_input_hash(tool_name: &str, input: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tool_name.hash(&mut hasher);
    // 使用规范化 JSON 字符串 (sort keys) 确保相同语义的 JSON 产生相同 hash
    normalize_json(input).hash(&mut hasher);
    hasher.finish()
}

/// 规范化 JSON 值 (递归排序 object keys)
fn normalize_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| k.as_str());
            let parts: Vec<String> = sorted.iter()
                .map(|(k, v)| format!("{}:{}", k, normalize_json(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(|v| normalize_json(v)).collect();
            format!("[{}]", parts.join(","))
        }
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

// ============================================================
//  FileCache — 文件级缓存 (mtime 感知, 全局单例)
// ============================================================

/// 文件缓存条目
#[derive(Debug, Clone)]
pub struct FileCacheEntry {
    pub content: String,
    pub mtime: std::time::SystemTime,
}

/// 全局文件缓存 — key = 绝对路径, 按 mtime 判断是否失效
/// 使用 moka::sync::Cache (同步, 不需要 await)
pub static FILE_CACHE: std::sync::LazyLock<moka::sync::Cache<String, FileCacheEntry>> =
    std::sync::LazyLock::new(|| {
        moka::sync::Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(3600))
            .build()
    });

/// 获取缓存文件内容 (mtime 匹配才返回)
pub fn file_cache_get(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let current_mtime = meta.modified().ok()?;
    let entry = FILE_CACHE.get(path)?;
    if entry.mtime == current_mtime {
        Some(entry.content)
    } else {
        None
    }
}

/// 写入文件缓存
pub fn file_cache_set(path: &str, content: String) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mtime) = meta.modified() {
            FILE_CACHE.insert(path.to_string(), FileCacheEntry { content, mtime });
        }
    }
}

/// 失效文件缓存 (write 工具调用)
pub fn file_cache_invalidate(path: &str) {
    FILE_CACHE.invalidate(path);
}
