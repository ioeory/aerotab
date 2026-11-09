use super::*;

#[test]
fn phase_six_tools_require_independent_grants_and_no_approval_escape() {
    let mut config = McpConfig::default();
    config.permissions.insert("runCommand".into(), true);
    for (tool, grant) in [
        ("search_notes", "readNotes"),
        ("read_note", "readNotes"),
        ("write_note", "writeNotes"),
        ("list_workspaces", "readWorkspaces"),
        ("workspace_snapshot", "readWorkspaces"),
        ("batch_plan", "batchOperations"),
        ("batch_execute", "batchOperations"),
        ("batch_status", "batchOperations"),
        ("batch_results", "batchOperations"),
        ("batch_cancel", "batchOperations"),
    ] {
        assert_eq!(tools::permission(tool), Some(grant));
        assert!(permissions::authorize(&config, tool, &json!({})).is_err());
        config.permissions.insert(grant.into(), true);
        assert!(permissions::authorize(&config, tool, &json!({})).is_ok());
        config.permissions.insert(grant.into(), false);
    }
    config.permissions.insert("callRpc".into(), true);
    for method in [
        "mcp.batchApprove",
        "scripts.prepare",
        "scripts.run",
        "notes.put",
        "settings.get",
    ] {
        assert!(permissions::authorize(&config, "call_rpc", &json!({"method":method})).is_err());
    }
}

#[test]
fn phase_six_capabilities_are_real() {
    let server = McpServer::new(
        AppState::new(),
        Arc::new(Dispatcher::new()),
        McpConfig::default(),
    );
    let info = server.get_info();
    assert!(info.capabilities.resources.is_some());
    assert!(info.capabilities.prompts.is_some());
}

#[test]
fn rotating_token_revokes_inflight_generation() {
    let runtime = McpRuntime::new();
    runtime.replace_token("first".into());
    let previous = runtime.auth.read().unwrap().cancel.clone();
    runtime.replace_token("second".into());
    assert!(previous.is_cancelled());
    assert!(!runtime.auth.read().unwrap().cancel.is_cancelled());
    assert!(runtime.status().token_configured);
}

#[test]
fn runtime_drop_revokes_auth_generation() {
    let runtime = McpRuntime::new();
    runtime.replace_token("first".into());
    let generation = runtime.auth.read().unwrap().cancel.clone();
    drop(runtime);
    assert!(generation.is_cancelled());
}

#[test]
fn official_sdk_accepts_all_tool_definitions() {
    let definitions = tools::definitions();
    assert!(!definitions.is_empty());
    for definition in definitions {
        let tool: Tool = serde_json::from_value(definition).unwrap();
        assert!(!tool.name.is_empty());
    }
}

#[tokio::test]
async fn denied_calls_are_structured_mcp_errors() {
    let server = McpServer::new(
        Arc::new(AppState::default()),
        Arc::new(Dispatcher::new()),
        McpConfig::default(),
    );
    for name in ["unknown_tool", "run_command", "call_rpc"] {
        let result = server.invoke(name, json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.unwrap()["error"].is_string());
        assert!(!result.content.is_empty());
    }
}

#[test]
fn registered_management_rpc_does_not_retain_runtime() {
    let dispatcher = Arc::new(Dispatcher::new());
    let runtime = Arc::new(McpRuntime::new());
    let weak = Arc::downgrade(&runtime);
    register_rpc(&dispatcher, Arc::new(AppState::default()), runtime.clone());
    drop(runtime);
    assert!(weak.upgrade().is_none());
}
