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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn touchpad_handles_release_on_replacement_and_cannot_replay_after_off_on() {
    use wind_core::{
        input::InputEvent,
        protocol::{Button, Channel},
    };
    use wind_ffi::TouchpadCommand;
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let key = PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap();
    let pc = DeviceIdentity::generate().unwrap();
    let pins = Arc::new(PinnedKeys::new());
    pins.pin(key);
    let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &pc, pins).unwrap();
    client
        .pin_peer(pc.public_key().as_bytes().to_vec())
        .unwrap();
    let accepting = endpoint.clone();
    let server = tokio::spawn(async move {
        let connection = accepting.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let input = router.route_input_version(2, 64);
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
        (session, input)
    });
    let connection = client
        .connect(
            endpoint.local_addr().unwrap().to_string(),
            pc.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    let (session, mut rx) = server.await.unwrap();
    let caps = connection.watch_touchpad();
    assert!(caps.current().await);
    let first = connection.open_touchpad().await.unwrap();
    assert!(first.gestures());
    first
        .send(vec![TouchpadCommand::Left { down: true }])
        .await
        .unwrap();
    assert_eq!(rx.recv().await, Some(InputEvent::Begin(0)));
    assert_eq!(
        rx.recv().await,
        Some(InputEvent::Button {
            button: Button::Left,
            down: true
        })
    );
    let second = connection.open_touchpad().await.unwrap();
    assert_eq!(
        rx.recv().await,
        Some(InputEvent::Button {
            button: Button::Left,
            down: false
        })
    );
    assert_eq!(rx.recv().await, Some(InputEvent::End(0)));
    assert!(first
        .send(vec![TouchpadCommand::Left { down: true }])
        .await
        .is_err());
    first.finish_input().await;
    second
        .send(vec![TouchpadCommand::Right { down: true }])
        .await
        .unwrap();
    assert_eq!(rx.recv().await, Some(InputEvent::Begin(1)));
    assert_eq!(
        rx.recv().await,
        Some(InputEvent::Button {
            button: Button::Right,
            down: true
        })
    );
    session.set_accepted(Channel::Input, false).await.unwrap();
    assert!(!caps.next().await.unwrap());
    assert_eq!(
        rx.recv().await,
        Some(InputEvent::Button {
            button: Button::Right,
            down: false
        })
    );
    assert_eq!(rx.recv().await, Some(InputEvent::End(1)));
    assert_eq!(rx.recv().await, Some(InputEvent::Withdrawn));
    session.set_accepted(Channel::Input, true).await.unwrap();
    assert!(caps.next().await.unwrap());
    assert!(second
        .send(vec![TouchpadCommand::Pointer { dx: 1, dy: 0 }])
        .await
        .is_err());
    let third = connection.open_touchpad().await.unwrap();
    third.send(vec![TouchpadCommand::TaskView]).await.unwrap();
    assert_eq!(rx.recv().await, Some(InputEvent::Begin(2)));
    assert_eq!(
        rx.recv().await,
        Some(InputEvent::Gesture(wind_core::protocol::Gesture::TaskView))
    );
    third.finish_input().await;
    assert_eq!(rx.recv().await, Some(InputEvent::End(2)));
    assert!(third
        .send(vec![TouchpadCommand::Left { down: true }])
        .await
        .is_err());
    connection.disconnect().await;
    client.shutdown().await;
    endpoint.close();
}

#[derive(Clone, Default)]
struct MemoryStorage(Arc<Mutex<Option<Vec<u8>>>>);

#[derive(Clone, Default)]
struct TestClipboard(
    Arc<Mutex<String>>,
    Arc<Mutex<Option<Arc<wind_ffi::ClipboardImage>>>>,
    Arc<Mutex<Option<Arc<wind_core::clipboard::IncomingImage>>>>,
);
impl wind_core::clipboard::Sink for TestClipboard {
    fn apply(&self, text: &str) -> bool {
        *self.0.lock().unwrap() = text.into();
        true
    }
    fn apply_image(&self, image: Arc<wind_core::clipboard::IncomingImage>) -> bool {
        *self.2.lock().unwrap() = Some(image);
        true
    }
}
impl wind_ffi::ClipboardSink for TestClipboard {
    fn apply_text(&self, text: String) -> bool {
        *self.0.lock().unwrap() = text;
        true
    }
    fn apply_image(&self, image: Arc<wind_ffi::ClipboardImage>) -> bool {
        *self.1.lock().unwrap() = Some(image);
        true
    }
}
struct Progress(Arc<Mutex<bool>>);

struct FileDestination(Arc<Mutex<Vec<wind_ffi::IncomingFile>>>);
impl wind_ffi::FileDestination for FileDestination {
    fn commit(
        &self,
        file: wind_ffi::IncomingFile,
        control: Arc<wind_ffi::IncomingControl>,
    ) -> wind_ffi::FileCommitResult {
        assert!(!control.is_cancelled());
        assert_eq!(std::fs::read(&file.source).unwrap(), b"outside");
        self.0.lock().unwrap().push(file);
        wind_ffi::FileCommitResult {
            status: wind_ffi::FileCommitStatus::Published,
            error: None,
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ffi_files_require_adoption_and_outbox_and_track_capability_withdrawal() {
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let key = PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "windconnect-wn40-ffi-{}-{}",
        std::process::id(),
        key.fingerprint().to_hex()
    ));
    let outbox = directory.join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    let outbox = outbox.canonicalize().unwrap();
    client
        .configure_files(
            directory.join("inbox").display().to_string(),
            directory.join("parts").display().to_string(),
            outbox.display().to_string(),
        )
        .await
        .unwrap();
    let source = outbox.join("photo.JPG");
    std::fs::write(&source, b"verified file bytes").unwrap();
    let outside = directory.join("outside.JPG");
    std::fs::write(&outside, b"outside").unwrap();
    let pc = DeviceIdentity::generate().unwrap();
    let pins = Arc::new(PinnedKeys::new());
    pins.pin(key);
    let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &pc, pins).unwrap();
    client
        .pin_peer(pc.public_key().as_bytes().to_vec())
        .unwrap();
    let accepting = endpoint.clone();
    let mut config = wind_core::files::FilesConfig::new(
        directory.join("PC/Downloads/WindConnect"),
        directory.join("PC/partials"),
    );
    config.type_folders = true;
    let engine = wind_core::files::FileEngine::new(config).unwrap();
    let receiving = engine.clone();
    let server = tokio::spawn(async move {
        let connection = accepting.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let incoming = router.route_files(1, 32);
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
        receiving.attach(session.clone(), Some(incoming));
        session
    });
    let connection = client
        .connect(
            endpoint.local_addr().unwrap().to_string(),
            pc.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    let session = server.await.unwrap();
    assert!(
        session.open_files().await.is_err(),
        "standby never advertises reception"
    );
    assert!(connection.set_file_receiving(true).await.is_err());
    assert!(connection.files_accepted());
    assert!(
        connection
            .queue_files(vec![source.display().to_string()])
            .await
            .is_err(),
        "standby must not queue"
    );
    connection.activate_files().await;
    assert!(
        connection.set_file_receiving(true).await.is_err(),
        "adoption alone is not a destination"
    );
    assert!(connection
        .queue_files(vec![outside.display().to_string()])
        .await
        .is_err());
    assert!(connection
        .queue_files(vec![outbox.display().to_string()])
        .await
        .is_err());
    let ids = connection
        .queue_files(vec![source.display().to_string()])
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rows = client.file_transfers().unwrap();
            if rows.iter().any(|r| r.id == ids[0] && r.state == "done") {
                break;
            }
            client.next_file_transfers().await.unwrap();
        }
    })
    .await
    .unwrap();
    let mut received_updates = engine.subscribe();
    let received = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(row) = engine
                .transfers()
                .into_iter()
                .find(|r| r.saved_as.is_some())
            {
                break row;
            }
            received_updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let saved = received.saved_as.unwrap();
    assert_eq!(saved.parent().unwrap().file_name().unwrap(), "Obrazy");
    assert_eq!(std::fs::read(saved).unwrap(), b"verified file bytes");
    let committed = Arc::new(Mutex::new(Vec::new()));
    client.set_file_destination(Box::new(FileDestination(committed.clone())));
    connection.set_file_receiving(true).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while session.open_files().await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let incoming_id = engine
        .send(*key.fingerprint().as_bytes(), &outside)
        .unwrap()[0];
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut updates = engine.subscribe();
        while !engine
            .transfers()
            .iter()
            .any(|row| row.id == incoming_id && row.state == wind_core::files::TransferState::Done)
        {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let file = committed.lock().unwrap()[0].clone();
    assert_eq!(file.peer, pc.fingerprint().to_hex());
    assert_eq!(file.name, "outside.JPG");
    assert!(client
        .file_transfers()
        .unwrap()
        .iter()
        .any(|row| row.incoming && row.id == file.id && row.state == "done"));
    client.release_incoming_file(file.peer, file.id).unwrap();
    assert!(!std::path::Path::new(&file.source).exists());
    connection.set_file_receiving(false).await.unwrap();
    session
        .set_accepted(wind_core::protocol::Channel::Files, false)
        .await
        .unwrap();
    assert!(
        !tokio::time::timeout(Duration::from_secs(3), connection.next_files_accepted())
            .await
            .unwrap()
            .unwrap()
    );
    assert!(!connection.files_accepted());
    assert!(connection
        .queue_files(vec![source.display().to_string()])
        .await
        .is_err());
    connection.disconnect().await;
    client.shutdown().await;
    endpoint.close();
    drop(session);
    drop(connection);
    drop(client);
    drop(engine);
    let _ = std::fs::remove_dir_all(directory);
}
impl wind_ffi::ClipboardImageProgress for Progress {
    fn applied(&self) {
        *self.0.lock().unwrap() = true;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ffi_clipboard_uses_sink_and_applied_ack_in_both_directions() {
    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let local = TestClipboard::default();
    client.set_clipboard_sink(Box::new(local.clone()));
    let identity = DeviceIdentity::generate().unwrap();
    let pins = Arc::new(PinnedKeys::new());
    pins.pin(PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap());
    let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &identity, pins).unwrap();
    let address = endpoint.local_addr().unwrap().to_string();
    client
        .pin_peer(identity.public_key().as_bytes().to_vec())
        .unwrap();
    let accepting = endpoint.clone();
    let accept = tokio::spawn(async move {
        let incoming = accepting.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let input = wind_core::clipboard::Clipboard::route(&mut router);
        let (session, _) = Session::accept(
            incoming,
            LocalInfo {
                name: "PC".into(),
                kind: DeviceKind::Computer,
                app: None,
            },
            router,
        )
        .await
        .unwrap();
        let hub = Arc::new(wind_core::clipboard::Clipboard::default());
        let remote = TestClipboard::default();
        hub.set_sink(Arc::new(remote.clone()));
        (hub.attach(session, input), remote)
    });
    let connection = client
        .connect(address, identity.public_key().as_bytes().to_vec())
        .await
        .unwrap();
    let (pc, remote) = accept.await.unwrap();
    connection.activate_clipboard().await;
    assert_eq!(
        connection
            .send_clipboard("telefon 👩‍💻".into(), false, false)
            .await,
        "applied"
    );
    assert_eq!(*remote.0.lock().unwrap(), "telefon 👩‍💻");
    assert_eq!(
        pc.send("https://example.com/ż".into(), false, false).await,
        wind_core::clipboard::Status::Applied
    );
    assert_eq!(*local.0.lock().unwrap(), "https://example.com/ż");
    assert_eq!(
        connection
            .send_clipboard("https://example.com/ż".into(), false, false)
            .await,
        "unchanged"
    );
    assert_eq!(
        connection
            .send_clipboard("test password".into(), true, true)
            .await,
        "sensitive"
    );
    // WN-36: an image PC -> phone reaches the sink at once and reads in full
    // through ClipboardImage::read; phone -> PC goes with send_clipboard_image.
    let png: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let image = |png: &Vec<u8>| wind_core::clipboard::OutgoingImage {
        png: png.clone(),
        thumbnail: vec![7; 100],
        width: 640,
        height: 480,
    };
    assert_eq!(
        pc.send_image(image(&png), false, false, || {}).await,
        wind_core::clipboard::Status::Applied
    );
    let got = local.1.lock().unwrap().clone().expect("image applied");
    assert_eq!((got.width(), got.height(), got.size()), (640, 480, 700_000));
    assert_eq!(got.thumbnail(), vec![7; 100]);
    assert_eq!(got.id().len(), 32);
    let read = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        loop {
            let part = got.read(out.len() as u64, 65_536).unwrap();
            if part.is_empty() {
                return out;
            }
            out.extend(part);
        }
    })
    .await
    .unwrap();
    assert!(read == png);
    let applied = Arc::new(Mutex::new(false));
    let back: Vec<u8> = png.iter().rev().copied().collect();
    let result = connection
        .send_clipboard_image(
            back.clone(),
            vec![1; 10],
            480,
            640,
            false,
            false,
            Box::new(Progress(applied.clone())),
        )
        .await;
    assert_eq!(
        (result.status.as_str(), result.peer_limit),
        ("applied", None)
    );
    assert!(*applied.lock().unwrap());
    let at_pc = remote.2.lock().unwrap().clone().unwrap();
    assert!(at_pc.wait_complete(Duration::from_secs(1)).unwrap() == back);
    // The phone's limit refuses a PC image over it, and says the limit.
    client.set_clipboard_image_limit(500_000);
    assert_eq!(
        pc.send_image(
            image(&png.iter().map(|b| b ^ 1).collect()),
            false,
            true,
            || {}
        )
        .await,
        wind_core::clipboard::Status::PeerLimit(500_000)
    );

    client.set_clipboard_enabled(false);
    assert_eq!(
        pc.send("disabled".into(), false, false).await,
        wind_core::clipboard::Status::Refused
    );
    connection.disconnect().await;
    client.shutdown().await;
    endpoint.close();
    endpoint.wait_idle().await;
}

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

/// WC-28 through the FFI: the list with rename, the computer removing the
/// phone in a session ("unpaired" -> forget_peer), the phone removing a
/// computer offline and delivering the notice, and "busy".
#[tokio::test(flavor = "multi_thread")]
async fn ffi_paired_list_rename_unpair_both_ways_and_busy() {
    use wind_core::pairing::{PairedDevice, PairedDevicesFile, PairedStore};
    use wind_core::protocol::ErrorCode;
    use wind_core::session::SessionEvent;

    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let phone_key =
        PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "windconnect-wn28-{}-{}",
        std::process::id(),
        phone_key.fingerprint().to_hex()
    ));
    let directory = dir.to_string_lossy().into_owned();
    let store = PairedDevicesFile::in_dir(&dir);
    let pc = |name: &str| {
        let id = DeviceIdentity::generate().unwrap();
        store
            .add(&PairedDevice::now(id.public_key(), Some(name.into())))
            .unwrap();
        let trust = Arc::new(PinnedKeys::new());
        trust.pin(phone_key);
        let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &id, trust).unwrap();
        (id, endpoint)
    };
    let local = || LocalInfo {
        name: "PC".into(),
        kind: DeviceKind::Computer,
        app: None,
    };
    let (desk, desk_ep) = pc("Desk");
    let (laptop, laptop_ep) = pc("Laptop");
    let list = client.paired_list(directory.clone()).unwrap();
    assert_eq!(list.len(), 2);
    client
        .rename_paired(
            directory.clone(),
            desk.public_key().as_bytes().to_vec(),
            "Komputer w pracy".into(),
        )
        .unwrap();
    assert!(matches!(
        client.rename_paired(
            directory.clone(),
            desk.public_key().as_bytes().to_vec(),
            " ".into()
        ),
        Err(CoreError::InvalidInput { .. })
    ));
    let list = client.paired_list(directory.clone()).unwrap();
    assert_eq!(list[0].name, "Komputer w pracy");

    // "busy": the computer keeps another device's session.
    let ep = desk_ep.clone();
    let busy = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        let (s, _) = Session::accept(conn, local(), Router::new()).await.unwrap();
        s.goodbye(ErrorCode::Busy, None).await
    });
    let connection = client
        .connect(
            desk_ep.local_addr().unwrap().to_string(),
            desk.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(connection.wait_end().await, "busy");
    busy.await.unwrap();

    // The computer removes the phone during a session.
    let ep = desk_ep.clone();
    let removal = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        let (s, _) = Session::accept(conn, local(), Router::new()).await.unwrap();
        s.goodbye(ErrorCode::Unpaired, None).await
    });
    let connection = client
        .connect(
            desk_ep.local_addr().unwrap().to_string(),
            desk.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(connection.wait_end().await, "unpaired");
    connection.forget_peer(directory.clone()).unwrap();
    assert!(removal.await.unwrap(), "the phone acknowledged");
    let list = client.paired_list(directory.clone()).unwrap();
    assert_eq!(list.len(), 1, "the desk computer is gone");
    assert_eq!(list[0].public_key, laptop.public_key().as_bytes());

    // The phone removes the laptop while away, then delivers the notice.
    client
        .unpair(directory.clone(), laptop.public_key().as_bytes().to_vec())
        .await
        .unwrap();
    let list = client.paired_list(directory.clone()).unwrap();
    assert!(list.len() == 1 && list[0].owed, "owed: {list:?}");
    let ep = laptop_ep.clone();
    let told = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        let (_s, mut events) = Session::accept(conn, local(), Router::new()).await.unwrap();
        loop {
            match events.recv().await {
                Some(SessionEvent::Goodbye(g)) => return g.code,
                Some(_) => {}
                None => panic!("no Goodbye"),
            }
        }
    });
    let delivered = client
        .deliver_notice(
            directory.clone(),
            laptop_ep.local_addr().unwrap().to_string(),
            laptop.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    assert!(delivered);
    assert_eq!(told.await.unwrap(), ErrorCode::Unpaired);
    assert!(client.paired_list(directory.clone()).unwrap().is_empty());
    client.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review WN-28 r1, phone side: a computer that removed this phone answers
/// its connection in notice mode. The connection comes up, then ends
/// "unpaired" (before any adoption/ping succeeds), so the caller can still
/// forget the computer instead of dropping the notice with the connection.
#[tokio::test(flavor = "multi_thread")]
async fn ffi_connection_to_a_computer_that_removed_us_ends_unpaired() {
    use wind_core::pairing::{PairedDevice, PairedDevicesFile, PairedStore};

    let client = open_client(
        Box::new(MemoryStorage::default()),
        "Phone".into(),
        "127.0.0.1:0".into(),
    )
    .await
    .unwrap();
    let phone_key =
        PublicKey::from_bytes(&client.identity().public_key.try_into().unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "windconnect-wn28-notice-{}-{}",
        std::process::id(),
        phone_key.fingerprint().to_hex()
    ));
    let directory = dir.to_string_lossy().into_owned();
    let pc = DeviceIdentity::generate().unwrap();
    PairedDevicesFile::in_dir(&dir)
        .add(&PairedDevice::now(pc.public_key(), Some("PC".into())))
        .unwrap();
    assert_eq!(client.paired_list(directory.clone()).unwrap().len(), 1);
    // The PC removed the phone while it was away: revoked, notice owed.
    let trust = Arc::new(PinnedKeys::new());
    trust.revoke(phone_key);
    let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &pc, trust).unwrap();
    let ep = endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        assert!(conn.is_notice());
        Session::accept(
            conn,
            LocalInfo {
                name: "PC".into(),
                kind: DeviceKind::Computer,
                app: None,
            },
            Router::new(),
        )
        .await
        .expect_err("notice mode")
    });
    let connection = client
        .connect(
            endpoint.local_addr().unwrap().to_string(),
            pc.public_key().as_bytes().to_vec(),
        )
        .await
        .unwrap();
    assert_eq!(connection.wait_end().await, "unpaired");
    connection.forget_peer(directory.clone()).unwrap();
    assert!(client.paired_list(directory.clone()).unwrap().is_empty());
    assert!(matches!(
        server.await.unwrap(),
        wind_core::session::SessionError::Unpaired {
            delivered: true,
            ..
        }
    ));
    client.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
