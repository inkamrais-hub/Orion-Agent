//! Project-scoped Session Memory — extract key knowledge from conversations,
//! persist across sessions, scoped to specific projects/workspaces.
//!
//! Storage locations:
//!   - Global:  ~/.orion/memories.json  (or ~/.config/orion/memories.json)
//!   - Project: <data_dir>/projects/{hash_of_project_path}/memories.json

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Memory category
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MemoryCategory {
    UserPreference,  // User habits, tool preferences
    ProjectFact,     // Code architecture, language, framework
    CodePattern,     // Coding style, conventions
    Decision,        // Important decisions
    Constraint,      // Discovered limitations
}

/// A single memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub category: MemoryCategory,
    pub content: String,
    pub confidence: f32,  // 0.0 - 1.0
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub session_id: String,
    /// The project/workspace this memory belongs to.
    /// `None` means the memory is global (not tied to any project).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    /// When this memory was last accessed (read or written).
    /// Used for memory decay / pruning.
    #[serde(default = "chrono::Utc::now")]
    pub last_accessed: chrono::DateTime<chrono::Utc>,
}

/// Session Memory manager
///
/// Supports both global memories (no project path) and project-scoped memories.
/// Project-scoped memories are stored under
/// `<data_dir>/projects/{hash_of_project_path}/memories.json`.
pub struct SessionMemory {
    memories: Vec<MemoryEntry>,
    file_path: PathBuf,
    /// Optional project path that scopes this memory instance.
    /// When set, `add()` tags new entries with this path and `as_context()`
    /// returns only memories matching this project (plus global memories).
    pub project_path: Option<String>,
}

/// Compute a stable hash for a project path, used as the subdirectory name
/// under `<data_dir>/projects/`.
fn hash_project_path(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Return the storage directory for a given project path.
/// - `None`           -> `<data_dir>/`            (global memories)
/// `Some(project)`    -> `<data_dir>/projects/{hash}/`
fn memory_storage_dir(project_path: Option<&str>) -> PathBuf {
    let base = crate::config::data_dir_path();
    match project_path {
        Some(p) => base.join("projects").join(hash_project_path(p)),
        None => base,
    }
}

impl SessionMemory {
    /// Load global memories from `<data_dir>/memories.json` (backward-compatible).
    pub fn load() -> Self {
        Self::load_for_project(None)
    }

    /// Load memories scoped to an optional project path.
    ///
    /// - `None`           -> loads from `<data_dir>/memories.json`
    /// - `Some(project)`  -> loads from `<data_dir>/projects/{hash}/memories.json`
    pub fn load_for_project(project_path: Option<&str>) -> Self {
        let dir = memory_storage_dir(project_path);
        let file_path = dir.join("memories.json");
        let memories = if file_path.exists() {
            let content = std::fs::read_to_string(&file_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            memories,
            file_path,
            project_path: project_path.map(|s| s.to_string()),
        }
    }

    /// Construct a `SessionMemory` scoped to a specific project/workspace.
    pub fn for_project(project_path: &str) -> Self {
        Self::load_for_project(Some(project_path))
    }

    /// Persist memories to disk.
    pub fn save(&self) -> crate::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.memories)?;
        std::fs::write(&self.file_path, json)?;
        Ok(())
    }

    /// Add a new memory (auto-deduplicates by content).
    ///
    /// When this `SessionMemory` has a `project_path`, new entries are
    /// automatically tagged with it.  `last_accessed` is set to now.
    pub fn add(&mut self, category: MemoryCategory, content: String, confidence: f32, session_id: &str) {
        // If an identical memory already exists, just bump its last_accessed timestamp.
        if let Some(existing) = self.memories.iter_mut().find(|m| m.content == content) {
            existing.last_accessed = chrono::Utc::now();
            return;
        }
        self.memories.push(MemoryEntry {
            category,
            content,
            confidence,
            timestamp: chrono::Utc::now(),
            session_id: session_id.to_string(),
            project_path: self.project_path.clone(),
            last_accessed: chrono::Utc::now(),
        });
    }

    /// Return high-confidence memories as context for the system prompt.
    ///
    /// For project-scoped instances this returns:
    ///   - memories matching the current project, **plus**
    ///   - global memories (those with no `project_path`).
    ///
    /// For global instances (no `project_path`) all memories are returned.
    pub fn as_context(&self) -> String {
        let filtered: Vec<&MemoryEntry> = self.memories.iter().filter(|m| {
            if self.project_path.is_none() {
                // Global instance: include everything.
                true
            } else {
                // Project-scoped: include matching-project memories + global memories.
                match (&m.project_path, &self.project_path) {
                    (None, _) => true, // global memories always visible
                    (Some(mp), Some(sp)) => mp == sp,
                    _ => false,
                }
            }
        }).collect();

        if filtered.is_empty() {
            return String::new();
        }

        let mut parts = vec!["[Learned from previous sessions]".to_string()];
        for m in &filtered {
            if m.confidence >= 0.7 {
                let cat = match m.category {
                    MemoryCategory::UserPreference => "Preference",
                    MemoryCategory::ProjectFact => "Project",
                    MemoryCategory::CodePattern => "Pattern",
                    MemoryCategory::Decision => "Decision",
                    MemoryCategory::Constraint => "Constraint",
                };
                parts.push(format!("- [{}] {}", cat, m.content));
            }
        }
        parts.join("\n")
    }

    /// Number of memory entries currently held.
    pub fn len(&self) -> usize {
        self.memories.len()
    }

    /// Whether the memory store is empty.
    pub fn is_empty(&self) -> bool {
        self.memories.is_empty()
    }

    /// Filter entries by category.
    pub fn by_category(&self, category: &MemoryCategory) -> Vec<&MemoryEntry> {
        self.memories.iter().filter(|m| &m.category == category).collect()
    }

    /// Remove memories whose `last_accessed` is older than `max_age_days` days.
    ///
    /// Default recommended value: 90 days.
    pub fn prune(&mut self, max_age_days: u64) {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
        self.memories.retain(|m| m.last_accessed >= cutoff);
    }
}

/// Rule-based memory extraction from a single conversation turn (no LLM call).
pub fn extract_memories(user_input: &str, response: &str, _session_id: &str) -> Vec<(MemoryCategory, String)> {
    let mut memories = Vec::new();
    let lower = user_input.to_lowercase();

    // Detect user preferences
    if lower.contains("edit") || lower.contains("use edit") {
        memories.push((MemoryCategory::UserPreference, "User prefers using edit tool for small changes".into()));
    }
    if lower.contains("don't") || lower.contains("do not") {
        memories.push((MemoryCategory::UserPreference, format!("User instruction: {}", user_input)));
    }

    // Detect project facts from tool results
    if response.contains("Cargo.toml") {
        memories.push((MemoryCategory::ProjectFact, "Project uses Rust/Cargo".into()));
    }
    if response.contains("package.json") {
        memories.push((MemoryCategory::ProjectFact, "Project uses Node.js/npm".into()));
    }

    memories
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_project_path_is_deterministic() {
        let h1 = hash_project_path("/home/user/myproject");
        let h2 = hash_project_path("/home/user/myproject");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_project_path_differs_for_different_paths() {
        let h1 = hash_project_path("/home/user/project_a");
        let h2 = hash_project_path("/home/user/project_b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_memory_storage_dir_global() {
        let dir = memory_storage_dir(None);
        // Should be the base data dir (no "projects" segment).
        assert!(!dir.to_string_lossy().contains("projects"));
    }

    #[test]
    fn test_memory_storage_dir_project() {
        let dir = memory_storage_dir(Some("/home/user/myproject"));
        let s = dir.to_string_lossy();
        assert!(s.contains("projects"));
    }

    #[test]
    fn test_add_sets_project_path() {
        let mut mem = SessionMemory {
            memories: Vec::new(),
            file_path: PathBuf::from("/tmp/test_memories.json"),
            project_path: Some("/home/user/proj".to_string()),
        };
        mem.add(MemoryCategory::ProjectFact, "Uses Rust".into(), 0.9, "sess1");
        assert_eq!(mem.len(), 1);
        assert_eq!(
            mem.memories[0].project_path.as_deref(),
            Some("/home/user/proj")
        );
    }

    #[test]
    fn test_add_dedup_bumps_last_accessed() {
        let mut mem = SessionMemory {
            memories: Vec::new(),
            file_path: PathBuf::from("/tmp/test_memories.json"),
            project_path: None,
        };
        mem.add(MemoryCategory::ProjectFact, "Uses Rust".into(), 0.9, "sess1");
        let first_accessed = mem.memories[0].last_accessed;
        // Small sleep to ensure timestamp differs.
        std::thread::sleep(std::time::Duration::from_millis(5));
        mem.add(MemoryCategory::ProjectFact, "Uses Rust".into(), 0.9, "sess2");
        assert_eq!(mem.len(), 1, "duplicate should not create a second entry");
        assert!(
            mem.memories[0].last_accessed >= first_accessed,
            "last_accessed should be bumped"
        );
    }

    #[test]
    fn test_as_context_filters_by_project() {
        let mut mem = SessionMemory {
            memories: Vec::new(),
            file_path: PathBuf::from("/tmp/test_memories.json"),
            project_path: Some("/proj_a".to_string()),
        };
        // Global memory
        mem.memories.push(MemoryEntry {
            category: MemoryCategory::UserPreference,
            content: "Likes dark mode".into(),
            confidence: 0.9,
            timestamp: chrono::Utc::now(),
            session_id: "s1".into(),
            project_path: None,
            last_accessed: chrono::Utc::now(),
        });
        // Memory for proj_a
        mem.memories.push(MemoryEntry {
            category: MemoryCategory::ProjectFact,
            content: "Proj A uses Rust".into(),
            confidence: 0.9,
            timestamp: chrono::Utc::now(),
            session_id: "s1".into(),
            project_path: Some("/proj_a".into()),
            last_accessed: chrono::Utc::now(),
        });
        // Memory for proj_b (should be excluded)
        mem.memories.push(MemoryEntry {
            category: MemoryCategory::ProjectFact,
            content: "Proj B uses Python".into(),
            confidence: 0.9,
            timestamp: chrono::Utc::now(),
            session_id: "s1".into(),
            project_path: Some("/proj_b".into()),
            last_accessed: chrono::Utc::now(),
        });

        let ctx = mem.as_context();
        assert!(ctx.contains("Likes dark mode"), "global memories should appear");
        assert!(ctx.contains("Proj A uses Rust"), "proj_a memories should appear");
        assert!(!ctx.contains("Proj B uses Python"), "proj_b memories should NOT appear");
    }

    #[test]
    fn test_prune_removes_old_memories() {
        let mut mem = SessionMemory {
            memories: Vec::new(),
            file_path: PathBuf::from("/tmp/test_memories.json"),
            project_path: None,
        };
        // Recent memory
        mem.memories.push(MemoryEntry {
            category: MemoryCategory::ProjectFact,
            content: "Recent".into(),
            confidence: 0.9,
            timestamp: chrono::Utc::now(),
            session_id: "s1".into(),
            project_path: None,
            last_accessed: chrono::Utc::now(),
        });
        // Stale memory (200 days ago)
        mem.memories.push(MemoryEntry {
            category: MemoryCategory::ProjectFact,
            content: "Stale".into(),
            confidence: 0.9,
            timestamp: chrono::Utc::now(),
            session_id: "s1".into(),
            project_path: None,
            last_accessed: chrono::Utc::now() - chrono::Duration::days(200),
        });
        assert_eq!(mem.len(), 2);
        mem.prune(90);
        assert_eq!(mem.len(), 1);
        assert_eq!(mem.memories[0].content, "Recent");
    }
}
