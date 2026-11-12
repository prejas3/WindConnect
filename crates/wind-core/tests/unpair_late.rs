//! WN-110: an unpair notice that reaches a busy PC late (slow phone, high
//! RTT) must never end "settled on the phone, kept on the PC". Its own test
//! binary: the notice delay hook is process-wide.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::pairing::{PairedDevice, PairedDevicesFile, PairedStore, Unpairing};
use wind_core::protocol::{DeviceKind, ErrorCode};
use wind_core::session::{
    admit, testing, turn_away, Admit, LocalInfo, Router, Session, SessionEvent, TurnedAway,
    NOTICE_GRACE,
};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

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
    fn paired(&self) -> Vec<PublicKey> {
        let list = self.store.load().unwrap();
        list.devices.iter().map(|d| d.key).collect()
    }
    fn owed(&self) -> Vec<PublicKey> {
        let list = self.store.load().unwrap();
        list.revoked.iter().map(|(k, _)| *k).collect()
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

/// One run: phone A holds the PC's session; phone B removed the PC and
/// delivers its notice `delay` after `HelloAck`. Returns how the PC turned B
/// away, whether the PC still has B, and whether B still owes the notice.
async fn late_notice(delay: Duration) -> (TurnedAway, bool, bool) {
    let pc = Device::new();
    let (a, b) = (Device::new(), Device::new());
    pair(&pc, &a);
    pair(&pc, &b);
    let (outcome_tx, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
    let (ep, pc_unpairing) = (pc.endpoint.clone(), pc.unpairing());
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
                        let _ = outcome_tx.send(how);
                    });
                }
                _ => active = Some(session),
            }
        }
    });
    let (_a_session, _a_events) =
        Session::dial(&a.endpoint, pc.addr(), pc.key(), info("a"), Router::new())
            .await
            .unwrap();

    b.unpairing().unpair(pc.key(), None).await.unwrap();
    testing::NOTICE_DELAY_MS.store(delay.as_millis() as u64, Ordering::SeqCst);
    let error = Session::dial(&b.endpoint, pc.addr(), pc.key(), info("b"), Router::new())
        .await
        .expect_err("a notice dial starts no session");
    testing::NOTICE_DELAY_MS.store(0, Ordering::SeqCst);
    b.unpairing().notice_result(&error).unwrap();
    let how = tokio::time::timeout(T, outcomes.recv())
        .await
        .unwrap()
        .unwrap();
    (
        how,
        pc.paired().contains(&b.key()),
        b.owed().contains(&pc.key()),
    )
}

/// Done-when: B's Goodbye later than NOTICE_GRACE (and around it, where the
/// PC's BUSY and B's notice cross) ends with B's key gone from the PC or the
/// notice still owed on B, never "settled on B, kept on the PC".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_late_notice_is_never_settled_only_on_the_phone() {
    let grace = NOTICE_GRACE.as_millis() as u64;
    let mut seen = Vec::new();
    for delay in (grace - 100..=grace + 200).step_by(20) {
        let (how, pc_keeps_b, b_owes) = late_notice(Duration::from_millis(delay)).await;
        assert!(
            !(pc_keeps_b && !b_owes),
            "{delay} ms: B settled but the PC kept B ({how:?})"
        );
        assert_eq!(
            pc_keeps_b,
            how == TurnedAway::Busy,
            "{delay} ms: the PC's store follows its answer"
        );
        seen.push((delay, how, b_owes));
    }
    // Well inside the grace the PC reads it; well after it answers BUSY and
    // B keeps the notice to repeat.
    assert_eq!(
        seen.first().map(|s| (s.1, s.2)),
        Some((TurnedAway::Unpaired, false))
    );
    assert_eq!(
        seen.last().map(|s| (s.1, s.2)),
        Some((TurnedAway::Busy, true))
    );
}

/// The phone's half: the PC's transport acked B's notice but its app had
/// already gone the BUSY way (closes with BUSY, keeps B). B must not count
/// that as delivered: the notice stays owed and repeats.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_close_after_the_ack_leaves_the_notice_owed() {
    let pc = Device::new();
    let b = Device::new();
    pair(&pc, &b);
    let ep = pc.endpoint.clone();
    tokio::spawn(async move {
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            let Ok((session, mut events)) = Session::accept(conn, info("pc"), Router::new()).await
            else {
                continue;
            };
            tokio::spawn(async move {
                // The notice is in (so acked); the app answers BUSY anyway.
                while let Some(event) = events.recv().await {
                    if matches!(event, SessionEvent::Goodbye(_)) {
                        break;
                    }
                }
                session.goodbye(ErrorCode::Busy, None).await;
            });
        }
    });
    b.unpairing().unpair(pc.key(), None).await.unwrap();
    let error = Session::dial(&b.endpoint, pc.addr(), pc.key(), info("b"), Router::new())
        .await
        .expect_err("a notice dial starts no session");
    assert!(b.unpairing().notice_result(&error).unwrap(), "a notice");
    assert!(
        matches!(
            error,
            wind_core::session::SessionError::Unpaired {
                delivered: false,
                ..
            }
        ),
        "BUSY after the ack is not delivered: {error:?}"
    );
    assert_eq!(b.owed(), vec![pc.key()], "the notice is still owed");
    assert!(pc.paired().contains(&b.key()), "the PC kept B");
}
