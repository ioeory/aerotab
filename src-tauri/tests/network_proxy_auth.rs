pub use aerotab_core::{network_resources, profile, proxy_transport, ssh, vault};
use network_resources::*;
use uuid::Uuid;

#[tokio::test]
async fn vault_authentication_is_resolved_only_on_dial_and_lock_fails_closed() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = std::env::temp_dir().join(format!("proxy-auth-{}", Uuid::new_v4()));
    let vault = vault::VaultStore::open(&dir).unwrap();
    vault.initialize(b"test master password").await.unwrap();
    let entry = vault
        .put(vault::VaultEntry {
            id: String::new(),
            label: "proxy".into(),
            kind: vault::EntryKind::Password,
            username: Some("user".into()),
            secret: "pass".into(),
        })
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = ProxyResource {
        id: Uuid::new_v4(),
        name: "authenticated".into(),
        config: ProxyConfig::Socks5 {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            vault_entry_id: Some(entry.id),
        },
    };
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 2]);
        stream.write_all(&[5, 2]).await.unwrap();
        let mut auth = [0; 11];
        stream.read_exact(&mut auth).await.unwrap();
        assert_eq!(&auth, b"\x01\x04user\x04pass");
        stream.write_all(&[1, 0]).await.unwrap();
        let mut header = [0; 5];
        stream.read_exact(&mut header).await.unwrap();
        let mut rest = vec![0; header[4] as usize + 2];
        stream.read_exact(&mut rest).await.unwrap();
        stream
            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
    });
    proxy_transport::dial_proxy(&proxy, "target", 22, Some(&vault))
        .await
        .unwrap();
    server.await.unwrap();
    vault.lock().await;
    assert!(
        proxy_transport::dial_proxy(&proxy, "target", 22, Some(&vault))
            .await
            .is_err()
    );
    assert!(!serde_json::to_string(&proxy).unwrap().contains("pass"));
}

#[tokio::test]
async fn http_connect_rejection_does_not_leak_response_or_credentials() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = ProxyResource {
        id: Uuid::new_v4(),
        name: "reject".into(),
        config: ProxyConfig::HttpConnect {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            vault_entry_id: None,
        },
    };
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        while !req.ends_with(b"\r\n\r\n") {
            req.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-secret: do-not-log\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let error = match proxy_transport::dial_proxy(&proxy, "target", 22, None).await {
        Ok(_) => panic!("rejected proxy must fail"),
        Err(e) => e,
    };
    assert!(!error.to_string().contains("do-not-log"));
    server.await.unwrap();
}
