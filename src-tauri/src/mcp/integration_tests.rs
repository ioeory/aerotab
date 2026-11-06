use super::*;
use rmcp::{transport::StreamableHttpClientTransport, ServiceExt};

#[tokio::test]
async fn streamable_http_roundtrip_permissions_and_revocation() {
    let state = AppState::new();
    let dispatcher = Arc::new(Dispatcher::new());
    crate::commands::register_all(&dispatcher, state.clone());
    let config = McpConfig {
        enabled: true,
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let auth = Arc::new(RwLock::new(http::AuthState {
        token: "integration-test-token".into(),
        cancel: CancellationToken::new(),
    }));
    let server = McpServer::new(state, dispatcher, config);
    let (address, future, handle) = http::prepare(server, auth.clone(), cancel.clone())
        .await
        .unwrap();
    let task = tokio::spawn(future);
    let url = format!("http://127.0.0.1:{}/mcp", address.port());
    let client = reqwest::Client::new();
    let denied = client.post(&url).body("{}").send().await.unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);
    let transport = StreamableHttpClientTransport::with_client(
        client,
        rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
            url.clone(),
        )
        .auth_header("integration-test-token"),
    );
    let service = tokio::time::timeout(std::time::Duration::from_secs(10), ().serve(transport))
        .await
        .unwrap()
        .unwrap();
    let listed = service.list_tools(None).await.unwrap();
    assert!(listed.tools.iter().any(|tool| tool.name == "run_command"));
    let help = service
        .call_tool(CallToolRequestParam {
            name: "help".into(),
            arguments: None,
        })
        .await
        .unwrap();
    assert_ne!(help.is_error, Some(true));
    let denied = service
        .call_tool(CallToolRequestParam {
            name: "run_command".into(),
            arguments: Some(serde_json::Map::new()),
        })
        .await
        .unwrap();
    assert_eq!(denied.is_error, Some(true));
    {
        let mut token = auth.write().unwrap();
        token.cancel.cancel();
        token.token = "rotated-test-token".into();
        token.cancel = CancellationToken::new();
    }
    assert!(service.list_tools(None).await.is_err());
    let _ = service.cancel().await;
    cancel.cancel();
    handle.shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
