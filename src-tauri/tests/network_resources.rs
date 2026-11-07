pub use aerotab_core::{network_resources, profile, proxy_transport, ssh, vault};

use network_resources::*;
use uuid::Uuid;

#[tokio::test]
async fn resource_crud_is_durable_and_references_block_deletion() {
    let dir = std::env::temp_dir().join(format!("network-test-{}", Uuid::new_v4()));
    let id = Uuid::new_v4();
    {
        let store = NetworkResourceStore::open(&dir).unwrap();
        let proxy = ProxyResource {
            id,
            name: "proxy".into(),
            config: ProxyConfig::Socks5 {
                host: "localhost".into(),
                port: 1080,
                vault_entry_id: None,
            },
        };
        store.save_proxy(proxy.clone()).unwrap();
        let mut renamed = proxy;
        renamed.name = "renamed".into();
        store.save_proxy(renamed).unwrap();
        assert!(store.delete_proxy(id, &["production".into()]).is_err());
        assert_eq!(store.list_proxies().unwrap()[0].id, id);
    }
    let store = NetworkResourceStore::open(&dir).unwrap();
    assert_eq!(store.get_proxy(id).unwrap().unwrap().name, "renamed");
    assert!(store.delete_proxy(id, &[]).unwrap());
}

#[test]
fn forwarding_defaults_to_loopback_and_rejects_missing_targets() {
    let raw = serde_json::json!({"id":Uuid::new_v4(),"name":"web","profile_id":Uuid::new_v4(),"kind":"local","bind_port":8080,"target_host":"example.com","target_port":80});
    let mut rule: ForwardingRule = serde_json::from_value(raw).unwrap();
    assert_eq!(rule.bind_host, "127.0.0.1");
    assert!(rule.validate().is_ok());
    rule.target_host.clear();
    assert!(rule.validate().is_err());
}

#[test]
fn refuses_plaintext_credentials_and_invalid_endpoints() {
    assert!(serde_json::from_value::<ProxyResource>(serde_json::json!({"id":Uuid::new_v4(),"name":"bad","config":{"kind":"socks5","host":"localhost","port":1080,"password":"secret"}})).is_err());
    let proxy = ProxyResource {
        id: Uuid::new_v4(),
        name: "bad".into(),
        config: ProxyConfig::HttpConnect {
            host: "bad\r\nHeader".into(),
            port: 80,
            vault_entry_id: None,
        },
    };
    assert!(proxy.validate().is_err());
}

#[test]
fn forwarding_and_jump_profile_dependencies_are_enumerated() {
    let dir = std::env::temp_dir().join(format!("network-deps-{}", Uuid::new_v4()));
    let store = NetworkResourceStore::open(dir).unwrap();
    let profile_id = Uuid::new_v4();
    store
        .save_proxy(ProxyResource {
            id: Uuid::new_v4(),
            name: "jump".into(),
            config: ProxyConfig::SshJump { profile_id },
        })
        .unwrap();
    let rule:ForwardingRule=serde_json::from_value(serde_json::json!({"id":Uuid::new_v4(),"name":"socks","profile_id":profile_id,"kind":"dynamic","bind_port":1080})).unwrap();
    store.save_rule(rule).unwrap();
    assert_eq!(store.profile_dependents(profile_id).unwrap().len(), 2);
}

#[tokio::test]
async fn socks5_handles_fragmented_reply_and_preserves_payload() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        tcp.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 0]);
        tcp.write_all(&[5]).await.unwrap();
        tcp.write_all(&[0]).await.unwrap();
        let mut header = [0; 5];
        tcp.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[..4], &[5, 1, 0, 3]);
        let mut addr = vec![0; header[4] as usize + 2];
        tcp.read_exact(&mut addr).await.unwrap();
        assert_eq!(&addr[..addr.len() - 2], b"target.invalid");
        tcp.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 22, b'S', b'S', b'H'])
            .await
            .unwrap();
    });
    let proxy = ProxyResource {
        id: Uuid::new_v4(),
        name: "test".into(),
        config: ProxyConfig::Socks5 {
            host: "127.0.0.1".into(),
            port,
            vault_entry_id: None,
        },
    };
    let mut stream = proxy_transport::dial_proxy(&proxy, "target.invalid", 22, None)
        .await
        .unwrap();
    let mut banner = [0; 3];
    stream.read_exact(&mut banner).await.unwrap();
    assert_eq!(&banner, b"SSH");
    server.await.unwrap();
}

#[tokio::test]
async fn http_connect_preserves_banner_and_rejects_header_injection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proxy = ProxyResource {
        id: Uuid::new_v4(),
        name: "http".into(),
        config: ProxyConfig::HttpConnect {
            host: "127.0.0.1".into(),
            port,
            vault_entry_id: None,
        },
    };
    assert!(
        proxy_transport::dial_proxy(&proxy, "evil\r\nInjected: yes", 22, None)
            .await
            .is_err()
    );
    let server = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(tcp.read_u8().await.unwrap());
        }
        assert!(request.starts_with(b"CONNECT target:22 HTTP/1.1\r\n"));
        tcp.write_all(b"HTTP/1.1 200 Connection established\r\n\r\nSSH")
            .await
            .unwrap();
    });
    let mut stream = proxy_transport::dial_proxy(&proxy, "target", 22, None)
        .await
        .unwrap();
    let mut banner = [0; 3];
    stream.read_exact(&mut banner).await.unwrap();
    assert_eq!(&banner, b"SSH");
    server.await.unwrap();
}
