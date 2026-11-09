use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use wind_core::{
    identity::{DeviceIdentity, PublicKey},
    protocol::DeviceKind,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
use wind_ffi::{open_client, CoreError, IdentityStorage};

#[derive(Clone, Default)]
struct MemoryStorage(Arc<Mutex<Option<Vec<u8>>>>);

impl IdentityStorage for MemoryStorage {
    fn load(&self) -> Result<Option<Vec<u8>>, CoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn store_new(&self, pem: Vec<u8>) -> Result<bool, CoreError> {
        let mut stored = self.0.lock().unwrap();
        if stored.is_some() {
            return Ok(false);
        }
        *stored = Some(pem);
        Ok(true)
    }
}

#[tokio::test]
async fn identity_survives_reopening_and_corruption_is_not_replaced() {
    let storage = MemoryStorage::default();
    let first = open_client(
        Box::new(storage.clone()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let key = first.identity().public_key;
    first.shutdown().await;
    let second = open_client(
        Box::new(storage.clone()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    assert_eq!(second.identity().public_key, key);
    second.shutdown().await;
    *storage.0.lock().unwrap() = Some(b"corrupt".to_vec());
    assert!(matches!(
        open_client(
            Box::new(storage.clone()),
            "Phone".into(),
            "127.0.0.1:0".into()
        )
        .await,
        Err(CoreError::Storage { .. })
    ));
    assert_eq!(storage.load().unwrap().unwrap(), b"corrupt");
}

#[tokio::test(flavor = "multi_thread")]
async fn ffi_connects_to_core_session_and_refuses_a_different_pinned_key() {
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let server_identity = DeviceIdentity::generate().unwrap();
    let trust = Arc::new(PinnedKeys::new());
    trust.pin(PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap());
    let server = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &server_identity, trust).unwrap();
    let address = server.local_addr().unwrap().to_string();
    assert!(matches!(
        client
            .connect(
                address.clone(),
                server_identity.public_key().as_bytes().to_vec()
            )
            .await,
        Err(CoreError::UntrustedPeer { .. })
    ));
    client
        .pin_peer(server_identity.public_key().as_bytes().to_vec())
        .unwrap();
    let server_task = tokio::spawn(async move {
        let connection = server.accept().await.unwrap().unwrap();
        let (session, _) = Session::accept(
            connection,
            LocalInfo {
                name: "PC".into(),
                kind: DeviceKind::Computer,
                app: None,
            },
            Router::new(),
        )
        .await
        .unwrap();
        session.closed().await;
        server.close();
        server.wait_idle().await;
    });
    let connection = client
        .connect(
            address.clone(),
            server_identity.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(connection.peer().name, "PC");
    assert_eq!(
        connection.peer().public_key,
        server_identity.public_key().as_bytes()
    );
    connection.ping().await.unwrap();
    connection.disconnect().await;
    tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap();

    let other = DeviceIdentity::generate().unwrap();
    client
        .pin_peer(other.public_key().as_bytes().to_vec())
        .unwrap();
    let server = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &server_identity,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let address = server.local_addr().unwrap().to_string();
    let (refused, _) = tokio::join!(
        client.connect(address, other.public_key().as_bytes().to_vec()),
        server.accept(),
    );
    assert!(matches!(refused, Err(CoreError::UntrustedPeer { .. })));
    server.close();
    server.wait_idle().await;
    client.shutdown().await;
}

#[tokio::test]
async fn platform_storage_failure_is_returned_without_creating_a_key() {
    struct Broken;
    impl IdentityStorage for Broken {
        fn load(&self) -> Result<Option<Vec<u8>>, CoreError> {
            Err(CoreError::Storage {
                reason: "Keystore unavailable".into(),
            })
        }
        fn store_new(&self, _: Vec<u8>) -> Result<bool, CoreError> {
            panic!("must not replace identity")
        }
    }
    assert!(matches!(
        open_client(Box::new(Broken), "Phone".into(), "127.0.0.1:0".into()).await,
        Err(CoreError::Storage { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn network_callback_restarts_an_inflight_connection_without_waiting_for_timeout() {
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let identity = DeviceIdentity::generate().unwrap();
    client
        .pin_peer(identity.public_key().as_bytes().to_vec())
        .unwrap();
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = silent.local_addr().unwrap();
    let dialing_client = client.clone();
    let key = identity.public_key().as_bytes().to_vec();
    let dialing =
        tokio::spawn(async move { dialing_client.connect(address.to_string(), key).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!dialing.is_finished());
    drop(silent);
    let trust = Arc::new(PinnedKeys::new());
    trust.pin(PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap());
    let server = Endpoint::bind(address, &identity, trust).unwrap();
    let accepted = tokio::spawn(async move {
        let session = loop {
            let Ok(conn) = server.accept().await.unwrap() else {
                continue;
            };
            // CCS/1 RULE:the pre-change handshake can arrive and then be cancelled; accept the fresh one.
            if let Ok((session, _)) = Session::accept(
                conn,
                LocalInfo {
                    name: "PC".into(),
                    kind: DeviceKind::Computer,
                    app: None,
                },
                Router::new(),
            )
            .await
            {
                break session;
            }
        };
        session.closed().await;
        server.close();
        server.wait_idle().await;
    });
    let start = std::time::Instant::now();
    client.network_changed().await.unwrap();
    let connection = tokio::time::timeout(Duration::from_millis(500), dialing)
        .await
        .expect("network signal must not wait for the 10 s attempt timeout")
        .unwrap()
        .unwrap();
    println!(
        "session established {:?} after network change",
        start.elapsed()
    );
    connection.ping().await.unwrap();
    connection.disconnect().await;
    accepted.await.unwrap();
    client.shutdown().await;
}
