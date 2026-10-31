//! Streamable HTTP transport with per-request authentication and DNS-rebinding guards.
use std::{net::SocketAddr, sync::{Arc, RwLock}};
use axum::{body::{to_bytes, Body}, http::{HeaderMap, Request, StatusCode, uri::Authority}, response::{IntoResponse, Response}, routing::any, Router};
use rmcp::transport::streamable_http_server::{session::local::LocalSessionManager, StreamableHttpService, StreamableHttpServerConfig};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use super::{McpConfig, McpServer};

pub(crate) struct AuthState {
    pub token: String,
    pub cancel: CancellationToken,
}

const MAX_BODY_BYTES: usize = 1024 * 1024;

pub(crate) fn validate_headers(headers: &HeaderMap, config: &McpConfig, token: &str) -> Result<(), StatusCode> {
    // Native MCP clients do not send Origin. Reject all browser origins rather
    // than trying to maintain a permissive cross-origin policy.
    if headers.contains_key("origin") { return Err(StatusCode::FORBIDDEN); }
    if headers.get_all("host").iter().count() != 1 { return Err(StatusCode::FORBIDDEN); }
    let authority = headers.get("host").and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<Authority>().ok()).ok_or(StatusCode::FORBIDDEN)?;
    if authority.port_u16() != Some(config.port) { return Err(StatusCode::FORBIDDEN); }
    let host = authority.host().trim_start_matches('[').trim_end_matches(']');
    let bind_ip = config.bind_ip().map_err(|_| StatusCode::FORBIDDEN)?;
    let host_ip = host.parse::<std::net::IpAddr>().ok();
    let allowed = if bind_ip.is_loopback() {
        host.eq_ignore_ascii_case("localhost") || host_ip.is_some_and(|ip| ip.is_loopback())
    } else if bind_ip.is_unspecified() {
        // Wildcard listeners accept literal destination IPs only. Arbitrary DNS
        // names are not trusted, preventing DNS-rebinding through browser hosts.
        host.eq_ignore_ascii_case("localhost") || host_ip.is_some_and(|ip| !ip.is_unspecified())
    } else {
        host_ip == Some(bind_ip)
    };
    if !allowed { return Err(StatusCode::FORBIDDEN); }
    if headers.get_all("authorization").iter().count() != 1 { return Err(StatusCode::UNAUTHORIZED); }
    let candidate = headers.get("authorization").and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ")).ok_or(StatusCode::UNAUTHORIZED)?;
    if token.is_empty() || !bool::from(candidate.as_bytes().ct_eq(token.as_bytes())) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

pub(crate) async fn prepare(
    server: McpServer,
    token: Arc<RwLock<AuthState>>,
    cancel: CancellationToken,
) -> Result<(SocketAddr, impl std::future::Future<Output = Result<(), String>> + Send + 'static, axum_server::Handle), String> {
    let config = server.config();
    config.validate()?;
    let addr = SocketAddr::new(config.bind_ip()?, config.port);
    let tls = if config.tls_enabled() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Some(axum_server::tls_rustls::RustlsConfig::from_pem_file(
            config.tls_cert_path.as_ref().unwrap(), config.tls_key_path.as_ref().unwrap()
        ).await.map_err(|e| format!("MCP TLS configuration failed: {e}"))?)
    } else { None };
    let listener = std::net::TcpListener::bind(addr).map_err(|e| format!("MCP bind failed: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    // Stateless Streamable HTTP avoids server-side session workers surviving
    // reconfiguration and guarantees each invocation passes the auth middleware.
    let service = StreamableHttpService::new(
        move || Ok(server.clone()), Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig { stateful_mode: false, ..Default::default() }
    );
    let app = Router::new().route("/mcp", any(move |request: Request<Body>| {
        let config = config.clone();
        let service = service.clone();
        let token = token.clone();
        let cancel = cancel.clone();
        async move {
            let request_cancel = {
                let auth = token.read().unwrap_or_else(|e| e.into_inner());
                if let Err(code) = validate_headers(request.headers(), &config, &auth.token) { return code.into_response(); }
                auth.cancel.clone()
            };
            let (parts, body) = request.into_parts();
            let bytes = match to_bytes(body, MAX_BODY_BYTES).await {
                Ok(bytes) => bytes,
                Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
            };
            // Authentication is checked again after reading a potentially slow
            // body, so token rotation cannot authorize a stale in-flight upload.
            {
                let token = token.read().unwrap_or_else(|e| e.into_inner());
                if let Err(code) = validate_headers(&parts.headers, &config, &token.token) { return code.into_response(); }
            }
            tokio::select! {
                _ = cancel.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                _ = request_cancel.cancelled() => StatusCode::UNAUTHORIZED.into_response(),
                response = service.handle(Request::from_parts(parts, Body::from(bytes))) => Response::new(Body::new(response.into_body())),
            }
        }
    }));
    let handle = axum_server::Handle::new();
    let run_handle = handle.clone();
    let future = async move {
        let result = match tls {
            Some(tls) => axum_server::from_tcp_rustls(listener, tls).handle(run_handle).serve(app.into_make_service()).await,
            None => axum_server::from_tcp(listener).handle(run_handle).serve(app.into_make_service()).await,
        };
        result.map_err(|e| format!("MCP HTTP server failed: {e}"))
    };
    Ok((bound, future, handle))
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    #[test]
    fn auth_and_host_checks_fail_closed() {
        let config = McpConfig::default();
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("localhost:23890"));
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer good-token"),
        );
        assert!(validate_headers(&headers, &config, "good-token").is_ok());
        assert!(validate_headers(&headers, &config, "rotated-token").is_err());
        headers.insert("origin", HeaderValue::from_static("https://evil.example"));
        assert!(validate_headers(&headers, &config, "good-token").is_err());
        headers.remove("origin");
        headers.insert("host", HeaderValue::from_static("evil.example:23890"));
        assert!(validate_headers(&headers, &config, "good-token").is_err());
        headers.remove("host");
        assert!(validate_headers(&headers, &config, "good-token").is_err());
    }
}
