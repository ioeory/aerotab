use aerotab_core::{scripts, ssh};
use scripts::*;
use serde_json::json;

#[test]
fn persistence_cas_and_no_credential_metadata() {
    let path = std::env::temp_dir().join(format!("scripts-test-{}", uuid::Uuid::new_v4()));
    let store = ScriptsStore::open(&path).unwrap();
    let script: ScriptInput = serde_json::from_value(
        json!({"id":"s","name":"Name","command":"echo ok","description":"d","profile_id":"p"}),
    )
    .unwrap();
    let saved = store.put(script.clone(), 0).unwrap();
    assert_eq!(saved.version, 1);
    assert!(store.put(script, 0).is_err());
    assert_eq!(store.references("p").unwrap(), vec!["Name".to_string()]);
    assert!(serde_json::from_value::<ScriptInput>(
        json!({"id":"x","name":"x","command":"x","ssh":{"auth":"secret"}})
    )
    .is_err());
    drop(store);
    let store = ScriptsStore::open(&path).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
    assert!(store.delete("s", 0).is_err());
    store.delete("s", 1).unwrap();
    drop(store);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn confirmation_expiry_changes_and_cancellation_are_enforced() {
    let store = ScriptsStore::temporary().unwrap();
    store
        .put(
            ScriptInput {
                id: "s".into(),
                name: "N".into(),
                command: "echo ok".into(),
                description: String::new(),
                profile_id: Some("p".into()),
            },
            0,
        )
        .unwrap();
    let service = ScriptsService::new(store);
    let resolve = |_: String| async {
        Ok(ResolvedTarget {
            profile: ssh::SshProfile {
                host: "127.0.0.1".into(),
                port: 1,
                user: "u".into(),
                auth: ssh::AuthMethod::Agent,
                jump_via: vec![],
                jump_profile_ids: vec![],
                proxy_id: None,
                resolved_proxy: None,
                inherit: vec![],
            },
            known_hosts: ssh::known_hosts::KnownHosts::open(
                std::env::temp_dir().join(format!("scripts-kh-{}", uuid::Uuid::new_v4())),
            )
            .unwrap(),
            transport: ssh::SshTransportSettings::default(),
        })
    };
    assert!(service
        .dispatch("scripts.run", json!({"token":"forged"}), &resolve)
        .await
        .is_err());
    let prepared = service
        .dispatch(
            "scripts.prepare",
            json!({"id":"s","profile_id":"p"}),
            &resolve,
        )
        .await
        .unwrap();
    assert_eq!(prepared["target"], "u@127.0.0.1:1");
    let task = service
        .dispatch("scripts.run", json!({"token":prepared["token"]}), &resolve)
        .await
        .unwrap();
    assert!(service
        .dispatch("scripts.run", json!({"token":prepared["token"]}), &resolve)
        .await
        .is_err());
    let id = task["id"].as_str().unwrap();
    service
        .dispatch("scripts.cancel", json!({"id":id}), &resolve)
        .await
        .unwrap();
    let state = service
        .dispatch("scripts.status", json!({"id":id}), &resolve)
        .await
        .unwrap();
    assert_ne!(state["status"], "running");
}

#[tokio::test]
async fn output_budget_and_timeout_cancel_are_real() {
    let mut output = BoundedOutput::new(5);
    assert!(!output.push(false, b"abc"));
    assert!(output.push(true, b"defgh"));
    assert_eq!(output.stdout, b"abc");
    assert_eq!(output.stderr, b"de");
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = controlled(
        std::future::pending::<Result<(), String>>(),
        cancel.clone(),
        std::time::Duration::from_millis(5),
    )
    .await;
    assert_eq!(result, Err(StopReason::TimedOut));
    cancel.cancel();
    assert_eq!(
        controlled(
            std::future::pending::<Result<(), String>>(),
            cancel,
            std::time::Duration::from_secs(1)
        )
        .await,
        Err(StopReason::Cancelled)
    );
}
