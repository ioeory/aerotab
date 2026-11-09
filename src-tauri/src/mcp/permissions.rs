//! Permission checks are deliberately fail-closed and run for every tool call.
use super::config::McpConfig;
use serde_json::Value;

pub fn authorize(config: &McpConfig, name: &str, args: &Value) -> Result<(), String> {
    let category = super::tools::permission(name).ok_or_else(|| format!("Unknown tool: {name}"))?;
    if !config.permitted(category) {
        return Err(format!("Permission denied: {category}"));
    }
    if name == "batch_execute" && !config.permitted("runCommand") {
        return Err("Permission denied: runCommand".into());
    }
    if name == "open_sftp" && !config.permitted("readFiles") && !config.permitted("writeFiles") {
        return Err("Permission denied: SFTP requires readFiles or writeFiles".into());
    }
    if name == "call_rpc" {
        let method = args
            .get("method")
            .and_then(Value::as_str)
            .ok_or("call_rpc requires method")?;
        let category = super::tools::rpc_permission(method)
            .ok_or_else(|| format!("RPC method is not exposed to MCP: {method}"))?;
        if !config.permitted(category) {
            return Err(format!("Permission denied: {category}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_allow_only_readonly_categories() {
        let cfg = McpConfig::default();
        assert!(cfg.permitted("listProfiles"));
        assert!(cfg.permitted("readTerminal"));
        for permission in [
            "runCommand",
            "readFiles",
            "writeFiles",
            "openSession",
            "manageProfiles",
            "remoteDesktop",
            "tunnels",
            "callRpc",
        ] {
            assert!(!cfg.permitted(permission));
        }
        assert!(!cfg.permitted("unknown"));
    }

    #[test]
    fn rpc_requires_both_outer_and_underlying_permissions() {
        let mut cfg = McpConfig::default();
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"profile.list"})).is_err());
        cfg.permissions.insert("callRpc".into(), true);
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"profile.list"})).is_ok());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"tunnel.list"})).is_err());
        cfg.permissions.insert("tunnels".into(), true);
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"tunnel.list"})).is_ok());
        cfg.permissions.insert("tunnels".into(), false);
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"tunnel.list"})).is_err());
    }

    #[test]
    fn sftp_open_requires_session_and_file_grants() {
        let mut config = McpConfig::default();
        let args = json!({"profile_id":"00000000-0000-0000-0000-000000000000"});
        assert!(authorize(&config, "open_sftp", &args).is_err());
        config.permissions.insert("openSession".into(), true);
        assert!(authorize(&config, "open_sftp", &args).is_err());
        config.permissions.insert("readFiles".into(), true);
        assert!(authorize(&config, "open_sftp", &args).is_ok());
    }

    #[test]
    fn sidebar_resource_rpcs_are_not_implicitly_exposed() {
        let mut cfg = McpConfig::default();
        // Even granting all existing categories must not open new content or execution APIs.
        for permission in [
            "callRpc",
            "listProfiles",
            "manageProfiles",
            "runCommand",
            "readFiles",
            "writeFiles",
            "tunnels",
            "openSession",
        ] {
            cfg.permissions.insert(permission.into(), true);
        }
        for method in [
            "notes.list",
            "notes.upsert",
            "notes.export",
            "scripts.list",
            "scripts.run",
            "scripts.tasks",
            "proxy.list",
            "proxy.upsert",
            "vault.get",
            "vault.list",
        ] {
            assert!(
                authorize(&cfg, "call_rpc", &json!({"method": method})).is_err(),
                "new resource method unexpectedly exposed: {method}"
            );
        }
    }

    #[test]
    fn unknown_tools_and_rpc_methods_fail_closed() {
        let mut cfg = McpConfig::default();
        cfg.permissions.insert("callRpc".into(), true);
        assert!(authorize(&cfg, "not_a_tool", &json!({})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"secret.getMaster"})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"mcp.getToken"})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"session.write"})).is_err());
        assert!(authorize(
            &cfg,
            "call_rpc",
            &json!({"method":"terminal.snapshotEnvelope"})
        )
        .is_err());
    }
}
