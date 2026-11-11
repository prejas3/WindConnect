//! Unpairing on both sides (WC-28, PROTOCOL.md §12.10): online at once,
//! offline through a notice-mode connection, settled only on the peer's
//! acknowledgement; a revoked key gets nothing but the notice.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::pairing::{PairedDevice, PairedDevicesFile, PairedStore, Unpairing};
use wind_core::protocol::cbor::Value;
use wind_core::protocol::{
    encode_frame, encode_raw_frame, stream_header, Capability, Channel, ControlMessage, DeviceKind,
    ErrorCode, Hello, Message, RawMessage, Version, PROTOCOL_MAJOR, PROTOCOL_MINOR,
};
use wind_core::session::{LocalInfo, Router, Session, SessionError, SessionEvent};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

/// A device with its own paired-device file, kept in step with its pins.
struct Device {
    id: DeviceIdentity,
    pins: Arc<PinnedKeys>,
    endpoint: Endpoint,
    store: PairedDevicesFile,
    _dir: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        let id = DeviceIdentity::generate().unwrap();
        let pins = Arc::new(PinnedKeys::new());
        let endpoint = Endpoint::bind(LOCALHOST, &id, pins.clone()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        Device {
            id,
            pins,
            endpoint,
            store,
            _dir: dir,
        }
    }
    fn key(&self) -> PublicKey {
        self.id.public_key()
    }
    fn addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap()
    }
    fn unpairing(&self) -> Unpairing {
        Unpairing::new(self.store.clone(), self.pins.clone(), self.endpoint.clone())
    }
    /// What a restart would see: the file, read again, into fresh pins.
    fn reloaded_pins(&self) -> PinnedKeys {
        let pins = PinnedKeys::new();
        self.store.load().unwrap().pin_all(&pins);
        pins
    }
    fn paired(&self) -> Vec<PublicKey> {
        self.store
            .load()
            .unwrap()
            .devices
            .iter()
            .map(|d| d.key)
            .collect()
    }
    /// Keys whose unpair notice is still owed (removed, not settled).
    fn owed(&self) -> Vec<PublicKey> {
        self.store
            .load()
            .unwrap()
            .revoked
            .iter()
            .map(|(k, _)| *k)
            .collect()
    }
}

fn pair(a: &Device, b: &Device) {
    a.store
        .add(&PairedDevice::now(b.key(), Some("b".into())))
        .unwrap();
    b.store
        .add(&PairedDevice::now(a.key(), Some("a".into())))
        .unwrap();
    a.pins.pin(b.key());
    b.pins.pin(a.key());
}

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: None,
    }
}

/// Waits for the next `Goodbye` among `events`.
async fn goodbye(events: &mut tokio::sync::mpsc::UnboundedReceiver<SessionEvent>) -> ErrorCode {
    tokio::time::timeout(T, async {
        loop {
            match events.recv().await {
                Some(SessionEvent::Goodbye(g)) => return g.code,
                Some(_) => {}
                None => panic!("session ended without Goodbye"),
            }
        }
    })
    .await
    .expect("Goodbye in time")
}

/// Done-when (WC-28), online: the PC unpairs the connected phone; the phone
/// learns it in the session and drops the PC's key; the PC's notice is
/// settled because the phone acknowledged it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpairing_in_a_session_removes_the_key_on_the_other_side() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    let ep = pc.endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        Session::accept(conn, info("pc"), Router::new())
            .await
            .unwrap()
    });
    let (phone_session, mut phone_events) = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    let (pc_session, _pc_events) = server.await.unwrap();

    let delivered = pc
        .unpairing()
        .unpair(phone.key(), Some(&pc_session))
        .await
        .unwrap();
    assert!(delivered, "the phone acknowledged the Goodbye");
    assert_eq!(goodbye(&mut phone_events).await, ErrorCode::Unpaired);
    phone.unpairing().unpaired_by(&phone_session).unwrap();

    assert!(
        pc.paired().is_empty() && pc.owed().is_empty(),
        "PC: removed and settled"
    );
    assert!(
        phone.paired().is_empty() && phone.owed().is_empty(),
        "phone: PC's key gone"
    );
    assert!(!phone.reloaded_pins().is_trusted_key(&pc.key()));
    assert!(!pc.reloaded_pins().is_revoked_key(&phone.key()));
}

/// Done-when, offline from the PC side: the PC unpairs while the phone is
/// away (notice owed, also across a restart); the phone's next connection is
/// in notice mode on the PC, the phone gets `Goodbye UNPAIRED` and drops the
/// key, and the PC settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pc_unpairs_offline_and_the_phone_learns_on_its_next_connection() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    assert!(!pc.unpairing().unpair(phone.key(), None).await.unwrap());
    assert_eq!(pc.owed(), vec![phone.key()]);
    assert!(
        pc.reloaded_pins().is_revoked_key(&phone.key()),
        "owed across a restart"
    );

    let ep = pc.endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        assert!(conn.is_notice());
        Session::accept(conn, info("pc"), Router::new()).await
    });
    let (phone_session, mut phone_events) = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    assert!(
        phone_session.peer_capabilities().is_empty(),
        "notice HelloAck has no capabilities"
    );
    assert_eq!(
        phone_session.peer().hello.name,
        "pc",
        "name as before, nothing new"
    );
    assert_eq!(goodbye(&mut phone_events).await, ErrorCode::Unpaired);
    phone.unpairing().unpaired_by(&phone_session).unwrap();

    let error = server
        .await
        .unwrap()
        .expect_err("no session in notice mode");
    assert!(matches!(
        error,
        SessionError::Unpaired {
            delivered: true,
            ..
        }
    ));
    assert!(pc.unpairing().notice_result(&error).unwrap());
    assert!(pc.owed().is_empty(), "settled on acknowledgement");
    assert!(phone.paired().is_empty());
    // From now on the phone's attempts fail in TLS.
    assert!(
        phone.endpoint.dial(pc.addr(), pc.key()).is_err(),
        "phone no longer pins the PC"
    );
}

/// Offline from the phone side: the phone unpairs the PC, then makes one
/// notice dial (full handshake, its own router never offered); the PC, which
/// still pins the phone, gets `Goodbye UNPAIRED` and drops the phone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_unpairs_offline_and_delivers_the_notice_by_dialing() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    phone.unpairing().unpair(pc.key(), None).await.unwrap();
    assert_eq!(phone.owed(), vec![pc.key()]);

    let ep = pc.endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        assert!(!conn.is_notice(), "the PC still pins the phone");
        let (session, mut events) = Session::accept(conn, info("pc"), Router::new())
            .await
            .unwrap();
        (session.clone(), goodbye(&mut events).await, session)
    });
    let error = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .expect_err("a notice dial starts no session");
    assert!(matches!(
        error,
        SessionError::Unpaired {
            delivered: true,
            ..
        }
    ));
    phone.unpairing().notice_result(&error).unwrap();
    let (pc_session, code, _) = server.await.unwrap();
    assert_eq!(code, ErrorCode::Unpaired);
    pc.unpairing().unpaired_by(&pc_session).unwrap();
    assert!(phone.owed().is_empty() && phone.paired().is_empty());
    assert!(pc.paired().is_empty() && pc.owed().is_empty());
}

/// Settled only on acknowledgement: when the peer is gone before it takes
/// the `Goodbye`, the notice stays owed and goes again next time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_notice_not_acknowledged_stays_owed_and_repeats() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    let ep = pc.endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        Session::accept(conn, info("pc"), Router::new())
            .await
            .unwrap()
    });
    let (phone_session, _events) = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    let (pc_session, _) = server.await.unwrap();
    // The phone vanishes without a word (no close frame reaches the PC).
    phone.endpoint.rebind(LOCALHOST).unwrap();
    drop(phone_session);
    phone.endpoint.close();
    let delivered = pc
        .unpairing()
        .unpair(phone.key(), Some(&pc_session))
        .await
        .unwrap();
    assert!(!delivered);
    assert_eq!(pc.owed(), vec![phone.key()], "still owed");

    // The phone comes back (new endpoint, same identity and pins): notice.
    let phone_again = Endpoint::bind(LOCALHOST, &phone.id, phone.pins.clone()).unwrap();
    let ep = pc.endpoint.clone();
    let server = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        Session::accept(conn, info("pc"), Router::new()).await
    });
    let (_s, mut events) = Session::dial(
        &phone_again,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    assert_eq!(goodbye(&mut events).await, ErrorCode::Unpaired);
    let error = server.await.unwrap().unwrap_err();
    pc.unpairing().notice_result(&error).unwrap();
    assert!(pc.owed().is_empty(), "settled the second time");
}

/// SEC: a ticket from before the unpairing gives no 0-RTT (the unpairing
/// side forgets the peer's tickets, and a notice dial is a full handshake).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_0rtt_with_a_ticket_from_before_unpairing() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    serve(&pc);
    let (first, _e) = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    first.ping().await.unwrap();
    first.connection().quinn().close(0u32.into(), b"");
    let probe = phone.endpoint.dial(pc.addr(), pc.key()).unwrap();
    assert!(probe.early().is_some(), "a resumable ticket exists");
    drop(probe);

    phone.unpairing().unpair(pc.key(), None).await.unwrap();
    let notice_dial = phone.endpoint.dial(pc.addr(), pc.key()).unwrap();
    assert!(notice_dial.early().is_none(), "no 0-RTT after unpairing");
}

/// SEC: the PC revoked the phone, but the phone still holds a ticket and
/// sends `Hello` and a clipboard stream as 0-RTT data. The PC's application
/// sees the connection only after the full handshake and the pin check
/// (notice mode); the clipboard stream reaches no router.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streams_from_a_revoked_key_get_nothing() {
    let pc = Device::new();
    let phone = Device::new();
    pair(&pc, &phone);
    let ep = pc.endpoint.clone();
    let (clips_tx, mut clips_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            let clips_tx = clips_tx.clone();
            tokio::spawn(async move {
                let mut router = Router::new();
                let mut clip = router.route(Channel::Clipboard, 1, 8);
                let notice = conn.is_notice();
                match Session::accept(conn, info("pc"), router).await {
                    Ok((session, _e)) => {
                        let _ = session.closed().await;
                    }
                    Err(e) => {
                        let _ = clips_tx.send((notice, Some(format!("{e}"))));
                    }
                }
                while let Some(m) = clip.recv().await {
                    let _ = clips_tx.send((notice, Some(format!("CLIPBOARD {}", m.message.ty))));
                }
            });
        }
    });
    // A session to get a ticket, then the PC revokes the phone.
    let (first, _e) = Session::dial(
        &phone.endpoint,
        pc.addr(),
        pc.key(),
        info("phone"),
        Router::new(),
    )
    .await
    .unwrap();
    first.ping().await.unwrap();
    first.connection().quinn().close(0u32.into(), b"");
    pc.unpairing().unpair(phone.key(), None).await.unwrap();

    let dialing = phone.endpoint.dial(pc.addr(), pc.key()).unwrap();
    let early = dialing
        .early()
        .expect("the phone still holds a ticket")
        .clone();
    // As Session::dial does: control header + Hello in the first flight.
    let (mut control, _control_recv) = early.open_bi().await.unwrap();
    let hello = ControlMessage::Hello(Hello {
        version: Version {
            major: PROTOCOL_MAJOR as u64,
            minor: PROTOCOL_MINOR,
        },
        name: "phone".into(),
        kind: DeviceKind::Phone,
        caps: vec![Capability::new("clipboard", 1).unwrap()],
        app: None,
    });
    let mut first = stream_header(Channel::Control).to_vec();
    first.extend(encode_frame(&Message::Control(hello)).unwrap());
    control.write_all(&first).await.unwrap();
    let mut clip = early.open_uni().await.unwrap();
    let mut data = stream_header(Channel::Clipboard).to_vec();
    data.extend(
        encode_raw_frame(Channel::Clipboard, &RawMessage::new(1, Value::Map(vec![]))).unwrap(),
    );
    clip.write_all(&data).await.unwrap();
    let _ = clip.finish();
    let _ = tokio::time::timeout(T, dialing.established()).await;

    let (notice, what) = tokio::time::timeout(T, clips_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(notice, "the PC saw the phone in notice mode");
    let what = what.unwrap();
    assert!(
        what.contains("unpair notice"),
        "no session, only the notice"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), clips_rx.recv())
            .await
            .is_err(),
        "nothing reached the clipboard router"
    );
}

/// Pairing again ends a revocation (the old owed notice is dropped).
#[tokio::test]
async fn pairing_again_ends_the_revocation() {
    let a = Device::new();
    let b_key = DeviceIdentity::generate().unwrap().public_key();
    a.store.add(&PairedDevice::now(b_key, None)).unwrap();
    a.store.remove(&b_key).unwrap();
    assert!(a.reloaded_pins().is_revoked_key(&b_key));
    a.store
        .add(&PairedDevice::now(b_key, Some("again".into())))
        .unwrap();
    let pins = a.reloaded_pins();
    assert!(pins.is_trusted_key(&b_key) && !pins.is_revoked_key(&b_key));
}

fn serve(server: &Device) {
    let ep = server.endpoint.clone();
    tokio::spawn(async move {
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            tokio::spawn(async move {
                if let Ok((session, _events)) =
                    Session::accept(conn, info("server"), Router::new()).await
                {
                    let _ = session.closed().await;
                }
            });
        }
    });
}

trait Check {
    fn is_trusted_key(&self, key: &PublicKey) -> bool;
    fn is_revoked_key(&self, key: &PublicKey) -> bool;
}
impl Check for PinnedKeys {
    fn is_trusted_key(&self, key: &PublicKey) -> bool {
        use wind_core::transport::PeerTrust;
        self.is_trusted(key)
    }
    fn is_revoked_key(&self, key: &PublicKey) -> bool {
        use wind_core::transport::PeerTrust;
        self.is_revoked(key)
    }
}

/// WC-28: two phones paired with one PC start at the same time. One gets
/// the session and keeps it; the other is told `BUSY` and, following the
/// rule (no retry until a network change or the user), stays away: no
/// ping-pong for 10 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_phones_at_once_one_session_the_other_busy_without_ping_pong() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wind_core::session::{admit, close_code, Admit};

    let pc = Device::new();
    let phones = [Device::new(), Device::new()];
    for phone in &phones {
        pair(&pc, phone);
    }
    let admitted = Arc::new(AtomicUsize::new(0));
    let busy = Arc::new(AtomicUsize::new(0));
    let switches = Arc::new(AtomicUsize::new(0));
    let ep = pc.endpoint.clone();
    let (a, b, c) = (admitted.clone(), busy.clone(), switches.clone());
    tokio::spawn(async move {
        let mut active: Option<Session> = None;
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            let Ok((session, _e)) = Session::accept(conn, info("pc"), Router::new()).await else {
                continue;
            };
            match admit(active.as_ref(), &session) {
                Admit::Busy => {
                    b.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move { session.goodbye(ErrorCode::Busy, None).await });
                }
                Admit::Replace | Admit::Take => {
                    if active
                        .as_ref()
                        .is_some_and(|s| s.peer().key != session.peer().key)
                    {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                    a.fetch_add(1, Ordering::SeqCst);
                    active = Some(session);
                }
            }
        }
    });

    let mut phone_tasks = Vec::new();
    for phone in &phones {
        let (endpoint, pc_addr, pc_key) = (phone.endpoint.clone(), pc.addr(), pc.key());
        phone_tasks.push(tokio::spawn(async move {
            let mut attempts = 0;
            loop {
                attempts += 1;
                match Session::dial(&endpoint, pc_addr, pc_key, info("phone"), Router::new()).await
                {
                    Ok((session, _e)) => {
                        let end = session.closed().await;
                        if close_code(&end) == Some(ErrorCode::Busy) {
                            return (attempts, true); // waits for a network change / the user
                        }
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                }
            }
        }));
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(admitted.load(Ordering::SeqCst), 1, "one session, kept");
    assert_eq!(busy.load(Ordering::SeqCst), 1, "the other told BUSY once");
    assert_eq!(switches.load(Ordering::SeqCst), 0, "never switched devices");
    let mut finished = 0;
    for task in phone_tasks {
        if task.is_finished() {
            let (attempts, was_busy) = task.await.unwrap();
            assert!(
                was_busy && attempts == 1,
                "the BUSY phone stopped after one try"
            );
            finished += 1;
        } else {
            task.abort();
        }
    }
    assert_eq!(finished, 1, "the phone with the session is still in it");
}

/// Review WN-28 r1: phone A holds the session; phone B removed the PC while
/// away and delivers its notice by dialing. The PC's loop refuses B (admit:
/// BUSY) but must still read B's `Goodbye UNPAIRED` and drop B's key; an
/// ordinary second phone still gets BUSY.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_session_still_honours_the_peers_unpair_notice() {
    use wind_core::session::{admit, close_code, turn_away, Admit, TurnedAway};

    let pc = Device::new();
    let (a, b, c) = (Device::new(), Device::new(), Device::new());
    for phone in [&a, &b, &c] {
        pair(&pc, phone);
    }
    let (outcome_tx, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
    let ep = pc.endpoint.clone();
    let pc_unpairing = pc.unpairing();
    tokio::spawn(async move {
        let mut active: Option<Session> = None;
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            let Ok((session, mut events)) = Session::accept(conn, info("pc"), Router::new()).await
            else {
                continue;
            };
            match admit(active.as_ref(), &session) {
                Admit::Busy => {
                    let (unpairing, outcome_tx) = (pc_unpairing.clone(), outcome_tx.clone());
                    tokio::spawn(async move {
                        let how = turn_away(&session, &mut events).await;
                        if how == TurnedAway::Unpaired {
                            unpairing.unpaired_by(&session).unwrap();
                        }
                        let _ = outcome_tx.send((session.peer().key, how));
                    });
                }
                _ => active = Some(session),
            }
        }
    });
    let (a_session, _a_events) =
        Session::dial(&a.endpoint, pc.addr(), pc.key(), info("a"), Router::new())
            .await
            .unwrap();

    // B removed the PC while away; it delivers the notice by dialing.
    b.unpairing().unpair(pc.key(), None).await.unwrap();
    let error = Session::dial(&b.endpoint, pc.addr(), pc.key(), info("b"), Router::new())
        .await
        .expect_err("a notice dial starts no session");
    b.unpairing().notice_result(&error).unwrap();
    let (who, how) = tokio::time::timeout(T, outcomes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((who, how), (b.key(), TurnedAway::Unpaired));
    assert!(
        !pc.paired().contains(&b.key()),
        "B's key gone from the PC's store"
    );
    assert!(!pc.reloaded_pins().is_trusted_key(&b.key()));
    assert!(!pc.pins.is_trusted_key(&b.key()), "and from its live pins");
    assert!(b.owed().is_empty() && b.paired().is_empty(), "B settled");

    // An ordinary second phone is still told BUSY, and A keeps the session.
    let (c_session, _c_events) =
        Session::dial(&c.endpoint, pc.addr(), pc.key(), info("c"), Router::new())
            .await
            .unwrap();
    let (who, how) = tokio::time::timeout(T, outcomes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((who, how), (c.key(), TurnedAway::Busy));
    assert_eq!(close_code(&c_session.closed().await), Some(ErrorCode::Busy));
    assert!(pc.paired().contains(&c.key()), "BUSY removes nothing");
    assert!(!a_session.is_closed(), "A keeps its session");
}
