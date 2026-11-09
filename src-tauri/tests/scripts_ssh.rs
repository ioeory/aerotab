// Exercises the real russh exec/channel lifecycle, not a UI-only cancellation flag.
use aerotab_core::{scripts, ssh};
use russh::server::Server;
use russh::{server, Channel, ChannelId};
use scripts::*;
use serde_json::json;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Clone)]
struct TestServer {
    active: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn auth_password(&mut self, _: &str, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.active.fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel);
        if data == b"bounded" {
            for _ in 0..10 {
                session.data(channel, russh::CryptoVec::from_slice(&vec![b'x'; 32768]));
            }
        } else if data == b"complete" {
            session.data(channel, russh::CryptoVec::from_slice(b"hello"));
            session.extended_data(channel, 1, russh::CryptoVec::from_slice(b"err"));
            session.exit_status_request(channel, 7);
            session.eof(channel);
            session.close(channel);
        }
        Ok(())
    }
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.active.fetch_add(100, Ordering::SeqCst);
    }
}
impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

#[tokio::test]
async fn real_exec_completion_limit_timeout_cancel_and_disconnect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = server::Config {
        auth_rejection_time: Duration::ZERO,
        ..Default::default()
    };
    config
        .keys
        .push(russh::keys::key::KeyPair::generate_ed25519().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let mut server = TestServer {
        active: active.clone(),
    };
    let server_task = tokio::spawn(async move {
        server
            .run_on_socket(Arc::new(config), &listener)
            .await
            .unwrap();
    });
    let path = std::env::temp_dir().join(format!("scripts-ssh-{}", uuid::Uuid::new_v4()));
    let kh = ssh::known_hosts::KnownHosts::open(&path).unwrap();
    let resolve = |_: String| {
        let kh = kh.clone();
        async move {
            Ok(ResolvedTarget {
                profile: ssh::SshProfile {
                    host: "127.0.0.1".into(),
                    port,
                    user: "u".into(),
                    auth: ssh::AuthMethod::Password {
                        secret: "test".into(),
                    },
                    jump_via: vec![],
                    jump_profile_ids: vec![],
                    proxy_id: None,
                    resolved_proxy: None,
                    inherit: vec![],
                },
                known_hosts: kh,
                transport: ssh::SshTransportSettings::default(),
            })
        }
    };
    let service = ScriptsService::new(ScriptsStore::temporary().unwrap());
    for (index, command, expected) in [
        (0, "complete", "completed"),
        (1, "bounded", "output_limit"),
        (2, "hang", "timed_out"),
        (3, "hang", "cancelled"),
    ] {
        let id = format!("s{index}");
        service
            .store
            .put(
                ScriptInput {
                    id: id.clone(),
                    name: id.clone(),
                    command: command.into(),
                    description: String::new(),
                    profile_id: Some("p".into()),
                },
                0,
            )
            .unwrap();
        let prepared = service
            .dispatch(
                "scripts.prepare",
                json!({"id":id,"profile_id":"p","timeout_seconds":1}),
                &resolve,
            )
            .await
            .unwrap();
        let task = service
            .dispatch("scripts.run", json!({"token":prepared["token"]}), &resolve)
            .await
            .unwrap();
        let task_id = task["id"].as_str().unwrap();
        if expected == "cancelled" {
            tokio::time::sleep(Duration::from_millis(100)).await;
            service
                .dispatch("scripts.cancel", json!({"id":task_id}), &resolve)
                .await
                .unwrap();
        }
        let state = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = service
                    .dispatch("scripts.status", json!({"id":task_id}), &resolve)
                    .await
                    .unwrap();
                if state["status"] != "running" {
                    break state;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state["status"], expected, "{state}");
        if expected == "completed" {
            assert_eq!(state["stdout"], "hello");
            assert_eq!(state["stderr"], "err");
            assert_eq!(state["exit_code"], 7);
        }
        if expected == "output_limit" {
            assert_eq!(state["stdout"].as_str().unwrap().len(), 256 * 1024);
        }
    }
    service.shutdown().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        active.load(Ordering::SeqCst) >= 400,
        "dedicated connections were not dropped"
    );
    server_task.abort();
    let _ = std::fs::remove_file(path);
}
