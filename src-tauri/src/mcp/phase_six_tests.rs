use super::*;

#[test]
fn batch_execution_also_requires_command_grant() {
    let mut config = McpConfig::default();
    config.permissions.insert("batchOperations".into(), true);
    assert!(permissions::authorize(&config, "batch_plan", &json!({})).is_ok());
    assert!(permissions::authorize(&config, "batch_execute", &json!({})).is_err());
    config.permissions.insert("runCommand".into(), true);
    assert!(permissions::authorize(&config, "batch_execute", &json!({})).is_ok());
}

#[tokio::test]
async fn notes_reads_and_writes_fail_closed() {
    let server = McpServer::new(
        AppState::new(),
        Arc::new(Dispatcher::new()),
        McpConfig::default(),
    );
    for name in [
        "search_notes",
        "read_note",
        "write_note",
        "list_workspaces",
        "batch_execute",
    ] {
        let result = server.invoke(name, json!({})).await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("Permission denied"));
    }
}

#[tokio::test]
async fn batch_requires_local_approval_rejects_changed_targets_and_cancelled() {
    let state = AppState::new();
    let d = Arc::new(Dispatcher::new());
    let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = changed.clone();
    d.register("profile.list",move |_|{let flag=flag.clone();async move {Ok(json!([{"id":"p","host":if flag.load(std::sync::atomic::Ordering::SeqCst){"changed"}else{"original"}}]))}});
    d.register("scripts.get", |_| async {
        Ok(json!({"id":"s","name":"S","command":"echo safe","version":1}))
    });
    d.register("scripts.preview", |_| async {
        Ok(json!({"target":"test"}))
    });
    d.register("scripts.prepare", |_| async {
        Ok(json!({"token":"internal","target":"user@host:22"}))
    });
    batch::register_rpc(&d, state.clone());
    let mut config = McpConfig::default();
    config.permissions.insert("batchOperations".into(), true);
    config.permissions.insert("runCommand".into(), true);
    let server = McpServer::new(state, d.clone(), config);
    let plan = server
        .invoke("batch_plan", json!({"script_id":"s","profile_ids":["p"]}))
        .await
        .structured_content
        .unwrap();
    assert!(plan.get("command").is_none());
    assert!(!plan.to_string().contains("internal"));
    let id = plan["id"].clone();
    assert_eq!(
        server
            .invoke("batch_execute", json!({"id":id}))
            .await
            .is_error,
        Some(true)
    );
    assert_eq!(
        server
            .invoke(
                "call_rpc",
                json!({"method":"mcp.batchApprove","params":{"id":id,"approved":true}})
            )
            .await
            .is_error,
        Some(true)
    );
    let approved = tools::dispatch(
        &d,
        "mcp.batchApprove",
        json!({"id":id,"digest":plan["digest"],"approved":true}),
    )
    .await
    .unwrap();
    assert_eq!(approved["status"], "approved");
    changed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        server
            .invoke("batch_execute", json!({"id":id}))
            .await
            .is_error,
        Some(true)
    );
    let status = server
        .invoke("batch_status", json!({"id":id}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(status["status"], "invalidated");
    let plan = server
        .invoke("batch_plan", json!({"script_id":"s","profile_ids":["p"]}))
        .await
        .structured_content
        .unwrap();
    let id = plan["id"].clone();
    let cancelled = server
        .invoke("batch_cancel", json!({"id":id}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert!(tools::dispatch(
        &d,
        "mcp.batchApprove",
        json!({"id":id,"digest":plan["digest"],"approved":true})
    )
    .await
    .is_err());
    assert_eq!(
        server
            .invoke("batch_execute", json!({"id":id}))
            .await
            .is_error,
        Some(true)
    );
}

#[test]
fn workspace_snapshot_never_exports_restore_secrets() {
    let input = json!({"id":"w","name":"Work","tabs":[{"title":"T","panes":[{"kind":"ssh","profile":{"auth":{"password":"SECRET"}}},{"kind":"shell","command":"SECRET"},{"kind":"ssh-profile","id":"p"}],"sftpDock":{"password":"SECRET"},"layout":{"type":"leaf","paneIndex":0}}]});
    let output = super::context::workspace_snapshot(&input);
    assert!(!output.to_string().contains("SECRET"));
    assert_eq!(output["tabs"][0]["panes"][2]["id"], "p");
}

#[tokio::test]
async fn batch_execute_limits_concurrency_cancels_and_sanitizes_results() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let state = AppState::new();
    let d = Arc::new(Dispatcher::new());
    d.register("profile.list", |_| async { Ok(json!([{"id":"p"}])) });
    d.register("scripts.get", |_| async {
        Ok(json!({"id":"s","name":"S","command":"echo test","version":1}))
    });
    d.register("scripts.preview", |_| async {
        Ok(json!({"target":"test"}))
    });
    d.register("scripts.prepare", |params| async move {
        Ok(json!({"token":params["profile_id"],"target":"test"}))
    });
    let started = Arc::new(AtomicUsize::new(0));
    let count = started.clone();
    d.register("scripts.run", move |params| {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"id":params["token"],"status":"running"}))
        }
    });
    d.register("scripts.status", |params| async move {
        Ok(json!({"id":params["id"],"status":"running"}))
    });
    d.register("scripts.cancel",|params|async move{Ok(json!({"id":params["id"],"status":"cancelled","stdout":"password=SECRET\nOK","stderr":"Bearer HIDDEN"}))});
    batch::register_rpc(&d, state.clone());
    let mut config = McpConfig::default();
    config.permissions.insert("batchOperations".into(), true);
    config.permissions.insert("runCommand".into(), true);
    let server = McpServer::new(state, d.clone(), config);
    let plan = server
        .invoke(
            "batch_plan",
            json!({"script_id":"s","profile_ids":["p1","p2","p3","p4"],"concurrency":2}),
        )
        .await
        .structured_content
        .unwrap();
    tools::dispatch(
        &d,
        "mcp.batchApprove",
        json!({"id":plan["id"],"digest":plan["digest"],"approved":true}),
    )
    .await
    .unwrap();
    assert_ne!(
        server
            .invoke("batch_execute", json!({"id":plan["id"]}))
            .await
            .is_error,
        Some(true)
    );
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while started.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 2);
    server
        .invoke("batch_cancel", json!({"id":plan["id"]}))
        .await;
    let results = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let result = server
                .invoke("batch_results", json!({"id":plan["id"]}))
                .await
                .structured_content
                .unwrap();
            if result["status"] == "cancelled" {
                break result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(started.load(Ordering::SeqCst), 2);
    assert_eq!(results["results"].as_array().unwrap().len(), 2);
    assert!(!results.to_string().contains("SECRET"));
    assert!(!results.to_string().contains("HIDDEN"));
}

#[tokio::test]
async fn batch_generation_stop_cancels_active_and_queued() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for reason in ["rotate", "stop", "revoke", "disable"] {
        let state = AppState::new();
        let d = Arc::new(Dispatcher::new());
        d.register("profile.list", |_| async { Ok(json!([])) });
        d.register("scripts.get", |_| async {
            Ok(json!({"id":"s","command":"safe"}))
        });
        d.register("scripts.preview", |_| async {
            Ok(json!({"target":"test"}))
        });
        d.register("scripts.prepare", |p| async move {
            Ok(json!({"token":p["profile_id"]}))
        });
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = starts.clone();
        d.register("scripts.run", move |p| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"id":p["token"]}))
            }
        });
        let cancels = Arc::new(AtomicUsize::new(0));
        let counter = cancels.clone();
        d.register("scripts.cancel", move |_| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(json!({"status":"cancelled"}))
            }
        });
        d.register("scripts.status", |_| async {
            Ok(json!({"status":"running"}))
        });
        batch::register_rpc(&d, state.clone());
        let mut cfg = McpConfig::default();
        cfg.permissions.insert("batchOperations".into(), true);
        cfg.permissions.insert("runCommand".into(), true);
        cfg.enabled = true;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let settings = crate::settings::SettingsStore::open(&root).unwrap();
        settings.set("mcp", &json!(cfg)).unwrap();
        *state.settings.lock().await = Some(settings.clone());
        let server = McpServer::new(state.clone(), d.clone(), cfg.clone());
        let plan = server
            .invoke(
                "batch_plan",
                json!({"script_id":"s","profile_ids":["1","2","3"],"concurrency":1}),
            )
            .await
            .structured_content
            .unwrap();
        tools::dispatch(
            &d,
            "mcp.batchApprove",
            json!({"id":plan["id"],"digest":plan["digest"],"approved":true}),
        )
        .await
        .unwrap();
        server
            .invoke("batch_execute", json!({"id":plan["id"]}))
            .await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while starts.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        match reason {
            "rotate" => server.cancellation.cancel(),
            "stop" => server.shutdown.cancel(),
            _ => {
                if reason == "disable" {
                    cfg.enabled = false;
                } else {
                    cfg.permissions.insert("runCommand".into(), false);
                }
                settings.set("mcp", &json!(cfg)).unwrap();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(starts.load(Ordering::SeqCst), 1, "{reason}");
        assert_eq!(cancels.load(Ordering::SeqCst), 1, "{reason}");
    }
}

#[tokio::test]
async fn batch_long_queue_prepares_fresh_tokens_and_preserves_order() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let state = AppState::new();
    let d = Arc::new(Dispatcher::new());
    d.register("profile.list", |_| async { Ok(json!([])) });
    d.register("scripts.get", |_| async {
        Ok(json!({"id":"s","command":"safe"}))
    });
    d.register("scripts.preview", |_| async {
        Ok(json!({"target":"test"}))
    });
    let clock = Arc::new(AtomicU64::new(0));
    let tokens = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        u64,
    >::new()));
    let times = tokens.clone();
    let now = clock.clone();
    d.register("scripts.prepare", move |p| {
        let times = times.clone();
        let now = now.clone();
        async move {
            let id = p["profile_id"].as_str().unwrap().to_string();
            times
                .lock()
                .unwrap()
                .insert(id.clone(), now.load(Ordering::SeqCst));
            Ok(json!({"token":id}))
        }
    });
    let times = tokens.clone();
    let now = clock.clone();
    d.register("scripts.run", move |p| {
        let times = times.clone();
        let now = now.clone();
        async move {
            let id = p["token"].as_str().unwrap();
            assert!(
                now.load(Ordering::SeqCst) - times.lock().unwrap()[id] < 60,
                "expired per-target token"
            );
            Ok(json!({"id":id}))
        }
    });
    let now = clock.clone();
    d.register("scripts.status", move |_| {
        let now = now.clone();
        async move {
            now.fetch_add(30, Ordering::SeqCst);
            Ok(json!({"status":"completed"}))
        }
    });
    batch::register_rpc(&d, state.clone());
    let mut cfg = McpConfig::default();
    cfg.permissions.insert("batchOperations".into(), true);
    cfg.permissions.insert("runCommand".into(), true);
    let server = McpServer::new(state, d.clone(), cfg);
    let ids: Vec<_> = (0..32).map(|i| i.to_string()).collect();
    let plan = server
        .invoke(
            "batch_plan",
            json!({"script_id":"s","profile_ids":ids,"concurrency":1}),
        )
        .await
        .structured_content
        .unwrap();
    assert!(
        tokens.lock().unwrap().is_empty(),
        "planning must not create expiring script tokens"
    );
    let other = server
        .invoke(
            "batch_plan",
            json!({"script_id":"s","profile_ids":["other"]}),
        )
        .await
        .structured_content
        .unwrap();
    for _ in 0..5 {
        let pending = tools::dispatch(&d, "mcp.batchPending", json!({}))
            .await
            .unwrap();
        assert_eq!(pending[0]["id"], plan["id"]);
        assert_eq!(pending[1]["id"], other["id"]);
    }
    tools::dispatch(
        &d,
        "mcp.batchApprove",
        json!({"id":plan["id"],"digest":plan["digest"],"approved":true}),
    )
    .await
    .unwrap();
    server
        .invoke("batch_execute", json!({"id":plan["id"]}))
        .await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let result = server
                .invoke("batch_results", json!({"id":plan["id"]}))
                .await
                .structured_content
                .unwrap();
            if result["status"] == "completed" {
                break result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 32);
    assert!(result["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["status"] == "completed"));
    assert_eq!(clock.load(Ordering::SeqCst), 960);
}

#[test]
fn batch_multiline_credentials_are_redacted() {
    let value = super::batch::public_result(
        json!({"stdout":"before\n-----BEGIN OPENSSH PRIVATE KEY-----\nSENSITIVEBASE64\n-----END OPENSSH PRIVATE KEY-----\nafter"}),
    );
    assert!(!value.to_string().contains("SENSITIVEBASE64"));
}

#[tokio::test]
async fn notes_real_store_search_profile_links_and_revision_conflicts() {
    let state = AppState::new();
    let db = sled::Config::new().temporary(true).open().unwrap();
    let store = crate::notes::NotesStore::from_db(&db).unwrap();
    *state.notes.lock().await = Some(store.clone());
    let d = Arc::new(Dispatcher::new());
    d.register("notes.put", move |params| {
        let store = store.clone();
        async move {
            crate::notes::dispatch(&store, "notes.put", params, &["p".into()]).map_err(|_| {
                crate::ipc::RpcError::new(crate::ipc::ErrorCode::InvalidParams, "Conflict")
            })
        }
    });
    let mut config = McpConfig::default();
    config.permissions.insert("readNotes".into(), true);
    let reader = McpServer::new(state.clone(), d.clone(), config.clone());
    let note = json!({"id":"n","title":"Maintenance","parent_id":null,"markdown":"Procedure","profile_ids":["p"]});
    let args = json!({"note":note,"expectedVersion":0,"expectedRevision":0});
    assert_eq!(
        reader.invoke("write_note", args.clone()).await.is_error,
        Some(true)
    );
    config.permissions.insert("writeNotes".into(), true);
    let writer = McpServer::new(state, d, config);
    assert_ne!(
        writer.invoke("write_note", args.clone()).await.is_error,
        Some(true)
    );
    assert_eq!(writer.invoke("write_note", args).await.is_error, Some(true));
    assert_eq!(
        writer
            .invoke("write_note", json!({"note":note,"expectedVersion":1}))
            .await
            .is_error,
        Some(true)
    );
    let found = reader
        .invoke(
            "search_notes",
            json!({"query":"procedure","profile_id":"p"}),
        )
        .await
        .structured_content
        .unwrap();
    assert_eq!(found["notes"].as_array().unwrap().len(), 1);
    let missing = reader
        .invoke("search_notes", json!({"profile_id":"other"}))
        .await
        .structured_content
        .unwrap();
    assert!(missing["notes"].as_array().unwrap().is_empty());
    let read = reader
        .invoke("read_note", json!({"id":"n"}))
        .await
        .structured_content
        .unwrap();
    assert_eq!(read["note"]["markdown"], "Procedure");
    assert_eq!(read["revision"], 1);
}

#[test]
fn batch_outputs_are_sanitized() {
    let value = super::batch::public_result(
        json!({"stdout":"password=SECRET\nBearer TOKEN\nOK","stderr":"private_key=KEY","token":"HIDDEN"}),
    );
    let text = value.to_string();
    for secret in ["SECRET", "TOKEN", "KEY", "HIDDEN"] {
        assert!(!text.contains(secret), "{text}");
    }
}
