use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, Default, PartialEq)]
struct Args {
    url: Option<String>,
    data_dir: Option<PathBuf>,
    help: bool,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut result = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => result.help = true,
            Some("--url") if result.url.is_none() => {
                let value = args.next().ok_or("--url requires a value")?;
                let value = value.into_string().map_err(|_| "--url must be UTF-8")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err("--url requires a value".into());
                }
                result.url = Some(value);
            }
            Some("--data-dir") if result.data_dir.is_none() => {
                let value = args.next().ok_or("--data-dir requires a value")?;
                if value.is_empty() || value.to_string_lossy().starts_with('-') {
                    return Err("--data-dir requires a value".into());
                }
                result.data_dir = Some(value.into());
            }
            _ => return Err("unknown or duplicate argument; use --help".into()),
        }
    }
    Ok(result)
}

fn resolve_data_dir(
    explicit: Option<PathBuf>,
    get: impl Fn(&str) -> Option<OsString>,
    os: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let get = |key| {
        get(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(path) = get("AEROTAB_DATA_DIR") {
        return Ok(path);
    }
    let root = match os {
        "windows" => get("APPDATA"),
        "macos" => get("HOME").map(|home| home.join("Library/Application Support")),
        _ => get("XDG_DATA_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| get("HOME").map(|home| home.join(".local/share"))),
    };
    root.map(|root| root.join("com.aerotab"))
        .ok_or_else(|| "cannot locate application data; specify --data-dir".into())
}

// The desktop publishes this descriptor only while its enabled MCP listener runs.
// Do not open sled merely to discover it: the desktop owns those exclusive locks.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    url: String,
    pid: u32,
}

fn read_endpoint(dir: &std::path::Path) -> anyhow::Result<Option<Endpoint>> {
    use std::io::Read;
    let file = match std::fs::File::open(dir.join("mcp-endpoint.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => anyhow::bail!("cannot read MCP endpoint descriptor"),
    };
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 8192, "MCP endpoint descriptor exceeds 8 KiB");
    let endpoint: Endpoint = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid MCP endpoint descriptor"))?;
    anyhow::ensure!(endpoint.pid > 0, "invalid MCP endpoint process ID");
    validate_url(&endpoint.url, false)?;
    Ok(Some(endpoint))
}

fn validate_url(raw: &str, explicit: bool) -> anyhow::Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid MCP endpoint URL"))?;
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "MCP endpoint must not contain credentials"
    );
    anyhow::ensure!(
        url.fragment().is_none() && url.query().is_none(),
        "MCP endpoint must not contain a query or fragment"
    );
    let local = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && local),
        "MCP requires HTTPS except on loopback"
    );
    anyhow::ensure!(
        explicit || local,
        "discovered MCP endpoint must be loopback; use --url for an explicit remote endpoint"
    );
    anyhow::ensure!(url.host_str().is_some(), "MCP endpoint requires a host");
    Ok(url)
}

fn process_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        match std::fs::metadata(std::path::Path::new("/proc").join(pid.to_string())) {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_or(true, |status| status.success())
    }
    #[cfg(windows)]
    {
        // Failure to inspect a process must never authorize a headless fallback.
        std::process::Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .map_or(true, |output| {
                !output.status.success()
                    || String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        true
    }
}

#[derive(Clone)]
struct HttpProxy {
    peer: rmcp::Peer<rmcp::RoleClient>,
    info: rmcp::model::ServerInfo,
}

impl rmcp::ServerHandler for HttpProxy {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        self.peer.list_tools(request).await.map_err(proxy_error)
    }

    async fn list_resources(
        &self,
        request: Option<rmcp::model::PaginatedRequestParam>,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, rmcp::ErrorData> {
        self.peer.list_resources(request).await.map_err(proxy_error)
    }
    async fn list_resource_templates(
        &self,
        request: Option<rmcp::model::PaginatedRequestParam>,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, rmcp::ErrorData> {
        self.peer
            .list_resource_templates(request)
            .await
            .map_err(proxy_error)
    }
    async fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, rmcp::ErrorData> {
        self.peer.read_resource(request).await.map_err(proxy_error)
    }
    async fn list_prompts(
        &self,
        request: Option<rmcp::model::PaginatedRequestParam>,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, rmcp::ErrorData> {
        self.peer.list_prompts(request).await.map_err(proxy_error)
    }
    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParam,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::GetPromptResult, rmcp::ErrorData> {
        self.peer.get_prompt(request).await.map_err(proxy_error)
    }
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        self.peer.call_tool(request).await.map_err(proxy_error)
    }
}

fn proxy_error(error: rmcp::service::ServiceError) -> rmcp::ErrorData {
    match error {
        rmcp::service::ServiceError::McpError(error) => error,
        _ => rmcp::ErrorData::internal_error("desktop MCP connection failed", None),
    }
}

async fn proxy(url: reqwest::Url) -> anyhow::Result<()> {
    use rmcp::{
        transport::streamable_http_client::StreamableHttpClientTransportConfig, ServiceExt,
    };
    let token = match std::env::var("AEROTAB_MCP_TOKEN") {
        Ok(token) if !token.trim().is_empty() => token,
        Ok(_) => anyhow::bail!("AEROTAB_MCP_TOKEN is empty"),
        Err(std::env::VarError::NotPresent) => aerotab_core::secret::get_secret("mcp.apiKey")
            .map_err(|_| {
                anyhow::anyhow!(
                    "MCP API token unavailable; configure the desktop keyring or AEROTAB_MCP_TOKEN"
                )
            })?,
        Err(_) => anyhow::bail!("AEROTAB_MCP_TOKEN must be valid UTF-8"),
    };
    anyhow::ensure!(!token.trim().is_empty(), "MCP API token is empty");
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()?;
    let transport = rmcp::transport::StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(url.as_str()).auth_header(token),
    );
    // Never print the SDK error: a transport error may contain endpoint credentials.
    // In particular auth and TLS failures are fatal, never headless fallbacks.
    let upstream = tokio::time::timeout(std::time::Duration::from_secs(15), ().serve(transport))
        .await
        .map_err(|_| anyhow::anyhow!("desktop MCP connection timed out; no headless fallback attempted"))?
        .map_err(|_| anyhow::anyhow!("desktop MCP connection failed; verify enablement, API token and TLS trust; no headless fallback attempted"))?;
    let info = upstream.peer_info().cloned().unwrap_or_default();
    let handler = HttpProxy {
        peer: upstream.peer().clone(),
        info,
    };
    let service = handler.serve(rmcp::transport::stdio()).await?;
    let result = service.waiting().await;
    let _ = upstream.cancel().await;
    result?;
    Ok(())
}

async fn headless(dir: &std::path::Path) -> anyhow::Result<()> {
    use aerotab_core::{
        commands::{register_all, AppState},
        ipc::Dispatcher,
        mcp::{McpConfig, McpServer},
        profile::ProfileStore,
        settings::SettingsStore,
        ssh::known_hosts::KnownHosts,
        vault::VaultStore,
    };
    use anyhow::Context;
    use rmcp::ServiceExt;
    use std::sync::Arc;

    // Never create database snapshots, recover, rename or reset existing stores.
    anyhow::ensure!(
        dir.join("settings.sled").is_dir(),
        "no existing AeroTab settings; enable MCP in the desktop first"
    );
    let settings = SettingsStore::open(dir)
        .context("cannot exclusively open settings; close the desktop or use its HTTP endpoint (no recovery attempted)")?;
    let config: McpConfig = serde_json::from_value(settings.get("mcp")?.unwrap_or_default())
        .context("MCP settings missing or invalid; enable MCP in the desktop first")?;
    anyhow::ensure!(config.enabled, "MCP is disabled in stored settings");
    config.validate().map_err(anyhow::Error::msg)?;
    let profiles = ProfileStore::open(dir.join("profiles.sled"))
        .context("cannot exclusively open profiles; no database recovery attempted")?;
    let known_hosts =
        KnownHosts::open(dir).context("cannot open known hosts; no database recovery attempted")?;
    let vault = VaultStore::open(dir)
        .context("cannot exclusively open vault; no database recovery attempted")?;
    let state = AppState::new();
    *state.settings.lock().await = Some(settings);
    *state.profiles.lock().await = Some(profiles);
    *state.known_hosts.lock().await = Some(known_hosts);
    *state.vault.lock().await = Some(vault);
    *state.notes.lock().await =
        Some(aerotab_core::notes::NotesStore::open(dir.join("notes")).map_err(anyhow::Error::msg)?);
    *state.scripts.lock().await = Some(Arc::new(aerotab_core::scripts::ScriptsService::new(
        aerotab_core::scripts::ScriptsStore::open(dir.join("scripts"))
            .map_err(anyhow::Error::msg)?,
    )));
    // Headless stdio can prepare plans but cannot create a visible local approval.
    let dispatcher = Arc::new(Dispatcher::new());
    register_all(&dispatcher, state.clone());
    McpServer::new(state, dispatcher, config)
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    // No tracing subscriber: SDK diagnostics must never contaminate protocol stdout
    // or emit HTTP authorization headers through caller-supplied log filters.
    if let Err(error) = run().await {
        eprintln!("aerotab-mcp: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let args = parse_args(std::env::args_os().skip(1)).map_err(anyhow::Error::msg)?;
    if args.help {
        eprintln!("AeroTab MCP stdio bridge\n\nUsage: aerotab-mcp [--url URL] [--data-dir PATH]\n\nConnects to the enabled desktop MCP listener; otherwise opens existing settings\nheadlessly with the same permissions. Exclusive database lock failures are fatal.\n\nEnvironment: AEROTAB_DATA_DIR, AEROTAB_MCP_TOKEN (overrides keyring mcp.apiKey).\nRemote endpoints require explicit --url and HTTPS. Redirects are never followed.");
        return Ok(());
    }
    if let Some(url) = args.url {
        return proxy(validate_url(&url, true)?).await;
    }
    let dir = resolve_data_dir(
        args.data_dir,
        |key| std::env::var_os(key),
        std::env::consts::OS,
    )
    .map_err(anyhow::Error::msg)?;
    if let Some(endpoint) = read_endpoint(&dir)? {
        if process_alive(endpoint.pid) {
            return proxy(validate_url(&endpoint.url, false)?).await;
        }
        // A stale descriptor contains no credentials and does not authorize MCP.
        // Headless must independently load and enforce the stored enabled flag.
    }
    headless(&dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Args, String> {
        parse_args(values.iter().map(OsString::from))
    }

    #[test]
    fn endpoint_urls_never_accept_credentials_or_insecure_remote_hosts() {
        for url in [
            "http://example.com/mcp",
            "https://user:secret@example.com/mcp",
            "https://example.com/mcp?token=secret",
            "https://example.com/mcp#secret",
            "file:///tmp/mcp",
        ] {
            assert!(validate_url(url, true).is_err());
        }
        assert!(validate_url("https://example.com/mcp", true).is_ok());
        assert!(validate_url("https://example.com/mcp", false).is_err());
        assert!(validate_url("http://127.0.0.1:23890/mcp", false).is_ok());
        assert!(validate_url("http://[::1]:23890/mcp", false).is_ok());
    }

    #[test]
    fn descriptor_read_is_bounded_and_missing_is_not_enabled() {
        let dir = std::env::temp_dir().join(format!("aerotab-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_endpoint(&dir).unwrap().is_none());
        std::fs::write(dir.join("mcp-endpoint.json"), vec![b' '; 8193]).unwrap();
        assert!(read_endpoint(&dir).is_err());
        std::fs::write(
            dir.join("mcp-endpoint.json"),
            br#"{"url":"http://127.0.0.1:23890/mcp","pid":1}"#,
        )
        .unwrap();
        assert_eq!(read_endpoint(&dir).unwrap().unwrap().pid, 1);
        std::fs::write(
            dir.join("mcp-endpoint.json"),
            br#"{"url":"https://example.com/mcp","pid":1}"#,
        )
        .unwrap();
        assert!(read_endpoint(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_explicit_endpoint_and_directory() {
        let parsed = args(&[
            "--url",
            "https://localhost:9443/mcp",
            "--data-dir",
            "/tmp/app",
        ])
        .unwrap();
        assert_eq!(parsed.url.as_deref(), Some("https://localhost:9443/mcp"));
        assert_eq!(parsed.data_dir, Some(PathBuf::from("/tmp/app")));
    }

    #[test]
    fn accepts_defaults_and_help() {
        assert_eq!(args(&[]).unwrap(), Args::default());
        assert!(args(&["--help"]).unwrap().help);
    }

    #[test]
    fn rejects_unknown_duplicate_and_missing_arguments() {
        for values in [
            vec!["--unknown"],
            vec!["--url"],
            vec!["--data-dir"],
            vec!["--url", "a", "--url", "b"],
            vec!["--data-dir", "a", "--data-dir", "b"],
        ] {
            assert!(args(&values).is_err(), "{values:?}");
        }
    }

    #[test]
    fn explicit_data_dir_precedes_environment() {
        let actual = resolve_data_dir(
            Some("/chosen".into()),
            |_| Some("/environment".into()),
            "linux",
        )
        .unwrap();
        assert_eq!(actual, PathBuf::from("/chosen"));
    }

    #[test]
    fn environment_data_dir_precedes_platform_defaults() {
        let actual = resolve_data_dir(
            None,
            |name| (name == "AEROTAB_DATA_DIR").then(|| "/override".into()),
            "linux",
        )
        .unwrap();
        assert_eq!(actual, PathBuf::from("/override"));
    }

    #[test]
    fn resolves_platform_data_directories() {
        for (os, key, root, expected) in [
            ("linux", "XDG_DATA_HOME", "/data", "/data/com.aerotab"),
            (
                "linux",
                "HOME",
                "/home/me",
                "/home/me/.local/share/com.aerotab",
            ),
            (
                "macos",
                "HOME",
                "/Users/me",
                "/Users/me/Library/Application Support/com.aerotab",
            ),
            (
                "windows",
                "APPDATA",
                "C:/Users/me/AppData/Roaming",
                "C:/Users/me/AppData/Roaming/com.aerotab",
            ),
        ] {
            let actual =
                resolve_data_dir(None, |name| (name == key).then(|| root.into()), os).unwrap();
            assert_eq!(actual, PathBuf::from(expected));
        }
        assert!(resolve_data_dir(None, |_| None, "linux").is_err());
    }
}
