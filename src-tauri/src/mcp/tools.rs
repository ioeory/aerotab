//! Permission-scoped MCP tools. No raw terminal output consumers or secret RPCs.
use crate::{
    commands::AppState,
    ipc::{Dispatcher, Request},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const MAX_BYTES: u64 = 1024 * 1024;
const TOOLS: &[(&str, &str, &str)] = &[
    ("help", "listProfiles", "Describe tools and safety limits."),
    ("open_sftp", "openSession", "Open SFTP for a saved SSH profile; returns sftp_id. Also requires readFiles or writeFiles."),
    ("close_sftp", "openSession", "Close an SFTP session opened by the application or MCP."),
    (
        "list_profiles",
        "listProfiles",
        "List saved profiles with credentials redacted.",
    ),
    (
        "get_profile",
        "listProfiles",
        "Read one profile with credentials redacted.",
    ),
    (
        "list_sessions",
        "readTerminal",
        "List sessions without consuming output.",
    ),
    (
        "host_stats",
        "readTerminal",
        "Probe SSH stats over a separate exec channel.",
    ),
    (
        "terminal_export",
        "readTerminal",
        "Read bounded native terminal scrollback.",
    ),
    (
        "terminal_search",
        "readTerminal",
        "Search native terminal scrollback.",
    ),
    (
        "list_files",
        "readFiles",
        "List trusted local paths or an existing SFTP session.",
    ),
    (
        "read_file",
        "readFiles",
        "Read a bounded file chunk as base64.",
    ),
    ("get_file_info", "readFiles", "Read file metadata."),
    (
        "find_file",
        "readFiles",
        "Bounded filename substring search without following symlinks.",
    ),
    (
        "write_file",
        "writeFiles",
        "Write at most 1 MiB base64 data, replacing a file.",
    ),
    (
        "create_file",
        "writeFiles",
        "Create a new local file exclusively; remote exclusive creation unsupported.",
    ),
    (
        "create_directory",
        "writeFiles",
        "Create one directory, without recursive parents.",
    ),
    (
        "remove_path",
        "writeFiles",
        "Remove a file or empty directory, never recursively.",
    ),
    (
        "rename_path",
        "writeFiles",
        "Rename a path in the permitted scope.",
    ),
    (
        "run_command",
        "runCommand",
        "Run an SSH command on a fresh exec connection with timeout and output limits.",
    ),
    (
        "open_session",
        "openSession",
        "Open a saved SSH profile session.",
    ),
    ("close_session", "openSession", "Close a session."),
    (
        "open_terminal",
        "openSession",
        "Ask desktop UI to open or focus a visible terminal tab.",
    ),
    (
        "upsert_profile",
        "manageProfiles",
        "Create or update a saved profile.",
    ),
    (
        "delete_profile",
        "manageProfiles",
        "Delete a saved profile.",
    ),
    ("tunnel_list", "tunnels", "List SSH forwarding tunnels."),
    (
        "tunnel_open",
        "tunnels",
        "Open forwarding using a saved SSH profile.",
    ),
    ("tunnel_close", "tunnels", "Close a forwarding tunnel."),
    (
        "remote_open",
        "remoteDesktop",
        "Launch a saved RDP/VNC profile.",
    ),
    (
        "call_rpc",
        "callRpc",
        "Call an explicit allowlisted RPC; its category permission is also required.",
    ),
];

pub fn permission(name: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .chain(super::context::TOOLS)
        .chain(super::batch::TOOLS)
        .find(|t| t.0 == name)
        .map(|t| t.1)
}
/// Deliberately exact, not prefix-based. Raw file/terminal write/configuration RPCs are forbidden.
pub fn rpc_permission(method: &str) -> Option<&'static str> {
    Some(match method {
        "core.version" | "core.protocolVersion" | "profile.list" | "profile.get" => "listProfiles",
        "session.list" | "terminal.exportText" | "terminal.search" => "readTerminal",
        "session.openSshProfile" | "session.close" => "openSession",
        "profile.upsert" | "profile.delete" => "manageProfiles",
        "tunnel.list" | "tunnel.open" | "tunnel.close" => "tunnels",
        "remote.openProfile" => "remoteDesktop",
        _ => return None,
    })
}

pub fn definitions() -> Vec<Value> {
    TOOLS.iter().chain(super::context::TOOLS).chain(super::batch::TOOLS).map(|(name,_,description)| {
let (fields,required):(&[&str],&[&str])=match *name {
"get_profile"|"close_session"|"close_sftp"|"delete_profile"|"tunnel_close"=>(&["id"],&["id"]),
"host_stats"|"open_session"|"open_sftp"|"remote_open"=>(&["profile_id"],&["profile_id"]),
"terminal_export"=>(&["id","max_bytes"],&["id"]),
"terminal_search"=>(&["id","query","regex","caseSensitive","direction"],&["id","query"]),
"list_files"|"get_file_info"=>(&["scope","sftp_id","path","max_entries"],&["path"]),
"read_file"=>(&["scope","sftp_id","path","offset","max_bytes"],&["path"]),
"find_file"=>(&["scope","sftp_id","path","query","max_entries","max_depth"],&["path","query"]),
"write_file"|"create_file"=>(&["scope","sftp_id","path","data"],&["path","data"]),
"create_directory"|"remove_path"=>(&["scope","sftp_id","path","directory"],&["path"]),
"rename_path"=>(&["scope","sftp_id","path","to"],&["path","to"]),
"run_command"=>(&["profile_id","command","timeout_ms","max_bytes"],&["profile_id","command"]),
"open_terminal"=>(&["profile_id"],&["profile_id"]),
"upsert_profile"=>(&["profile"],&["profile"]),
"tunnel_open"=>(&["profile_id","kind","bind_host","bind_port","target_host","target_port"],&["profile_id","kind","bind_host","bind_port","target_host","target_port"]),
"search_notes"=>(&["query","profile_id"],&[]),
"read_note"|"workspace_snapshot"|"batch_execute"|"batch_status"|"batch_results"|"batch_cancel"=>(&["id"],&["id"]),
"write_note"=>(&["note","expectedRevision","expectedVersion"],&["note","expectedRevision","expectedVersion"]),
"batch_plan"=>(&["script_id","profile_ids","concurrency"],&["script_id","profile_ids"]),
"call_rpc"=>(&["method","params"],&["method"]), _=>(&[],&[]) };
let mut properties=serde_json::Map::new();
for field in fields {
let schema=match *field {
"max_bytes"=>json!({"type":"integer","minimum":1,"maximum":MAX_BYTES}),
"max_entries"=>json!({"type":"integer","minimum":1,"maximum":1000}),
"max_depth"=>json!({"type":"integer","minimum":1,"maximum":8}),
"timeout_ms"=>json!({"type":"integer","minimum":1,"maximum":120000}),
"offset"=>json!({"type":"integer","minimum":0,"maximum":9007199254740991u64}),
"bind_port"|"target_port"=>json!({"type":"integer","minimum":0,"maximum":65535}),
"regex"|"caseSensitive"|"directory"=>json!({"type":"boolean"}),
"profile"|"params"|"note"=>json!({"type":"object"}),
"expectedRevision"|"expectedVersion"=>json!({"type":"integer","minimum":0}),
"concurrency"=>json!({"type":"integer","minimum":1,"maximum":4}),
"profile_ids"=>json!({"type":"array","items":{"type":"string"},"minItems":1,"maxItems":32}),
"scope"=>json!({"type":"string","enum":["local","sftp"]}),
"direction"=>json!({"type":"string","enum":["next","prev"]}),
_=>json!({"type":"string","maxLength":if *field=="data" {1398104} else {16384}}) };
properties.insert((*field).into(),schema);
}
json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}).collect()
}

pub(super) fn redact(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(redact).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                    let sensitive = normalized == "auth"
                        || normalized.contains("password")
                        || normalized.contains("passphrase")
                        || normalized.contains("privatekey")
                        || normalized.contains("token")
                        || normalized == "keypath"
                        || normalized == "entryid"
                        || normalized == "credential";
                    (
                        key,
                        if sensitive {
                            json!("[redacted]")
                        } else {
                            redact(value)
                        },
                    )
                })
                .collect(),
        ),
        other => other,
    }
}
fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && !v.contains('\0') && v.len() <= 16384)
        .ok_or_else(|| format!("{key} must be nonempty, at most 16384 bytes, without NUL"))
}
fn limit(args: &Value, key: &str, default: u64, maximum: u64) -> Result<u64, String> {
    match args.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| *n > 0 && *n <= maximum)
            .ok_or_else(|| format!("{key} must be between 1 and {maximum}")),
    }
}
pub(super) fn validate_args(name: &str, args: &Value) -> Result<(), String> {
    let object = args.as_object().ok_or("arguments must be an object")?;
    let definition = definitions()
        .into_iter()
        .find(|v| v["name"] == name)
        .ok_or("unknown tool")?;
    let schema = &definition["inputSchema"];
    for key in schema["required"].as_array().expect("required") {
        if !object.contains_key(key.as_str().expect("key")) {
            return Err(format!("missing {key}"));
        }
    }
    for (key, value) in object {
        let field = &schema["properties"][key];
        let valid = match field["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|v| {
                !v.contains('\0') && v.len() as u64 <= field["maxLength"].as_u64().unwrap_or(16384)
            }),
            Some("integer") => value.as_u64().is_some_and(|v| {
                v >= field["minimum"].as_u64().unwrap_or(0)
                    && v <= field["maximum"].as_u64().unwrap_or(u64::MAX)
            }),
            Some("boolean") => value.is_boolean(),
            Some("object") => value.is_object(),
            Some("array") => value.as_array().is_some_and(|a| {
                !a.is_empty()
                    && a.len() <= 32
                    && a.iter()
                        .all(|v| v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 128))
            }),
            _ => false,
        };
        if !valid
            || field
                .get("enum")
                .is_some_and(|e| !e.as_array().expect("enum").contains(value))
        {
            return Err(format!("invalid argument {key}"));
        }
    }
    Ok(())
}
pub(super) async fn dispatch(
    dispatcher: &Dispatcher,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response = dispatcher
        .dispatch(Request {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        })
        .await;
    if let Some(error) = response.error {
        // Backend parse/auth errors may embed credential-bearing payload fragments.
        return Err(format!("RPC {method} failed (code {})", error.code));
    }
    Ok(response.result.unwrap_or(Value::Null))
}
async fn saved_ssh(state: &AppState, id: &str) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| "invalid profile_id")?;
    let store = state
        .profiles
        .lock()
        .await
        .clone()
        .ok_or("profiles not configured")?;
    let profile = store
        .get(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("profile not found")?;
    match profile.spec {
        crate::profile::ProfileKind::Ssh { ssh } => {
            serde_json::to_value(ssh).map_err(|e| e.to_string())
        }
        _ => Err("profile is not SSH".into()),
    }
}

pub async fn call(
    name: &str,
    args: Value,
    dispatcher: Arc<Dispatcher>,
    state: Arc<AppState>,
) -> Result<Value, String> {
    validate_args(name, &args)?;
    let ms = limit(&args, "timeout_ms", 30000, 120000)?;
    tokio::time::timeout(
        Duration::from_millis(ms),
        call_inner(name, args, &dispatcher, &state),
    )
    .await
    .map_err(|_| "tool timed out; mutations may already have occurred".to_string())?
}
async fn call_inner(
    name: &str,
    args: Value,
    dispatcher: &Dispatcher,
    state: &AppState,
) -> Result<Value, String> {
    match name {
        "help" => Ok(
            json!({"tools":definitions(),"max_file_bytes":MAX_BYTES,"max_command_timeout_ms":120000,"local_scope":"absolute paths under settings.mcp.localRoots; symlinks rejected","remote_scope":"sftp_id from open_sftp or application SFTP dock","call_rpc":"requires callRpc and rpc_permission(method)"}),
        ),
        "list_profiles" => Ok(redact(
            dispatch(dispatcher, "profile.list", json!({})).await?,
        )),
        "get_profile" => Ok(redact(dispatch(dispatcher, "profile.get", args).await?)),
        "list_sessions" => dispatch(dispatcher, "session.list", args).await,
        "terminal_export" => {
            let max = limit(&args, "max_bytes", 65536, MAX_BYTES)? as usize;
            let mut result = dispatch(
                dispatcher,
                "terminal.exportText",
                json!({"id":text(&args,"id")?}),
            )
            .await?;
            if let Some(content) = result.get_mut("text") {
                let s = content.as_str().unwrap_or_default();
                let mut end = max.min(s.len());
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                let truncated = end < s.len();
                *content = json!(&s[..end]);
                result["truncated"] = json!(truncated);
            }
            Ok(result)
        }
        "terminal_search" => {
            if text(&args, "query")?.len() > 1024 {
                return Err("query exceeds 1024 bytes".into());
            }
            dispatch(dispatcher, "terminal.search", args).await
        }
        "host_stats" => {
            let profile = saved_ssh(state, text(&args, "profile_id")?).await?;
            dispatch(dispatcher, "ssh.hostStats", json!({"profile":profile})).await
        }
        "run_command" => {
            let id = uuid::Uuid::parse_str(text(&args, "profile_id")?)
                .map_err(|_| "invalid profile_id")?;
            crate::commands::mcp_run_command(
                state,
                id,
                text(&args, "command")?,
                limit(&args, "timeout_ms", 30000, 120000)?,
                limit(&args, "max_bytes", 65536, MAX_BYTES)? as usize,
            )
            .await
        }
        "open_sftp" => {
            let profile = saved_ssh(state, text(&args, "profile_id")?).await?;
            let opened = dispatch(dispatcher, "sftp.open", json!({"profile":profile})).await?;
            Ok(json!({"sftp_id":opened["id"]}))
        }
        "close_sftp" => dispatch(dispatcher, "sftp.close", args).await,
        "open_session" => dispatch(dispatcher, "session.openSshProfile", args).await,
        "close_session" => dispatch(dispatcher, "session.close", args).await,
        "open_terminal" => {
            let profile_id = text(&args, "profile_id")?;
            uuid::Uuid::parse_str(profile_id).map_err(|_| "invalid profile_id")?;
            crate::commands::mcp_open_terminal(profile_id)?;
            Ok(json!({"requested":true}))
        }
        "upsert_profile" => Ok(redact(
            dispatch(dispatcher, "profile.upsert", args["profile"].clone()).await?,
        )),
        "delete_profile" => dispatch(dispatcher, "profile.delete", args).await,
        "tunnel_list" => dispatch(dispatcher, "tunnel.list", args).await,
        "tunnel_close" => dispatch(dispatcher, "tunnel.close", args).await,
        "tunnel_open" => {
            let profile = saved_ssh(state, text(&args, "profile_id")?).await?;
            let mut params = args;
            params.as_object_mut().expect("args").remove("profile_id");
            params["profile"] = profile;
            dispatch(dispatcher, "tunnel.open", params).await
        }
        "remote_open" => dispatch(dispatcher, "remote.openProfile", args).await,
        "call_rpc" => {
            let method = text(&args, "method")?;
            rpc_permission(method).ok_or("RPC method is not allowlisted")?;
            let params = args.get("params").cloned().unwrap_or(json!({}));
            let tool = match method {
                "profile.list" => "list_profiles",
                "profile.get" => "get_profile",
                "session.list" => "list_sessions",
                "terminal.exportText" => "terminal_export",
                "terminal.search" => "terminal_search",
                "session.openSshProfile" => "open_session",
                "session.close" => "close_session",
                "profile.delete" => "delete_profile",
                "tunnel.list" => "tunnel_list",
                "tunnel.close" => "tunnel_close",
                "remote.openProfile" => "remote_open",
                "tunnel.open" => "tunnel_open",
                "profile.upsert" => {
                    return Ok(redact(dispatch(dispatcher, method, params).await?));
                }
                "core.version" | "core.protocolVersion" => {
                    return dispatch(dispatcher, method, params).await
                }
                _ => return Err("RPC method is not allowlisted".into()),
            };
            validate_args(tool, &params)?;
            Box::pin(call_inner(tool, params, dispatcher, state)).await
        }
        "list_files" | "read_file" | "get_file_info" | "find_file" | "write_file"
        | "create_file" | "create_directory" | "remove_path" | "rename_path" => {
            file_tool(name, args, dispatcher, state).await
        }
        name if super::context::TOOLS.iter().any(|t| t.0 == name) => {
            super::context::call(name, args, dispatcher, state).await
        }
        _ => Err("unknown tool".into()),
    }
}

async fn local_path(state: &AppState, raw: &str, new: bool) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("local path must be absolute without dot components".into());
    }
    let store = state
        .settings
        .lock()
        .await
        .clone()
        .ok_or("settings not configured")?;
    let config = store
        .get("mcp")
        .map_err(|e| e.to_string())?
        .ok_or("localRoots not configured")?;
    let roots = config
        .get("localRoots")
        .and_then(Value::as_array)
        .ok_or("localRoots not configured")?;
    let existing = if new {
        path.parent().ok_or("no parent")?
    } else {
        path
    };
    let canonical = tokio::fs::canonicalize(existing)
        .await
        .map_err(|e| e.to_string())?;
    let mut cursor = PathBuf::new();
    for part in path.components() {
        cursor.push(part);
        match tokio::fs::symlink_metadata(&cursor).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("local symlinks are not allowed".into())
            }
            Ok(meta) if !meta.is_dir() && !meta.is_file() => {
                return Err("only regular local files and directories are allowed".into());
            }
            Ok(_) => {}
            Err(e) if new && cursor == path && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    for root in roots.iter().filter_map(Value::as_str) {
        if !Path::new(root).is_absolute() {
            continue;
        }
        if let Ok(root) = tokio::fs::canonicalize(root).await {
            if canonical.starts_with(root) {
                return Ok(path.to_path_buf());
            }
        }
    }
    Err("path outside trusted settings.mcp.localRoots".into())
}

async fn file_tool(
    name: &str,
    args: Value,
    dispatcher: &Dispatcher,
    state: &AppState,
) -> Result<Value, String> {
    let raw = text(&args, "path")?;
    let max = limit(&args, "max_bytes", 65536, MAX_BYTES)?;
    let count = limit(&args, "max_entries", 200, 1000)? as usize;
    let remote = args["scope"] == "sftp" || args.get("sftp_id").is_some();
    let data = if matches!(name, "write_file" | "create_file") {
        let encoded = args["data"].as_str().ok_or("data must be base64")?;
        if encoded.len() > 1398104 {
            return Err("file exceeds 1 MiB".into());
        }
        let bytes = BASE64.decode(encoded).map_err(|_| "invalid base64")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("file exceeds 1 MiB".into());
        }
        bytes
    } else {
        Vec::new()
    };
    if remote {
        if args["scope"] == "local" {
            return Err("local scope cannot include remote ids".into());
        }
        // Existing ids are deliberately required: opening hidden SFTP sessions on a
        // timeout would leak handles and bypass explicit session ownership.
        let id = if let Some(id) = args.get("sftp_id") {
            id.clone()
        } else {
            return Err("provide an existing sftp_id (use the application's SFTP dock)".into());
        };
        if name == "create_file" {
            let session_id = uuid::Uuid::parse_str(id.as_str().ok_or("invalid sftp_id")?)
                .map_err(|_| "invalid sftp_id")?;
            let session = state
                .sftp_sessions
                .lock()
                .await
                .get(&session_id)
                .cloned()
                .ok_or("SFTP session not found")?;
            session
                .create_file_exclusive(raw, &data)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(json!({"created":true}));
        }
        return remote_file(name, &args, &id, dispatcher, &data).await;
    }
    let new = matches!(name, "write_file" | "create_file" | "create_directory");
    let path = local_path(state, raw, new).await?;
    if matches!(name, "remove_path" | "rename_path") {
        let store = state
            .settings
            .lock()
            .await
            .clone()
            .ok_or("settings not configured")?;
        let config = store
            .get("mcp")
            .map_err(|e| e.to_string())?
            .unwrap_or(Value::Null);
        let canonical = tokio::fs::canonicalize(&path)
            .await
            .map_err(|e| e.to_string())?;
        for root in config["localRoots"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if tokio::fs::canonicalize(root).await.ok().as_ref() == Some(&canonical) {
                return Err("cannot mutate configured root".into());
            }
        }
    }
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
    match name {
        "get_file_info" => {
            let m = tokio::fs::symlink_metadata(path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"size":m.len(),"directory":m.is_dir(),"symlink":m.file_type().is_symlink()}))
        }
        "read_file" => {
            let mut f = tokio::fs::File::open(path)
                .await
                .map_err(|e| e.to_string())?;
            if !f.metadata().await.map_err(|e| e.to_string())?.is_file() {
                return Err("not a regular file".into());
            }
            f.seek(std::io::SeekFrom::Start(
                args["offset"].as_u64().unwrap_or(0),
            ))
            .await
            .map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            f.take(max)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"data":BASE64.encode(&bytes),"bytes":bytes.len()}))
        }
        "write_file" | "create_file" => {
            let mut options = tokio::fs::OpenOptions::new();
            options.write(true);
            if name == "create_file" {
                options.create_new(true);
            } else {
                options.create(true).truncate(true);
            }
            let mut file = options.open(path).await.map_err(|e| e.to_string())?;
            if !file.metadata().await.map_err(|e| e.to_string())?.is_file() {
                return Err("not a regular file".into());
            }
            file.write_all(&data).await.map_err(|e| e.to_string())?;
            Ok(json!({"bytes":data.len()}))
        }
        "create_directory" => {
            tokio::fs::create_dir(path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "remove_path" => {
            if args["directory"] == true {
                tokio::fs::remove_dir(path).await
            } else {
                tokio::fs::remove_file(path).await
            }
            .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "rename_path" => {
            let to = local_path(state, text(&args, "to")?, true).await?;
            tokio::fs::rename(path, to)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "list_files" | "find_file" => {
            let depth_limit = limit(&args, "max_depth", 3, 8)?;
            let query = if name == "find_file" {
                Some(text(&args, "query")?)
            } else {
                None
            };
            let mut pending = vec![(path, 0)];
            let mut entries = Vec::new();
            let mut visited = 0;
            let mut truncated = false;
            'walk: while let Some((dir, depth)) = pending.pop() {
                local_path(state, dir.to_str().ok_or("non-UTF8 path")?, false).await?;
                let mut reader = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
                while let Some(entry) = reader.next_entry().await.map_err(|e| e.to_string())? {
                    visited += 1;
                    if visited > 10000 || entries.len() >= count {
                        truncated = true;
                        break 'walk;
                    }
                    let kind = entry.file_type().await.map_err(|e| e.to_string())?;
                    let filename = entry.file_name().to_string_lossy().into_owned();
                    if query.map_or(true, |q| filename.contains(q)) {
                        entries.push(json!({"name":filename,"path":entry.path(),"directory":kind.is_dir(),"symlink":kind.is_symlink()}));
                    }
                    if query.is_some() && kind.is_dir() && !kind.is_symlink() && depth < depth_limit
                    {
                        pending.push((entry.path(), depth + 1));
                    }
                }
            }
            Ok(json!({"entries":entries,"truncated":truncated}))
        }
        _ => Err("unknown file tool".into()),
    }
}

async fn remote_file(
    name: &str,
    args: &Value,
    id: &Value,
    dispatcher: &Dispatcher,
    data: &[u8],
) -> Result<Value, String> {
    let max = limit(args, "max_bytes", 65536, MAX_BYTES)?;
    let count = limit(args, "max_entries", 200, 1000)? as usize;
    let path = text(args, "path")?;
    let params = json!({"id":id,"path":path});
    match name {
        "get_file_info" => dispatch(dispatcher, "sftp.stat", params).await,
        "read_file" => dispatch(
            dispatcher,
            "sftp.readChunk",
            json!({"id":id,"path":path,"offset":args["offset"].as_u64().unwrap_or(0),"len":max}),
        )
        .await,
        "write_file" => {
            dispatch(
                dispatcher,
                "sftp.write",
                json!({"id":id,"path":path,"data":BASE64.encode(data)}),
            )
            .await
        }
        "create_file" => {
            Err("remote exclusive creation unsupported; use write_file explicitly".into())
        }
        "create_directory" => dispatch(dispatcher, "sftp.mkdir", params).await,
        "remove_path" => {
            dispatch(
                dispatcher,
                if args["directory"] == true {
                    "sftp.removeDir"
                } else {
                    "sftp.removeFile"
                },
                params,
            )
            .await
        }
        "rename_path" => {
            dispatch(
                dispatcher,
                "sftp.rename",
                json!({"id":id,"from":path,"to":text(args,"to")?}),
            )
            .await
        }
        "list_files" | "find_file" => {
            let query = if name == "find_file" {
                Some(text(args, "query")?)
            } else {
                None
            };
            let depth_limit = limit(args, "max_depth", 3, 8)?;
            let mut pending = vec![(path.to_owned(), 0)];
            let mut entries = Vec::new();
            let mut visited = 0;
            let mut truncated = false;
            'walk: while let Some((dir, depth)) = pending.pop() {
                let list = dispatch(dispatcher, "sftp.list", json!({"id":id,"path":dir})).await?;
                for entry in list.as_array().ok_or("invalid SFTP list")? {
                    visited += 1;
                    if visited > 10000 || entries.len() >= count {
                        truncated = true;
                        break 'walk;
                    }
                    let filename = entry["name"].as_str().ok_or("invalid SFTP entry")?;
                    if filename == "."
                        || filename == ".."
                        || filename.contains('/')
                        || filename.contains('\\')
                    {
                        continue;
                    }
                    let full = format!("{}/{filename}", dir.trim_end_matches('/'));
                    if query.map_or(true, |q| filename.contains(q)) {
                        let mut e = entry.clone();
                        e["path"] = json!(full);
                        entries.push(e);
                    }
                    if query.is_some() && entry["kind"] == "Dir" && depth < depth_limit {
                        pending.push((full, depth + 1));
                    }
                }
            }
            Ok(json!({"entries":entries,"truncated":truncated,"sftp_id":id}))
        }
        _ => Err("unknown file tool".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn rpc_cannot_bypass_export_limits_or_profile_redaction() {
        let dispatcher = Arc::new(Dispatcher::new());
        dispatcher.register("terminal.exportText", |_| async {
            Ok(json!({"text":"中文hello"}))
        });
        dispatcher.register("profile.get", |_| async {
            Ok(json!({"auth":{"password":"hidden"},"name":"safe"}))
        });
        let state = AppState::new();
        let result = call(
            "call_rpc",
            json!({"method":"terminal.exportText","params":{"id":"x","max_bytes":4}}),
            dispatcher.clone(),
            state.clone(),
        )
        .await
        .unwrap();
        assert_eq!(result["text"], "中");
        assert_eq!(result["truncated"], true);
        let result = call(
            "call_rpc",
            json!({"method":"profile.get","params":{"id":"x"}}),
            dispatcher.clone(),
            state.clone(),
        )
        .await
        .unwrap();
        assert!(!result.to_string().contains("hidden"));
        assert!(call(
            "call_rpc",
            json!({"method":"terminal.exportText","params":{"id":"x","max_bytes":MAX_BYTES+1}}),
            dispatcher,
            state
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn local_access_requires_trusted_roots_and_rejects_escape() {
        let dir = std::env::temp_dir().join(format!("aerotab-mcp-tools-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let state = AppState::new();
        assert!(local_path(&state, dir.to_str().unwrap(), false)
            .await
            .is_err());
        let store = crate::settings::SettingsStore::open(dir.join("settings")).unwrap();
        store.set("mcp", &json!({"localRoots":[dir]})).unwrap();
        *state.settings.lock().await = Some(store);
        assert!(local_path(&state, dir.to_str().unwrap(), false)
            .await
            .is_ok());
        assert!(local_path(&state, "/etc/passwd", false).await.is_err());
        assert!(
            local_path(&state, dir.join("../escape").to_str().unwrap(), true)
                .await
                .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
            assert!(
                local_path(&state, dir.join("link/passwd").to_str().unwrap(), false)
                    .await
                    .is_err()
            );
        }
        *state.settings.lock().await = None;
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn every_advertised_tool_has_permission_and_schema() {
        let tools = definitions();
        assert_eq!(tools.len(), 39);
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

    #[tokio::test]
    async fn refuses_invalid_arguments_before_dispatch() {
        let dispatcher = Arc::new(Dispatcher::new());
        dispatcher.register("session.openLocal", |_| async {
            panic!("unsafe method reached")
        });
        let state = AppState::new();
        for (name, args) in [
            (
                "run_command",
                json!({"profile_id":"x","command":"echo ok","timeout_ms":120001}),
            ),
            (
                "run_command",
                json!({"profile_id":"x","command":"echo\u{0}ok"}),
            ),
            (
                "read_file",
                json!({"path":"/etc/passwd","scope":"local","max_bytes":MAX_BYTES+1}),
            ),
            (
                "call_rpc",
                json!({"method":"session.openLocal","params":{}}),
            ),
            ("help", json!({"unknown":true})),
        ] {
            assert!(
                call(name, args, dispatcher.clone(), state.clone())
                    .await
                    .is_err(),
                "accepted {name}"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_or_wrong_type_limits() {
        assert!(limit(&json!({"max_bytes":0}), "max_bytes", 10, 100).is_err());
        assert!(limit(&json!({"max_bytes":101}), "max_bytes", 10, 100).is_err());
        assert!(limit(&json!({"max_bytes":"12"}), "max_bytes", 10, 100).is_err());
        assert_eq!(limit(&json!({}), "max_bytes", 10, 100).unwrap(), 10);
    }
}
