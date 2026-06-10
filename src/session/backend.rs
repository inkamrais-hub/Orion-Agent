//! SessionBackend trait — 存储抽象层
//!
//! 将 UnifiedStore 的具体实现隐藏在 trait 后面,
//! 使应用层不依赖特定存储后端。
//! 未来可替换为 PostgreSQL、远程 API 等。

use crate::session::unified::{
    AgentConfigModel, RollbackAction, SessionMeta, SessionSnapshot, SessionStatus, TranscriptEntry,
};

/// 存储后端 trait — 所有方法都是 async
///
/// 将 `UnifiedStore` 的公共 API 抽象为 trait，便于:
/// - 单元测试中使用 mock 实现
/// - 未来迁移到 PostgreSQL / 远程存储
/// - 应用层代码不绑定具体存储引擎
#[async_trait::async_trait]
pub trait SessionBackend: Send + Sync {
    // ── Agent Config ──

    /// 列出全部 Agent 配置
    async fn list_agents(&self) -> crate::Result<Vec<AgentConfigModel>>;

    /// 按 ID 获取单个 Agent 配置
    async fn get_agent(&self, id: &str) -> crate::Result<Option<AgentConfigModel>>;

    /// 新增 Agent 配置
    async fn create_agent(&self, config: &AgentConfigModel) -> crate::Result<()>;

    /// 更新 Agent 配置，返回是否命中记录
    async fn update_agent(&self, id: &str, config: &AgentConfigModel) -> crate::Result<bool>;

    /// 删除 Agent 配置，返回是否命中记录
    async fn delete_agent(&self, id: &str) -> crate::Result<bool>;

    // ── Sessions ──

    /// 创建新会话
    async fn create_session(&self, meta: &SessionMeta) -> crate::Result<()>;

    /// 按 ID 获取单个会话
    async fn get_session(&self, session_id: &str) -> crate::Result<Option<SessionMeta>>;

    /// 列出最近的会话 (按更新时间倒序)
    async fn list_sessions(&self, limit: u32) -> crate::Result<Vec<SessionMeta>>;

    /// 更新会话状态
    async fn update_session_status(
        &self,
        session_id: &str,
        status: SessionStatus,
    ) -> crate::Result<()>;

    /// 更新会话统计数据
    async fn update_session_stats(
        &self,
        session_id: &str,
        turn_count: u64,
        tool_call_count: u64,
        total_tokens: u64,
    ) -> crate::Result<()>;

    /// 删除会话及关联数据
    async fn delete_session(&self, session_id: &str) -> crate::Result<()>;

    // ── Transcripts ──

    /// 追加一条对话记录
    async fn append_transcript(&self, entry: &TranscriptEntry) -> crate::Result<()>;

    /// 获取会话的全部对话记录 (按时间排序)
    async fn get_transcripts(&self, session_id: &str) -> crate::Result<Vec<TranscriptEntry>>;

    // ── Snapshots ──

    /// 保存文件快照 (工具执行前的备份)
    async fn save_snapshot(&self, snapshot: &SessionSnapshot) -> crate::Result<()>;

    /// 获取会话的全部快照
    async fn get_snapshots(&self, session_id: &str) -> crate::Result<Vec<SessionSnapshot>>;

    /// 回滚到指定轮次，返回需要执行的文件恢复动作
    async fn rollback_to_turn(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>>;
}

// ============================================================
//  UnifiedStore 实现
// ============================================================

#[async_trait::async_trait]
impl SessionBackend for crate::session::UnifiedStore {
    // ── Agent Config ──

    async fn list_agents(&self) -> crate::Result<Vec<AgentConfigModel>> {
        self.list_agents().await
    }

    async fn get_agent(&self, id: &str) -> crate::Result<Option<AgentConfigModel>> {
        self.get_agent(id).await
    }

    async fn create_agent(&self, config: &AgentConfigModel) -> crate::Result<()> {
        self.create_agent(config).await
    }

    async fn update_agent(&self, id: &str, config: &AgentConfigModel) -> crate::Result<bool> {
        self.update_agent(id, config).await
    }

    async fn delete_agent(&self, id: &str) -> crate::Result<bool> {
        self.delete_agent(id).await
    }

    // ── Sessions ──

    async fn create_session(&self, meta: &SessionMeta) -> crate::Result<()> {
        self.create_session(meta).await
    }

    async fn get_session(&self, session_id: &str) -> crate::Result<Option<SessionMeta>> {
        self.get_session(session_id).await
    }

    async fn list_sessions(&self, limit: u32) -> crate::Result<Vec<SessionMeta>> {
        self.list_sessions(limit).await
    }

    async fn update_session_status(
        &self,
        session_id: &str,
        status: SessionStatus,
    ) -> crate::Result<()> {
        self.update_session_status(session_id, status).await
    }

    async fn update_session_stats(
        &self,
        session_id: &str,
        turn_count: u64,
        tool_call_count: u64,
        total_tokens: u64,
    ) -> crate::Result<()> {
        self.update_session_stats(session_id, turn_count, tool_call_count, total_tokens)
            .await
    }

    async fn delete_session(&self, session_id: &str) -> crate::Result<()> {
        self.delete_session(session_id).await
    }

    // ── Transcripts ──

    async fn append_transcript(&self, entry: &TranscriptEntry) -> crate::Result<()> {
        self.append_transcript(entry).await
    }

    async fn get_transcripts(&self, session_id: &str) -> crate::Result<Vec<TranscriptEntry>> {
        self.get_transcripts(session_id).await
    }

    // ── Snapshots ──

    async fn save_snapshot(&self, snapshot: &SessionSnapshot) -> crate::Result<()> {
        self.save_snapshot(snapshot).await
    }

    async fn get_snapshots(&self, session_id: &str) -> crate::Result<Vec<SessionSnapshot>> {
        self.get_snapshots(session_id).await
    }

    async fn rollback_to_turn(
        &self,
        session_id: &str,
        turn_index: u32,
    ) -> crate::Result<Vec<RollbackAction>> {
        self.rollback_to_turn(session_id, turn_index).await
    }
}

// ============================================================
//  Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::unified::{SessionMeta, SessionStatus, TranscriptEntry};
    use std::path::Path;

    /// 创建内存数据库用于测试
    async fn test_store() -> crate::session::UnifiedStore {
        crate::session::UnifiedStore::open_path(Path::new(":memory:"))
            .await
            .expect("open in-memory store")
    }

    fn make_agent(id: &str) -> AgentConfigModel {
        AgentConfigModel {
            id: id.to_string(),
            name: "Test Agent".to_string(),
            model: "deepseek-chat".to_string(),
            system_prompt: "You are a test agent.".to_string(),
            tools_json: r#"["read","write"]"#.to_string(),
            mcp_servers_json: "[]".to_string(),
            max_turns: 20,
            max_tool_calls: 30,
            token_budget: 128000,
            thinking: false,
            reasoning_effort: "medium".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_session(id: &str) -> SessionMeta {
        SessionMeta {
            session_id: id.to_string(),
            agent_name: "test-agent".to_string(),
            model: "deepseek-chat".to_string(),
            working_dir: "/tmp/test".to_string(),
            status: SessionStatus::Active,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            turn_count: 0,
            tool_call_count: 0,
            total_tokens: 0,
        }
    }

    /// 通过 trait object 调用，验证 trait 方法委托正确
    #[tokio::test]
    async fn test_trait_agent_crud() {
        let store = test_store().await;
        let backend: &dyn SessionBackend = &store;

        let agent = make_agent("agent-1");
        backend.create_agent(&agent).await.unwrap();

        let fetched = backend.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(fetched.name, "Test Agent");

        let list = backend.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);

        let mut updated = agent.clone();
        updated.name = "Updated".to_string();
        assert!(backend.update_agent("agent-1", &updated).await.unwrap());

        let fetched2 = backend.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(fetched2.name, "Updated");

        assert!(backend.delete_agent("agent-1").await.unwrap());
        assert!(backend.get_agent("agent-1").await.unwrap().is_none());
        assert!(!backend.delete_agent("agent-1").await.unwrap());
    }

    #[tokio::test]
    async fn test_trait_session_crud() {
        let store = test_store().await;
        let backend: &dyn SessionBackend = &store;

        let session = make_session("sess-1");
        backend.create_session(&session).await.unwrap();

        let fetched = backend.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched.status, SessionStatus::Active);

        let list = backend.list_sessions(10).await.unwrap();
        assert_eq!(list.len(), 1);

        backend
            .update_session_status("sess-1", SessionStatus::Completed)
            .await
            .unwrap();
        let fetched2 = backend.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched2.status, SessionStatus::Completed);

        backend
            .update_session_stats("sess-1", 5, 10, 2000)
            .await
            .unwrap();
        let fetched3 = backend.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched3.turn_count, 5);
        assert_eq!(fetched3.total_tokens, 2000);

        backend.delete_session("sess-1").await.unwrap();
        assert!(backend.get_session("sess-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_trait_transcripts() {
        let store = test_store().await;
        let backend: &dyn SessionBackend = &store;

        let session = make_session("sess-t");
        backend.create_session(&session).await.unwrap();

        let entry = TranscriptEntry {
            id: "t-1".to_string(),
            session_id: "sess-t".to_string(),
            parent_id: None,
            role: "user".to_string(),
            content: "Hello!".to_string(),
            tool_calls: None,
            timestamp: "2025-01-01T00:00:01Z".to_string(),
        };
        backend.append_transcript(&entry).await.unwrap();

        let transcripts = backend.get_transcripts("sess-t").await.unwrap();
        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].content, "Hello!");
    }

    #[tokio::test]
    async fn test_trait_snapshots_and_rollback() {
        let store = test_store().await;
        let backend: &dyn SessionBackend = &store;

        let snap = SessionSnapshot {
            snapshot_id: "snap-1".to_string(),
            session_id: "sess-s".to_string(),
            agent_id: "agent-1".to_string(),
            turn_index: 1,
            tool_name: "edit".to_string(),
            target_path: "/tmp/file.txt".to_string(),
            content_before: Some("original".to_string()),
            created_at: "2025-01-01T00:00:01Z".to_string(),
        };
        backend.save_snapshot(&snap).await.unwrap();

        let snapshots = backend.get_snapshots("sess-s").await.unwrap();
        assert_eq!(snapshots.len(), 1);

        let actions = backend.rollback_to_turn("sess-s", 1).await.unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].target_path, "/tmp/file.txt");
        assert_eq!(
            actions[0].content_before,
            Some("original".to_string())
        );
    }

    /// 验证 trait object 可以作为 Arc<dyn SessionBackend> 使用
    #[tokio::test]
    async fn test_trait_object_in_arc() {
        use std::sync::Arc;

        let store = test_store().await;
        let backend: Arc<dyn SessionBackend> = Arc::new(store);

        let agent = make_agent("agent-arc");
        backend.create_agent(&agent).await.unwrap();

        let list = backend.list_agents().await.unwrap();
        assert_eq!(list.len(), 1);
    }
}
