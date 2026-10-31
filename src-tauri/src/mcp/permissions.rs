//! Permission checks are deliberately fail-closed and run for every tool call.
use super::config::McpConfig;
use serde_json::Value;

pub fn authorize(config: &McpConfig, name: &str, args: &Value) -> Result<(), String> {
    let category = super::tools::permission(name).ok_or_else(|| format!("Unknown tool: {name}"))?;
    if !config.permitted(category) {
        return Err(format!("Permission denied: {category}"));
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
        assert!(!cfg.permitted("executeCommand"));
        assert!(!cfg.permitted("unknown"));
    }

    #[test]
    fn unknown_tools_and_rpc_methods_fail_closed() {
        let mut cfg = McpConfig::default();
        cfg.permissions.insert("callRpc".into(), true);
        assert!(authorize(&cfg, "not_a_tool", &json!({})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"secret.getMaster"})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"mcp.getToken"})).is_err());
        assert!(authorize(&cfg, "call_rpc", &json!({"method":"session.write"})).is_err());
    }
}
