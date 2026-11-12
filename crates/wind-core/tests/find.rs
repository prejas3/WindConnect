//! Find my phone over real sessions (WN-50, PROTOCOL.md §9.5): ring, stop from
//! either end, the limit, a second ring, and a ring that outlives its asker.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use wind_core::find::{self, Asked, Ringer, Sink, State};
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Channel, DeviceKind};
use wind_core::session::{LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Phone,
        app: Some("test".into()),
    }
}

#[derive(Default)]
struct Sound {
    playing: AtomicBool,
    starts: AtomicUsize,
    stops: AtomicUsize,
    broken: AtomicBool,
}

impl Sink for Sound {
    fn ring(&self) -> bool {
        if self.broken.load(Ordering::SeqCst) {
            return false;
        }
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        true
    }
    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.playing.store(false, Ordering::SeqCst);
    }
}

struct Pair {
    pc: Session,
    phone: Session,
    ringer: Arc<Ringer>,
    sound: Arc<Sound>,
    _endpoints: (Endpoint, Endpoint),
}

/// A PC and a phone; the phone rings (if `rings`) with `limit`.
async fn pair(rings: bool, limit: Duration) -> Pair {
    pair_with(rings, false, limit).await
}

async fn pair_with(rings: bool, disabled: bool, limit: Duration) -> Pair {
    let (iphone, ipc) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pphone, ppc) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pphone.pin(ipc.public_key());
    ppc.pin(iphone.public_key());
    let a = Endpoint::bind(LOCALHOST, &iphone, pphone).unwrap();
    let b = Endpoint::bind(LOCALHOST, &ipc, ppc).unwrap();
    let addr = b.local_addr().unwrap();
    let accepting = b.clone();
    let accept = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let c = a.connect(addr, ipc.public_key()).await.unwrap();
    let s = accept.await.unwrap();
    let server = tokio::spawn(Session::accept(s, info("PC"), Router::new()));
    let mut router = Router::new();
    let streams = rings.then(|| router.route_find(4));
    if disabled {
        router.start_disabled(Channel::Find);
    }
    let (phone, _) = Session::connect(c, info("Pixel"), router).await.unwrap();
    let (pc, _) = server.await.unwrap().unwrap();
    let sound = Arc::new(Sound::default());
    let ringer = Ringer::with_limit(sound.clone(), limit);
    if let Some(streams) = streams {
        tokio::spawn(ringer.clone().serve(streams));
    }
    Pair {
        pc,
        phone,
        ringer,
        sound,
        _endpoints: (a, b),
    }
}

async fn wait(ring: &find::Ring, want: Asked) {
    let mut state = ring.state.clone();
    tokio::time::timeout(T, state.wait_for(|s| *s == want))
        .await
        .unwrap_or_else(|_| panic!("want {want:?}, have {:?}", *ring.state.borrow()))
        .unwrap();
}

#[tokio::test]
async fn ring_then_stop_from_the_pc() {
    let p = pair(true, find::RING_LIMIT).await;
    let ring = find::ring(&p.pc).await.unwrap();
    wait(&ring, Asked::Ringing).await;
    assert!(p.sound.playing.load(Ordering::SeqCst));
    assert!(p.ringer.is_ringing());
    ring.stop();
    wait(&ring, Asked::Ended(Some(State::StoppedByRequest))).await;
    assert!(!p.sound.playing.load(Ordering::SeqCst));
    assert!(!p.ringer.is_ringing());
    let _ = &p.phone;
}

#[tokio::test]
async fn stop_on_the_phone_tells_the_pc() {
    let p = pair(true, find::RING_LIMIT).await;
    let ring = find::ring(&p.pc).await.unwrap();
    wait(&ring, Asked::Ringing).await;
    p.ringer.stop_here();
    wait(&ring, Asked::Ended(Some(State::StoppedHere))).await;
    assert!(!p.sound.playing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn limit_ends_the_ring() {
    let p = pair(true, Duration::from_millis(300)).await;
    let ring = find::ring(&p.pc).await.unwrap();
    wait(&ring, Asked::Ringing).await;
    wait(&ring, Asked::Ended(Some(State::TimedOut))).await;
    assert!(!p.sound.playing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn second_ring_takes_over_without_restarting_the_sound() {
    let p = pair(true, find::RING_LIMIT).await;
    let first = find::ring(&p.pc).await.unwrap();
    wait(&first, Asked::Ringing).await;
    let second = find::ring(&p.pc).await.unwrap();
    wait(&second, Asked::Ringing).await;
    wait(&first, Asked::Ended(Some(State::StoppedByRequest))).await;
    assert!(p.sound.playing.load(Ordering::SeqCst), "still ringing");
    assert_eq!(
        p.sound.starts.load(Ordering::SeqCst),
        1,
        "sound not restarted"
    );
    first.stop(); // the old stream's Stop no longer stops anything
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(p.ringer.is_ringing());
    second.stop();
    wait(&second, Asked::Ended(Some(State::StoppedByRequest))).await;
    assert!(!p.ringer.is_ringing());
}

#[tokio::test]
async fn ring_outlives_the_session() {
    let p = pair(true, Duration::from_millis(800)).await;
    let ring = find::ring(&p.pc).await.unwrap();
    wait(&ring, Asked::Ringing).await;
    p.pc.connection()
        .close(wind_core::protocol::ErrorCode::GoingAway, b"bye");
    wait(&ring, Asked::Ended(None)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(p.ringer.is_ringing(), "the person is still searching");
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(!p.ringer.is_ringing(), "the limit still applies");
    assert!(!p.sound.playing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn no_sound_is_unavailable() {
    let p = pair(true, find::RING_LIMIT).await;
    p.sound.broken.store(true, Ordering::SeqCst);
    let ring = find::ring(&p.pc).await.unwrap();
    wait(&ring, Asked::Ended(Some(State::Unavailable))).await;
    assert!(!p.ringer.is_ringing());
}

#[tokio::test]
async fn phone_without_find_is_capability_missing() {
    let p = pair(false, find::RING_LIMIT).await;
    match find::ring(&p.pc).await {
        Err(SessionError::CapabilityMissing(Channel::Find)) => {}
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("rang"),
    }
}

#[tokio::test]
async fn withdrawing_find_refuses_new_rings() {
    let p = pair(true, find::RING_LIMIT).await;
    assert!(p.phone.set_accepted(Channel::Find, false).await.unwrap());
    let deadline = tokio::time::Instant::now() + T;
    loop {
        if matches!(
            find::ring(&p.pc).await,
            Err(SessionError::CapabilityMissing(Channel::Find))
        ) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "PC still sees find/1"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!p.ringer.is_ringing());
}

#[tokio::test]
async fn started_disabled_then_allowed_live() {
    let p = pair_with(true, true, find::RING_LIMIT).await;
    assert!(
        p.pc.peer_capabilities().iter().all(|c| c.name() != "find"),
        "not in Hello"
    );
    assert!(matches!(
        find::ring(&p.pc).await,
        Err(SessionError::CapabilityMissing(Channel::Find))
    ));
    assert!(p.phone.set_accepted(Channel::Find, true).await.unwrap());
    let deadline = tokio::time::Instant::now() + T;
    let ring = loop {
        if let Ok(ring) = find::ring(&p.pc).await {
            break ring;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "PC never saw find/1"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    wait(&ring, Asked::Ringing).await;
    ring.stop();
    wait(&ring, Asked::Ended(Some(State::StoppedByRequest))).await;
}
