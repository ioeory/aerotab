//! Permission-scoped MCP tools. No raw terminal output consumers or secret RPCs.
use serde_json::Value;

pub fn definitions() -> Vec<Value> {
    Vec::new()
}
pub fn permission(_name: &str) -> Option<&'static str> {
    None
}
pub fn rpc_permission(_method: &str) -> Option<&'static str> {
    None
}
fn redact(value: Value) -> Value {
    value
}
fn limit(_args: &Value, _key: &str, default: u64, _maximum: u64) -> Result<u64, String> {
    Ok(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_advertised_tool_has_permission_and_schema() {
        let tools = definitions();
        assert_eq!(tools.len(), 27);
        let mut names = std::collections::HashSet::new();
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            assert!(names.insert(name.to_owned()), "duplicate tool {name}");
            assert!(permission(name).is_some(), "no permission for {name}");
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
    }

    #[test]
    fn raw_rpc_is_deny_by_default_and_categorized() {
        for method in [
            "secret.getMaster",
            "vault.get",
            "settings.get",
            "settings.set",
            "profile.configure",
            "profile.importApply",
            "plugin.call",
            "session.poll",
            "session.pollOutput",
            "session.write",
            "session.openLocal",
            "sftp.read",
            "sftp.write",
            "sftp.readChunk",
            "sftp.open",
            "sync.now",
            "ssh.knownHosts.remove",
        ] {
            assert_eq!(rpc_permission(method), None, "unsafe RPC {method}");
        }
        assert_eq!(rpc_permission("profile.list"), Some("listProfiles"));
        assert_eq!(rpc_permission("terminal.exportText"), Some("readTerminal"));
        assert_eq!(rpc_permission("tunnel.open"), Some("tunnels"));
        assert_eq!(permission("call_rpc"), Some("callRpc"));
    }

    #[test]
    fn recursively_redacts_profile_credentials() {
        let value = redact(
            json!({"name":"safe", "ssh":{"auth":{"Password":{"password":"hidden"}}, "jump_via":[{"auth":{"PublicKey":{"key_path":"secret"}},"host":"jump"}], "host":"target"}, "password":"hidden", "privateKey":"hidden"}),
        );
        let text = value.to_string();
        assert!(!text.contains("hidden"));
        assert!(!text.contains("secret"));
        assert_eq!(value["ssh"]["host"], "target");
        assert_eq!(value["ssh"]["jump_via"][0]["host"], "jump");
    }

    #[test]
    fn rejects_out_of_range_or_wrong_type_limits() {
        assert!(limit(&json!({"max_bytes":0}), "max_bytes", 10, 100).is_err());
        assert!(limit(&json!({"max_bytes":101}), "max_bytes", 10, 100).is_err());
        assert!(limit(&json!({"max_bytes":"12"}), "max_bytes", 10, 100).is_err());
        assert_eq!(limit(&json!({}), "max_bytes", 10, 100).unwrap(), 10);
    }
}
