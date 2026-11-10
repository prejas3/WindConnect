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

/// WN-84: a fixed listening port survives `network_changed` (firewall rules
/// and known addresses keep matching); port 0 still moves to a fresh one
/// (WN-16). In both cases an open session keeps working.
#[tokio::test(flavor = "multi_thread")]
async fn network_change_keeps_a_fixed_port_and_the_session() {
    async fn port_across_network_change(bind: String) -> (u16, u16) {
        let client = open_client(Box::new(MemoryStorage::default()), "Phone".into(), bind)
            .await
            .unwrap();
        let server_identity = DeviceIdentity::generate().unwrap();
        client
            .pin_peer(server_identity.public_key().as_bytes().to_vec())
            .unwrap();
        let trust = Arc::new(PinnedKeys::new());
        trust
            .pin(PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap());
        let server =
            Endpoint::bind("127.0.0.1:0".parse().unwrap(), &server_identity, trust).unwrap();
        let address = server.local_addr().unwrap().to_string();
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
            .connect(address, server_identity.public_key().as_bytes().to_vec())
            .await
            .unwrap();
        connection.ping().await.unwrap();
        let port = |client: &wind_ffi::CoreClient| {
            client
                .local_address()
                .unwrap()
                .parse::<std::net::SocketAddr>()
                .unwrap()
                .port()
        };
        let before = port(&client);
        client.network_changed().await.unwrap();
        let after = port(&client);
        tokio::time::timeout(Duration::from_secs(2), connection.ping())
            .await
            .expect("the session must survive the network change")
            .unwrap();
        connection.disconnect().await;
        tokio::time::timeout(Duration::from_secs(5), server_task)
            .await
            .unwrap()
            .unwrap();
        client.shutdown().await;
        (before, after)
    }

    let free = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let fixed = free.local_addr().unwrap().port();
    drop(free);
    assert_eq!(
        port_across_network_change(format!("127.0.0.1:{fixed}")).await,
        (fixed, fixed)
    );
    let (before, after) = port_across_network_change("127.0.0.1:0".into()).await;
    assert_ne!(before, after, "port 0 keeps WN-16: a fresh socket");
}

/// WN-60 (D3): the relay is off until set; a URL the core refuses changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn relay_is_off_by_default_and_checked_when_set() {
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    assert_eq!(client.relay_url(), None);
    for bad in [
        "http://relay.example",
        "ws://relay.example.com",
        "wss://x/v1/relay",
    ] {
        assert!(matches!(
            client.set_relay(Some(bad.into())),
            Err(CoreError::InvalidInput { .. })
        ));
    }
    assert_eq!(client.relay_url(), None);
    client
        .set_relay(Some("wss://windconnect-relay.example.workers.dev".into()))
        .unwrap();
    assert_eq!(
        client.relay_url().as_deref(),
        Some("wss://windconnect-relay.example.workers.dev")
    );
    client.set_relay(None).unwrap();
    assert_eq!(client.relay_url(), None);
    // Off: an unreachable address fails without any relay attempt.
    let identity = DeviceIdentity::generate().unwrap();
    let key = identity.public_key().as_bytes().to_vec();
    client.pin_peer(key.clone()).unwrap();
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let error = client
        .connect(silent.local_addr().unwrap().to_string(), key)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, CoreError::Transport { .. }), "{error:?}");
    client.shutdown().await;
}

/// WN-60: away from home (the LAN address does not answer) the phone falls
/// back to the relay; back home, `connect_direct` reaches the PC again.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the relay Worker running locally (wrangler dev) and RELAY_URL"]
async fn falls_back_to_the_relay_and_returns_to_lan() {
    let url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:8787".into());
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let phone_key =
        PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap();
    let pc = DeviceIdentity::generate().unwrap();
    let trust = Arc::new(PinnedKeys::new());
    trust.pin(phone_key);
    let server = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &pc, trust).unwrap();
    let waiting = wind_core::relay::RelayLink::connect(&url, &pc, phone_key, server.clone())
        .await
        .unwrap();
    let acceptor = server.clone();
    let accepted = tokio::spawn(async move {
        let mut sessions = Vec::new();
        for _ in 0..2 {
            let conn = acceptor.accept().await.unwrap().unwrap();
            let (session, _) = Session::accept(
                conn,
                LocalInfo {
                    name: "PC".into(),
                    kind: DeviceKind::Computer,
                    app: None,
                },
                Router::new(),
            )
            .await
            .unwrap();
            sessions.push(session);
        }
        sessions
    });
    let key = pc.public_key().as_bytes().to_vec();
    client.pin_peer(key.clone()).unwrap();
    client.set_relay(Some(url)).unwrap();

    // The PC's home address does not answer from here.
    let away = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let start = std::time::Instant::now();
    let relayed = client
        .connect(away.local_addr().unwrap().to_string(), key.clone())
        .await
        .unwrap();
    println!("relay fallback after {:?}", start.elapsed());
    assert_eq!(relayed.route(), "relay");
    assert_eq!(relayed.peer().name, "PC");
    relayed.ping().await.unwrap();

    let home = client
        .connect_direct(server.local_addr().unwrap().to_string(), key)
        .await
        .unwrap();
    assert_eq!(home.route(), "lan");
    home.ping().await.unwrap();
    relayed.disconnect().await;
    home.disconnect().await;
    drop(accepted.await.unwrap());
    drop(waiting);
    server.close();
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ffi_runs_remote_actions_by_id_only() {
    use wind_core::actions::{Action, ActionHost, Builtin, Executor, Policy};
    #[derive(Default)]
    struct Recorder(Mutex<Vec<Action>>);
    impl Executor for Recorder {
        fn supports(&self, b: Builtin) -> bool {
            b != Builtin::Sleep
        }
        fn run(&self, action: &Action) -> Result<(), String> {
            self.0.lock().unwrap().push(action.clone());
            Ok(())
        }
    }
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
    let server = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &server_identity,
        trust.clone(),
    )
    .unwrap();
    let address = server.local_addr().unwrap().to_string();
    client
        .pin_peer(server_identity.public_key().as_bytes().to_vec())
        .unwrap();
    let ran = Arc::new(Recorder::default());
    let host_ran = ran.clone();
    let server_task = tokio::spawn(async move {
        let connection = server.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let streams = router.route_actions(4);
        let (session, _) = Session::accept(
            connection,
            LocalInfo {
                name: "PC".into(),
                kind: DeviceKind::Computer,
                app: None,
            },
            router,
        )
        .await
        .unwrap();
        tokio::spawn(
            ActionHost::new(Arc::new(Policy::default()), host_ran, trust)
                .serve(session.clone(), streams),
        );
        session.closed().await;
        server.close();
        server.wait_idle().await;
    });
    let connection = client
        .connect(address, server_identity.public_key().as_bytes().to_vec())
        .await
        .unwrap();
    let ids: Vec<_> = connection
        .list_actions()
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(ids, ["lock", "open-url"]);
    assert_eq!(
        connection.run_action("lock".into(), None).await.unwrap(),
        "done"
    );
    assert_eq!(
        connection.run_action("sleep".into(), None).await.unwrap(),
        "refused"
    );
    assert_eq!(
        connection
            .run_action("open-url".into(), Some("javascript:alert(1)".into()))
            .await
            .unwrap(),
        "bad-argument"
    );
    assert!(matches!(
        connection.run_action("cmd /c calc".into(), None).await,
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(*ran.0.lock().unwrap(), vec![Action::Lock]);
    connection.disconnect().await;
    tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap();
    client.shutdown().await;
}
