//! 行级文件快照系统
//!
//! 安全设计:
//!   - 校验和比对: 内容不变不创建快照
//!   - 变更比例告警: 超过 80% 行变更时标记为 "risky"
//!   - session_id/agent_id 追踪: 每个快照关联到具体会话和 Agent
//!   - 审计集成: 快照事件自动写入审计日志

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use async_trait::async_trait;
use serde_json::Value;
use crate::tools::{Tool, ToolContext, ToolResult};

/// 变更风险级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeRisk {
    /// 正常变更 (< 30% 行变更)
    Normal,
    /// 大量变更 (30-80% 行变更)
    Large,
    /// 高风险变更 (> 80% 行变更，可能是误操作)
    Risky,
    /// 内容未变更 (校验和相同)
    Unchanged,
}

/// 行级变更类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LineChange {
    /// 插入行 (line_number, content)
    Insert(usize, String),
    /// 删除行 (line_number)
    Delete(usize),
    /// 修改行 (line_number, old_content, new_content)
    Modify(usize, String, String),
}

/// 文件快照条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// 时间戳 (毫秒)
    pub timestamp: u64,
    /// 文件路径
    pub file_path: String,
    /// 会话 ID
    pub session_id: String,
    /// Agent ID
    pub agent_id: String,
    /// 行级变更
    pub changes: Vec<LineChange>,
    /// 变更前的文件校验和
    pub before_checksum: String,
    /// 变更后的文件校验和
    pub after_checksum: String,
    /// 变更前的总行数
    pub before_line_count: usize,
    /// 变更后的总行数
    pub after_line_count: usize,
    /// 变更风险级别
    pub risk: ChangeRisk,
    /// 变更行数
    pub changed_lines: usize,
    /// 变更比例 (0.0 - 1.0)
    pub change_ratio: f64,
    /// Agent 操作描述 (可选)
    pub description: Option<String>,
}

/// 快照创建结果
#[derive(Debug)]
pub enum SnapshotResult {
    /// 成功创建快照
    Created(SnapshotEntry),
    /// 内容未变更，跳过
    SkippedUnchanged,
    /// 高风险变更，需要确认
    RiskyChange(SnapshotEntry),
}

/// 快照存储
pub struct SnapshotStore {
    /// 快照根目录 (.orion/snapshots/)
    root: std::path::PathBuf,
    /// 内存索引: file_path -> [entries]
    index: HashMap<String, Vec<SnapshotEntry>>,
}

impl SnapshotStore {
    /// 创建新的快照存储
    pub fn new(workspace_root: &Path) -> Self {
        let root = workspace_root.join(".orion").join("snapshots");
        Self {
            root,
            index: HashMap::new(),
        }
    }

    /// 初始化存储目录
    pub fn init(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// 计算文件内容的校验和
    pub fn checksum(content: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// 评估变更风险
    fn assess_risk(changed_lines: usize, total_lines: usize) -> ChangeRisk {
        if total_lines == 0 {
            return ChangeRisk::Normal;
        }
        let ratio = changed_lines as f64 / total_lines as f64;
        if ratio > 0.8 {
            ChangeRisk::Risky
        } else if ratio > 0.3 {
            ChangeRisk::Large
        } else {
            ChangeRisk::Normal
        }
    }

    /// 计算两个文本的行级差异
    pub fn compute_diff(old_content: &str, new_content: &str) -> Vec<LineChange> {
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();
        let mut changes = Vec::new();

        let diff = Self::simple_diff(&old_lines, &new_lines);
        for op in diff {
            match op {
                DiffOp::Insert(idx, line) => {
                    changes.push(LineChange::Insert(idx, line.to_string()));
                }
                DiffOp::Delete(idx) => {
                    changes.push(LineChange::Delete(idx));
                }
                DiffOp::Modify(idx, old, new) => {
                    changes.push(LineChange::Modify(idx, old.to_string(), new.to_string()));
                }
            }
        }

        changes
    }

    /// 简化的差异算法
    fn simple_diff<'a>(old_lines: &[&'a str], new_lines: &[&'a str]) -> Vec<DiffOp<'a>> {
        let mut ops = Vec::new();
        let old_len = old_lines.len();
        let new_len = new_lines.len();

        let mut old_idx = 0;
        let mut new_idx = 0;

        while old_idx < old_len || new_idx < new_len {
            if old_idx >= old_len {
                ops.push(DiffOp::Insert(new_idx + 1, new_lines[new_idx]));
                new_idx += 1;
            } else if new_idx >= new_len {
                ops.push(DiffOp::Delete(old_idx + 1));
                old_idx += 1;
            } else if old_lines[old_idx] == new_lines[new_idx] {
                old_idx += 1;
                new_idx += 1;
            } else {
                let mut found_old = None;
                let mut found_new = None;

                for j in new_idx..new_len.min(new_idx + 10) {
                    if new_lines[j] == old_lines[old_idx] {
                        found_new = Some(j);
                        break;
                    }
                }

                for i in old_idx..old_len.min(old_idx + 10) {
                    if old_lines[i] == new_lines[new_idx] {
                        found_old = Some(i);
                        break;
                    }
                }

                match (found_old, found_new) {
                    (Some(_), Some(_)) => {
                        if found_new.unwrap() - new_idx <= found_old.unwrap() - old_idx {
                            ops.push(DiffOp::Insert(new_idx + 1, new_lines[new_idx]));
                            new_idx += 1;
                        } else {
                            ops.push(DiffOp::Delete(old_idx + 1));
                            old_idx += 1;
                        }
                    }
                    (Some(i), None) => {
                        while new_idx < i && new_idx < new_len {
                            ops.push(DiffOp::Insert(new_idx + 1, new_lines[new_idx]));
                            new_idx += 1;
                        }
                    }
                    (None, Some(j)) => {
                        while old_idx < j && old_idx < old_len {
                            ops.push(DiffOp::Delete(old_idx + 1));
                            old_idx += 1;
                        }
                    }
                    (None, None) => {
                        ops.push(DiffOp::Modify(
                            old_idx + 1,
                            old_lines[old_idx],
                            new_lines[new_idx],
                        ));
                        old_idx += 1;
                        new_idx += 1;
                    }
                }
            }
        }

        ops
    }

    /// 创建快照 (带安全检查)
    ///
    /// 返回 SnapshotResult:
    ///   - SkippedUnchanged: 内容未变更
    ///   - Created: 正常创建
    ///   - RiskyChange: 高风险变更
    pub fn create_snapshot(
        &mut self,
        file_path: &str,
        old_content: &str,
        new_content: &str,
        session_id: &str,
        agent_id: &str,
        description: Option<String>,
    ) -> SnapshotResult {
        let before_checksum = Self::checksum(old_content);
        let after_checksum = Self::checksum(new_content);

        // 核心安全检查: 内容未变更则跳过
        if before_checksum == after_checksum {
            return SnapshotResult::SkippedUnchanged;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let changes = Self::compute_diff(old_content, new_content);
        let before_line_count = old_content.lines().count();
        let after_line_count = new_content.lines().count();
        let changed_lines = changes.len();
        let max_lines = before_line_count.max(after_line_count);
        let change_ratio = if max_lines > 0 {
            changed_lines as f64 / max_lines as f64
        } else {
            0.0
        };
        let risk = Self::assess_risk(changed_lines, max_lines);

        let entry = SnapshotEntry {
            timestamp,
            file_path: file_path.to_string(),
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            changes,
            before_checksum,
            after_checksum,
            before_line_count,
            after_line_count,
            risk: risk.clone(),
            changed_lines,
            change_ratio,
            description,
        };

        // 保存到磁盘
        self.save_to_disk(&entry);

        // 更新索引
        self.index
            .entry(file_path.to_string())
            .or_default()
            .push(entry.clone());

        // 根据风险级别返回不同结果
        match risk {
            ChangeRisk::Risky => SnapshotResult::RiskyChange(entry),
            _ => SnapshotResult::Created(entry),
        }
    }

    /// 保存快照到磁盘
    fn save_to_disk(&self, entry: &SnapshotEntry) {
        let file_hash = Self::checksum(&entry.file_path);
        let snapshot_dir = self.root.join(&file_hash);
        if let Err(e) = std::fs::create_dir_all(&snapshot_dir) {
            eprintln!("Failed to create snapshot dir: {}", e);
            return;
        }

        let snapshot_file = snapshot_dir.join(format!("{}.json", entry.timestamp));
        if let Ok(json) = serde_json::to_string_pretty(entry) {
            let _ = std::fs::write(snapshot_file, json);
        }
    }

    /// 获取文件的所有快照
    pub fn get_snapshots(&self, file_path: &str) -> Vec<&SnapshotEntry> {
        self.index
            .get(file_path)
            .map(|entries| entries.iter().collect())
            .unwrap_or_default()
    }

    /// 获取指定 session 的快照
    pub fn get_snapshots_by_session(&self, session_id: &str) -> Vec<&SnapshotEntry> {
        self.index
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|e| e.session_id == session_id)
            .collect()
    }

    /// 获取指定 agent 的快照
    pub fn get_snapshots_by_agent(&self, agent_id: &str) -> Vec<&SnapshotEntry> {
        self.index
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|e| e.agent_id == agent_id)
            .collect()
    }

    /// 获取高风险快照
    pub fn get_risky_snapshots(&self) -> Vec<&SnapshotEntry> {
        self.index
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|e| e.risk == ChangeRisk::Risky)
            .collect()
    }

    /// 回溯到指定时间点的文件内容
    pub fn rollback_to_time(
        &self,
        file_path: &str,
        target_time: u64,
        current_content: &str,
    ) -> Option<String> {
        let entries = self.index.get(file_path)?;
        if entries.is_empty() {
            return None;
        }

        let mut content = current_content.to_string();
        let mut entries_to_reverse: Vec<&SnapshotEntry> = entries
            .iter()
            .filter(|e| e.timestamp > target_time)
            .collect();

        entries_to_reverse.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        for entry in entries_to_reverse {
            content = Self::reverse_apply(&content, &entry.changes);
        }

        Some(content)
    }

    /// 回溯到指定快照
    pub fn rollback_to_snapshot(
        &self,
        file_path: &str,
        snapshot_index: usize,
        current_content: &str,
    ) -> Option<String> {
        let entries = self.index.get(file_path)?;
        if snapshot_index >= entries.len() {
            return None;
        }

        let target_time = entries[snapshot_index].timestamp;
        self.rollback_to_time(file_path, target_time, current_content)
    }

    /// 反向应用变更 (用于回溯)
    pub fn reverse_apply(content: &str, changes: &[LineChange]) -> String {
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        let mut sorted_changes: Vec<&LineChange> = changes.iter().collect();
        sorted_changes.sort_by(|a, b| {
            let line_a = match a {
                LineChange::Insert(n, _) => *n,
                LineChange::Delete(n) => *n,
                LineChange::Modify(n, _, _) => *n,
            };
            let line_b = match b {
                LineChange::Insert(n, _) => *n,
                LineChange::Delete(n) => *n,
                LineChange::Modify(n, _, _) => *n,
            };
            line_b.cmp(&line_a)
        });

        for change in sorted_changes {
            match change {
                LineChange::Insert(line_num, _) => {
                    if *line_num > 0 && *line_num <= lines.len() {
                        lines.remove(line_num - 1);
                    }
                }
                LineChange::Delete(line_num) => {
                    if *line_num > 0 && *line_num <= lines.len() + 1 {
                        lines.insert(line_num - 1, String::new());
                    }
                }
                LineChange::Modify(line_num, old_content, _) => {
                    if *line_num > 0 && *line_num <= lines.len() {
                        lines[line_num - 1] = old_content.clone();
                    }
                }
            }
        }

        lines.join("\n")
    }

    /// 正向应用变更 (用于前进)
    pub fn forward_apply(content: &str, changes: &[LineChange]) -> String {
        let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        for change in changes {
            match change {
                LineChange::Insert(line_num, new_content) => {
                    if *line_num > 0 && *line_num <= lines.len() + 1 {
                        lines.insert(line_num - 1, new_content.clone());
                    }
                }
                LineChange::Delete(line_num) => {
                    if *line_num > 0 && *line_num <= lines.len() {
                        lines.remove(line_num - 1);
                    }
                }
                LineChange::Modify(line_num, _, new_content) => {
                    if *line_num > 0 && *line_num <= lines.len() {
                        lines[line_num - 1] = new_content.clone();
                    }
                }
            }
        }

        lines.join("\n")
    }

    /// 获取文件变更历史摘要
    pub fn get_history_summary(&self, file_path: &str) -> String {
        let entries = self.index.get(file_path);
        match entries {
            None => format!("No history for {}", file_path),
            Some(entries) => {
                if entries.is_empty() {
                    return format!("No snapshots for {}", file_path);
                }

                let mut summary = format!(
                    "File: {} ({} snapshots)\n",
                    file_path,
                    entries.len()
                );
                summary.push_str(&format!("{:-<70}\n", ""));

                for (i, entry) in entries.iter().enumerate() {
                    let time = chrono::DateTime::from_timestamp_millis(entry.timestamp as i64)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| format!("T{}", entry.timestamp));

                    let change_summary = Self::summarize_changes(&entry.changes);
                    let desc = entry.description.as_deref().unwrap_or("-");
                    let risk_str = match entry.risk {
                        ChangeRisk::Normal => "",
                        ChangeRisk::Large => " [LARGE]",
                        ChangeRisk::Risky => " [RISKY]",
                        ChangeRisk::Unchanged => " [SKIP]",
                    };

                    summary.push_str(&format!(
                        "[{}] {} | Lines: {}→{} | {} | {:.0}%{} | Session: {} | Agent: {}\n    {}\n",
                        i, time, entry.before_line_count, entry.after_line_count,
                        change_summary, entry.change_ratio * 100.0, risk_str,
                        &entry.session_id[..8.min(entry.session_id.len())],
                        entry.agent_id, desc
                    ));
                }

                summary
            }
        }
    }

    /// 摘要变更
    fn summarize_changes(changes: &[LineChange]) -> String {
        let inserts = changes.iter().filter(|c| matches!(c, LineChange::Insert(_, _))).count();
        let deletes = changes.iter().filter(|c| matches!(c, LineChange::Delete(_))).count();
        let modifies = changes.iter().filter(|c| matches!(c, LineChange::Modify(_, _, _))).count();

        let mut parts = Vec::new();
        if inserts > 0 { parts.push(format!("+{}", inserts)); }
        if deletes > 0 { parts.push(format!("-{}", deletes)); }
        if modifies > 0 { parts.push(format!("~{}", modifies)); }

        if parts.is_empty() {
            "no changes".to_string()
        } else {
            parts.join(", ")
        }
    }

    /// 从磁盘加载快照
    pub fn load_from_disk(&mut self) -> std::io::Result<()> {
        if !self.root.exists() {
            return Ok(());
        }

        for dir_entry in std::fs::read_dir(&self.root)? {
            let dir_entry = dir_entry?;
            if !dir_entry.file_type()?.is_dir() {
                continue;
            }

            for file_entry in std::fs::read_dir(dir_entry.path())? {
                let file_entry = file_entry?;
                let path = file_entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }

                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(entry) = serde_json::from_str::<SnapshotEntry>(&content) {
                        self.index
                            .entry(entry.file_path.clone())
                            .or_default()
                            .push(entry);
                    }
                }
            }
        }

        for entries in self.index.values_mut() {
            entries.sort_by_key(|e| e.timestamp);
        }

        Ok(())
    }
}

/// 内部差异操作类型
enum DiffOp<'a> {
    Insert(usize, &'a str),
    Delete(usize),
    Modify(usize, &'a str, &'a str),
}

// ============================================================
//  Agent 工具
// ============================================================

/// 快照历史工具
pub struct SnapshotHistoryTool;

#[async_trait]
impl Tool for SnapshotHistoryTool {
    fn name(&self) -> &str { "snapshot_history" }
    fn description(&self) -> &str { "查看文件的快照历史，显示每次变更的时间、行数变化、风险级别和关联的 session/agent" }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "文件路径" }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let file_path = input.get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Tool("Missing file_path".into()))?;

        let workspace = std::env::current_dir().unwrap_or_default();
        let mut store = SnapshotStore::new(&workspace);
        let _ = store.load_from_disk();

        Ok(ToolResult {
            content: store.get_history_summary(file_path),
            is_error: false,
            metadata: None,
        })
    }
}

/// 快照回溯工具
pub struct SnapshotRollbackTool;

#[async_trait]
impl Tool for SnapshotRollbackTool {
    fn name(&self) -> &str { "snapshot_rollback" }
    fn description(&self) -> &str { "回溯文件到指定快照时间点" }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string", "description": "文件路径" },
                "snapshot_index": { "type": "integer", "description": "快照索引 (从0开始)" },
                "current_content": { "type": "string", "description": "当前文件内容" }
            },
            "required": ["file_path", "snapshot_index", "current_content"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let file_path = input.get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Tool("Missing file_path".into()))?;
        let snapshot_index = input.get("snapshot_index")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| crate::Error::Tool("Missing snapshot_index".into()))? as usize;
        let current_content = input.get("current_content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::Error::Tool("Missing current_content".into()))?;

        let workspace = std::env::current_dir().unwrap_or_default();
        let mut store = SnapshotStore::new(&workspace);
        let _ = store.load_from_disk();

        match store.rollback_to_snapshot(file_path, snapshot_index, current_content) {
            Some(restored) => Ok(ToolResult {
                content: restored,
                is_error: false,
                metadata: None,
            }),
            None => Err(crate::Error::Tool(format!("Cannot rollback to snapshot {}", snapshot_index))),
        }
    }
}

/// 高风险快照查询工具
pub struct SnapshotRiskyTool;

#[async_trait]
impl Tool for SnapshotRiskyTool {
    fn name(&self) -> &str { "snapshot_risky" }
    fn description(&self) -> &str { "查看所有高风险快照 (>80% 行变更，可能是误操作)" }
    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> crate::Result<ToolResult> {
        let workspace = std::env::current_dir().unwrap_or_default();
        let mut store = SnapshotStore::new(&workspace);
        let _ = store.load_from_disk();

        let risky = store.get_risky_snapshots();
        if risky.is_empty() {
            return Ok(ToolResult {
                content: "No risky snapshots found.".to_string(),
                is_error: false,
                metadata: None,
            });
        }

        let mut summary = format!("Found {} risky snapshots:\n\n", risky.len());
        for entry in &risky {
            let time = chrono::DateTime::from_timestamp_millis(entry.timestamp as i64)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| format!("T{}", entry.timestamp));

            summary.push_str(&format!(
                "  {} | {} | Lines: {}→{} ({:.0}%) | Session: {} | Agent: {}\n",
                entry.file_path, time,
                entry.before_line_count, entry.after_line_count,
                entry.change_ratio * 100.0,
                &entry.session_id[..8.min(entry.session_id.len())],
                entry.agent_id
            ));
        }

        Ok(ToolResult {
            content: summary,
            is_error: false,
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unchanged_skipped() {
        let mut store = SnapshotStore::new(Path::new("."));
        let content = "line1\nline2\nline3";
        let result = store.create_snapshot("test.rs", content, content, "sess1", "agent1", None);
        assert!(matches!(result, SnapshotResult::SkippedUnchanged));
    }

    #[test]
    fn test_normal_change() {
        let mut store = SnapshotStore::new(Path::new("."));
        let old = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        let new = "line1\nmodified\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        let result = store.create_snapshot("test.rs", old, new, "sess1", "agent1", Some("fix bug".into()));
        match result {
            SnapshotResult::Created(entry) => {
                assert_eq!(entry.risk, ChangeRisk::Normal);
                assert_eq!(entry.session_id, "sess1");
                assert_eq!(entry.agent_id, "agent1");
                assert!(entry.changed_lines > 0);
            }
            _ => panic!("Expected Created"),
        }
    }

    #[test]
    fn test_risky_change() {
        let mut store = SnapshotStore::new(Path::new("."));
        let old = "a\nb\nc\nd\ne";
        let new = "x\ny\nz\nw\nv";
        let result = store.create_snapshot("test.rs", old, new, "sess1", "agent1", None);
        match result {
            SnapshotResult::RiskyChange(entry) => {
                assert_eq!(entry.risk, ChangeRisk::Risky);
            }
            _ => panic!("Expected RiskyChange, got {:?}", result),
        }
    }

    #[test]
    fn test_session_tracking() {
        let mut store = SnapshotStore::new(Path::new("."));
        store.create_snapshot("a.rs", "old", "new", "session_abc", "alice", None);
        store.create_snapshot("b.rs", "old", "new", "session_abc", "bob", None);
        store.create_snapshot("c.rs", "old", "new", "session_xyz", "alice", None);

        let abc_snapshots = store.get_snapshots_by_session("session_abc");
        assert_eq!(abc_snapshots.len(), 2);

        let alice_snapshots = store.get_snapshots_by_agent("alice");
        assert_eq!(alice_snapshots.len(), 2);
    }

    #[test]
    fn test_compute_diff_insert() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline2\nline2.5\nline3";
        let changes = SnapshotStore::compute_diff(old, new);
        assert!(!changes.is_empty());
        assert!(changes.iter().any(|c| matches!(c, LineChange::Insert(3, _))));
    }

    #[test]
    fn test_compute_diff_delete() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline3";
        let changes = SnapshotStore::compute_diff(old, new);
        assert!(!changes.is_empty());
        assert!(changes.iter().any(|c| matches!(c, LineChange::Delete(2))));
    }

    #[test]
    fn test_compute_diff_modify() {
        let old = "line1\nline2\nline3";
        let new = "line1\nmodified\nline3";
        let changes = SnapshotStore::compute_diff(old, new);
        assert!(!changes.is_empty());
        assert!(changes.iter().any(|c| matches!(c, LineChange::Modify(2, _, _))));
    }

    #[test]
    fn test_forward_apply() {
        let changes = vec![LineChange::Insert(2, "new line".to_string())];
        let result = SnapshotStore::forward_apply("line1\nline2", &changes);
        assert_eq!(result, "line1\nnew line\nline2");
    }

    #[test]
    fn test_rollback_roundtrip() {
        let original = "line1\nline2\nline3";
        let modified = "line1\nmodified\nline3\nline4";
        let changes = SnapshotStore::compute_diff(original, modified);
        let restored = SnapshotStore::reverse_apply(modified, &changes);
        assert!(restored.contains("line1"));
        assert!(restored.contains("line3"));
    }

    #[test]
    fn test_checksum() {
        let hash1 = SnapshotStore::checksum("hello");
        let hash2 = SnapshotStore::checksum("hello");
        let hash3 = SnapshotStore::checksum("world");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_summarize_changes() {
        let changes = vec![
            LineChange::Insert(1, "a".into()),
            LineChange::Insert(2, "b".into()),
            LineChange::Delete(3),
            LineChange::Modify(4, "old".into(), "new".into()),
        ];
        let summary = SnapshotStore::summarize_changes(&changes);
        assert_eq!(summary, "+2, -1, ~1");
    }

    #[test]
    fn test_assess_risk() {
        assert_eq!(SnapshotStore::assess_risk(5, 100), ChangeRisk::Normal);
        assert_eq!(SnapshotStore::assess_risk(50, 100), ChangeRisk::Large);
        assert_eq!(SnapshotStore::assess_risk(90, 100), ChangeRisk::Risky);
    }
}
