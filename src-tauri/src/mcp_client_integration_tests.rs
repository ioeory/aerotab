use super::*;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{service::RequestContext, ErrorData, RoleServer, ServerHandler};
#[derive(Clone)]
struct Fixture;
impl ServerHandler for Fixture {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "third-party-fixture".into(),
                version: "1".into(),
                ..Default::default()
            },
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            ..Default::default()
        }
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        serde_json::from_value(
            json!({"tools":[{"name":"fixture","inputSchema":{"type":"object"}}]}),
        )
        .map_err(|_| ErrorData::internal_error("fixture", None))
    }
    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::default())
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        serde_json::from_value(json!({"resourceTemplates":[{"uriTemplate":"fixture://{path}","name":"Parameterized fixture"}]})).map_err(|_|ErrorData::internal_error("fixture",None))
    }
    async fn read_resource(
        &self,
        p: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        serde_json::from_value(json!({"contents":[{"uri":p.uri,"text":"template content"}]}))
            .map_err(|_| ErrorData::internal_error("fixture", None))
    }
    async fn list_prompts(
        &self,
        _: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::default())
    }
    async fn call_tool(
        &self,
        p: CallToolRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        Ok(CallToolResult::structured(json!({"untrusted":p.arguments})))
    }
}
#[tokio::test]
async fn streamable_http_transport_is_real_trusted_and_reconnectable() {
    let service = StreamableHttpService::new(
        || Ok(Fixture),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig {
            stateful_mode: false,
            ..Default::default()
        },
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
    let store = SettingsStore::open(&dir).unwrap();
    let cfg = ClientConfig {
        id: "http".into(),
        name: "HTTP".into(),
        transport: TransportConfig::Http {
            url: format!("http://127.0.0.1:{port}/mcp"),
        },
    };
    save(&store, vec![cfg.clone()]).unwrap();
    let runtime = ClientRuntime::default();
    assert!(runtime.connect(&store, "http", None).await.is_err());
    trust(&store, &cfg, true).unwrap();
    assert!(runtime.connect(&store, "http", Some(port)).await.is_err());
    let catalog = runtime.connect(&store, "http", None).await.unwrap();
    assert_eq!(catalog["tools"][0]["name"], "fixture");
    assert_eq!(
        catalog["resourceTemplates"][0]["uriTemplate"],
        "fixture://{path}"
    );
    let resource = runtime
        .request("http", "resources/read", json!({"uri":"fixture://chosen"}))
        .await
        .unwrap();
    assert_eq!(resource["contents"][0]["uri"], "fixture://chosen");
    let result = runtime
        .request(
            "http",
            "tools/call",
            json!({"name":"fixture","arguments":{"value":"local"}}),
        )
        .await
        .unwrap();
    assert!(result.to_string().contains("local"));
    runtime.disconnect("http").await;
    assert!(runtime
        .request("http", "tools/list", json!({}))
        .await
        .is_err());
    runtime.connect(&store, "http", None).await.unwrap();
    runtime.disconnect_all().await;
    server.abort();
}
