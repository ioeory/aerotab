//! Local-only note/workspace access and permission-scoped MCP context catalog.
use super::{config::McpConfig, tools};
use crate::{commands::AppState, ipc::Dispatcher};
use serde_json::{json, Value};

pub const TOOLS: &[(&str,&str,&str)] = &[
    ("search_notes","readNotes","Search ordinary notes by text and optional associated profile_id."),
    ("read_note","readNotes","Read one ordinary note, including its revision and profile associations."),
    ("write_note","writeNotes","Write an ordinary note with required expectedRevision and expectedVersion; no Vault access."),
    ("list_workspaces","readWorkspaces","List saved local workspaces without connecting to any host."),
    ("workspace_snapshot","readWorkspaces","Read a sanitized saved workspace; never opens remote sessions."),
];

pub async fn call(
    name: &str,
    args: Value,
    d: &Dispatcher,
    state: &AppState,
) -> Result<Value, String> {
    match name {
        "search_notes" | "read_note" => {
            let store = state
                .notes
                .lock()
                .await
                .clone()
                .ok_or("notes unavailable")?;
            let snapshot = crate::notes::dispatch(&store, "notes.list", json!({}), &[])?;
            let notes = snapshot["notes"].as_array().ok_or("invalid notes")?;
            if name == "read_note" {
                let note = notes
                    .iter()
                    .find(|n| n["id"] == args["id"])
                    .ok_or("note not found")?;
                return Ok(json!({"note":note,"revision":snapshot["revision"]}));
            }
            let query = args["query"].as_str().unwrap_or("").to_lowercase();
            let profile = args["profile_id"].as_str();
            let found:Vec<_>=notes.iter().filter(|n| {
                (query.is_empty() || ["title","markdown"].iter().any(|key| n[*key].as_str().unwrap_or("").to_lowercase().contains(&query)) || n["tags"].to_string().to_lowercase().contains(&query))
                && profile.map_or(true, |id|n["profile_ids"].as_array().is_some_and(|ids|ids.iter().any(|v|v==id)))
            }).take(100).map(|n|json!({"id":n["id"],"title":n["title"],"tags":n["tags"],"profile_ids":n["profile_ids"],"version":n["version"]})).collect();
            Ok(json!({"notes":found,"revision":snapshot["revision"]}))
        }
        "write_note" => tools::dispatch(d, "notes.put", args).await,
        "list_workspaces" | "workspace_snapshot" => {
            let store = state
                .settings
                .lock()
                .await
                .clone()
                .ok_or("settings unavailable")?;
            let value = store
                .get("sessionWorkspaces")
                .map_err(|_| "workspace read failed")?
                .unwrap_or(json!([]));
            let workspaces = value.as_array().ok_or("invalid workspaces")?;
            if name == "workspace_snapshot" {
                return workspaces
                    .iter()
                    .find(|w| w["id"] == args["id"])
                    .map(workspace_snapshot)
                    .ok_or("workspace not found".into());
            }
            Ok(
                json!({"workspaces":workspaces.iter().take(100).map(|w|json!({"id":w["id"],"name":w["name"],"updatedAt":w["updatedAt"],"tabCount":w["tabs"].as_array().map_or(0,Vec::len)})).collect::<Vec<_>>()}),
            )
        }
        _ => Err("unknown context tool".into()),
    }
}

/// Strict projection rather than recursive key redaction: inline SSH/shell recipes
/// may contain credentials under arbitrary user-controlled keys.
pub fn workspace_snapshot(w: &Value) -> Value {
    let tabs:Vec<_>=w["tabs"].as_array().into_iter().flatten().take(100).map(|t| {
        let panes:Vec<_>=t["panes"].as_array().into_iter().flatten().take(100).map(|p|match p["kind"].as_str(){
            Some("ssh-profile")=>json!({"kind":"ssh-profile","id":p["id"]}),
            Some("local")=>json!({"kind":"local"}),
            _=>json!({"kind":"redacted"}),
        }).collect();
        json!({"title":t["title"],"layout":layout(&t["layout"],0),"activePaneIndex":t["activePaneIndex"],"panes":panes,"sftpDock":t["sftpDock"]==true,"sftpDockCollapsed":t["sftpDockCollapsed"]==true})
    }).collect();
    json!({"id":w["id"],"name":w["name"],"createdAt":w["createdAt"],"updatedAt":w["updatedAt"],"tabs":tabs})
}
fn layout(v: &Value, depth: usize) -> Value {
    if depth > 16 {
        return Value::Null;
    }
    if v["type"] == "leaf" {
        json!({"type":"leaf","paneIndex":v["paneIndex"].as_u64()})
    } else {
        json!({"type":"split","direction":if v["direction"]=="vertical"{"vertical"}else{"horizontal"},"ratios":v["ratios"].as_array().map(|r|r.iter().take(100).filter_map(Value::as_f64).collect::<Vec<_>>()),"children":v["children"].as_array().map(|r|r.iter().take(100).map(|v|layout(v,depth+1)).collect::<Vec<_>>())})
    }
}

pub fn resources(config: &McpConfig) -> Vec<Value> {
    [
        ("aerotab://hosts", "Host summaries", "listProfiles"),
        ("aerotab://notes", "Ordinary notes", "readNotes"),
        ("aerotab://terminals", "Terminal contexts", "readTerminal"),
    ]
    .into_iter()
    .filter(|(_, _, g)| config.permitted(g))
    .map(|(uri, name, _)| json!({"uri":uri,"name":name,"mimeType":"application/json"}))
    .collect()
}
pub async fn read_resource(
    uri: &str,
    config: &McpConfig,
    d: &std::sync::Arc<Dispatcher>,
    state: &std::sync::Arc<AppState>,
) -> Result<Value, String> {
    let (tool, args) = match uri {
        "aerotab://hosts" => ("list_profiles", json!({})),
        "aerotab://notes" => ("search_notes", json!({})),
        "aerotab://terminals" => ("list_sessions", json!({})),
        _ if uri.starts_with("aerotab://notes/") => (
            "read_note",
            json!({"id":uri.trim_start_matches("aerotab://notes/")}),
        ),
        _ if uri.starts_with("aerotab://terminals/") => (
            "terminal_export",
            json!({"id":uri.trim_start_matches("aerotab://terminals/"),"max_bytes":65536}),
        ),
        _ => return Err("unknown resource".into()),
    };
    super::permissions::authorize(config, tool, &args)?;
    if TOOLS.iter().any(|t| t.0 == tool) {
        call(tool, args, d, state).await
    } else {
        tools::call(tool, args, d.clone(), state.clone()).await
    }
}
