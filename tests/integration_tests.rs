//! Integration tests for Orion Agent Framework

use orion_agent::session::unified::UnifiedStore;

// ── UnifiedStore Integration Tests ──

#[tokio::test]
async fn test_unified_store_session_lifecycle() {
    let store = UnifiedStore::in_memory().unwrap();

    // Create a session
    let meta = orion_agent::session::unified::SessionMeta {
        session_id: "test-session-1".into(),
        agent_name: "test-agent".into(),
        model: "test-model".into(),
        working_dir: "/tmp/test".into(),
        status: orion_agent::session::unified::SessionStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    };

    store.create_session(&meta).await.unwrap();

    // Get session
    let loaded = store.get_session("test-session-1").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.agent_name, "test-agent");

    // Update status
    store
        .update_session_status(
            "test-session-1",
            orion_agent::session::unified::SessionStatus::Completed,
        )
        .await
        .unwrap();
    let updated = store
        .get_session("test-session-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        updated.status,
        orion_agent::session::unified::SessionStatus::Completed
    );

    // List sessions
    let sessions = store.list_sessions(10).await.unwrap();
    assert_eq!(sessions.len(), 1);

    // Delete session
    let deleted = store.delete_session("test-session-1").await.unwrap();
    assert!(deleted);
    let gone = store.get_session("test-session-1").await.unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn test_unified_store_agent_crud() {
    let store = UnifiedStore::in_memory().unwrap();

    let config = orion_agent::session::unified::AgentConfigModel {
        id: "agent-1".into(),
        name: "Test Agent".into(),
        model: "deepseek-chat".into(),
        system_prompt: "You are helpful.".into(),
        tools_json: r#"["read","write"]"#.into(),
        mcp_servers_json: "[]".into(),
        max_turns: 20,
        max_tool_calls: 30,
        token_budget: 128000,
        thinking: false,
        reasoning_effort: "medium".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    store.create_agent(&config).await.unwrap();

    let loaded = store.get_agent("agent-1").await.unwrap().unwrap();
    assert_eq!(loaded.name, "Test Agent");

    let agents = store.list_agents().await.unwrap();
    assert_eq!(agents.len(), 1);

    store.delete_agent("agent-1").await.unwrap();
    let gone = store.get_agent("agent-1").await.unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn test_unified_store_transcript() {
    let store = UnifiedStore::in_memory().unwrap();

    let entry = orion_agent::session::unified::TranscriptEntry {
        id: "msg-1".into(),
        parent_id: None,
        role: "user".into(),
        content: "Hello, agent!".into(),
        tool_calls: None,
        timestamp: chrono::Utc::now(),
    };

    store.append_transcript("session-1", &entry).await.unwrap();

    let entries = store.get_transcript("session-1").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "Hello, agent!");
}

#[tokio::test]
async fn test_unified_store_turns_with_tool_calls() {
    let store = UnifiedStore::in_memory().unwrap();

    // Create session first (FK constraint)
    let meta = orion_agent::session::unified::SessionMeta {
        session_id: "session-tc".into(),
        agent_name: "test".into(),
        model: "test".into(),
        working_dir: "/tmp".into(),
        status: orion_agent::session::unified::SessionStatus::Active,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        turn_count: 0,
        tool_call_count: 0,
        total_tokens: 0,
    };
    store.create_session(&meta).await.unwrap();

    // Save a turn
    let turn = orion_agent::session::unified::Turn {
        turn_id: "turn-1".into(),
        session_id: "session-tc".into(),
        turn_index: 0,
        role: "assistant".into(),
        content: "Let me read that file.".into(),
        thinking: None,
        tool_calls: vec![],
        created_at: chrono::Utc::now(),
        tokens_used: 50,
    };
    store.save_turn(&turn).await.unwrap();

    // Save tool calls for the turn
    let tc = orion_agent::session::unified::ToolCallRecord {
        call_id: "tc-1".into(),
        tool_name: "read".into(),
        input_summary: "path=/etc/hosts".into(),
        output_summary: "127.0.0.1 localhost".into(),
        success: true,
        duration_ms: 42,
    };
    store
        .save_tool_call(&tc, "turn-1", "session-tc")
        .await
        .unwrap();

    // Get turns should include tool calls
    let turns = store.get_turns("session-tc").await.unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].tool_calls.len(), 1);
    assert_eq!(turns[0].tool_calls[0].tool_name, "read");
}

// ── Redaction Tests ──

#[test]
fn test_redact_multibyte_utf8() {
    use orion_agent::logging::redact::redact_value;
    // This used to panic with byte slicing
    let result = redact_value("api_key", "你好世界测试密钥");
    assert!(result.contains("***"));
    assert!(!result.contains("你好世界测试密钥")); // Should be redacted
}

#[test]
fn test_redact_ascii() {
    use orion_agent::logging::redact::redact_value;
    let result = redact_value("api_key", "sk-abc123def456");
    assert_eq!(result, "sk-a***");
}

// ── ExecPolicy Tests ──

#[test]
fn test_execpolicy_blocks_rm_rf() {
    use orion_agent::core::execpolicy::{Decision, ExecPolicy};
    let policy = ExecPolicy::load_default();
    let decision = policy.check("rm -rf /");
    assert_eq!(decision, Decision::Forbid);
}

#[test]
fn test_execpolicy_blocks_rm_r_f_slash() {
    use orion_agent::core::execpolicy::{Decision, ExecPolicy};
    let policy = ExecPolicy::load_default();
    // This was the bypass: rm -r -f / doesn't contain "-rf" as substring
    let decision = policy.check("rm -r -f /");
    assert_eq!(decision, Decision::Forbid);
}

#[test]
fn test_execpolicy_allows_safe_commands() {
    use orion_agent::core::execpolicy::{Decision, ExecPolicy};
    let policy = ExecPolicy::load_default();
    let decision = policy.check("ls -la");
    assert_eq!(decision, Decision::Allow);
}

#[test]
fn test_execpolicy_matches_full_path() {
    use orion_agent::core::execpolicy::{Decision, ExecPolicy};
    let policy = ExecPolicy::load_default();
    // /usr/bin/rm should match the "rm" rule
    let decision = policy.check("/usr/bin/rm -rf /");
    assert_eq!(decision, Decision::Forbid);
}
