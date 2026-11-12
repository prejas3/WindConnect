//! Notifications over real sessions (WN-42, PROTOCOL.md §9.7): the phone's notifications to a PC
//! that takes notify/1; Icon once per key, the one-a-second window, Removed only for what went out.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::Receiver;
use wind_core::identity::DeviceIdentity;
use wind_core::notify::{self, icon_key, Message, Outcome, Posted, Sender};
use wind_core::protocol::{Channel, DeviceKind};
use wind_core::session::{ChannelMessage, LocalInfo, Router, Session};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);
const PNG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Phone,
        app: Some("test".into()),
    }
}

struct Pair {
    pc: Session,
    phone: Session,
    shown: Option<Receiver<ChannelMessage>>,
    _endpoints: (Endpoint, Endpoint),
}

async fn pair(pc_takes: bool) -> Pair {
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
    let mut pc_router = Router::new();
    let shown = pc_takes.then(|| notify::route(&mut pc_router));
    let server = tokio::spawn(Session::accept(s, info("PC"), pc_router));
    let (phone, _) = Session::connect(c, info("Pixel"), Router::new())
        .await
        .unwrap();
    let (pc, _) = server.await.unwrap().unwrap();
    Pair {
        pc,
        phone,
        shown,
        _endpoints: (a, b),
    }
}

fn note(id: u8, text: &str) -> Posted {
    Posted {
        id: [id; 16],
        app: "Signal".into(),
        package: "org.thoughtcrime.securesms".into(),
        title: Some("Ola".into()),
        text: Some(text.into()),
        time_ms: 1_794_500_000_000,
        icon: None,
    }
}

async fn next(rx: &mut Receiver<ChannelMessage>) -> Message {
    let m = tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap();
    assert_eq!(m.channel, Channel::Notify);
    Message::decode(&m.message).expect("well-formed")
}

#[tokio::test]
async fn notifications_reach_the_pc() {
    let mut p = pair(true).await;
    let shown = p.shown.as_mut().unwrap();
    let sender = Sender::new(p.phone.clone());
    let icon = PNG.to_vec();
    let key = icon_key(&icon);

    assert_eq!(
        sender
            .posted(note(1, "Będziesz o 19?"), Some(icon.clone()), false)
            .await
            .unwrap(),
        Outcome::Sent
    );
    assert_eq!(
        next(shown).await,
        Message::Icon {
            key,
            png: icon.clone()
        },
        "the icon first"
    );
    assert_eq!(
        next(shown).await,
        Message::Posted(Posted {
            icon: Some(key),
            ..note(1, "Będziesz o 19?")
        })
    );
    // Another notification of the same app: no second Icon.
    assert_eq!(
        sender
            .posted(note(2, "Biorę klucze"), Some(icon.clone()), false)
            .await
            .unwrap(),
        Outcome::Sent
    );
    assert!(
        matches!(next(shown).await, Message::Posted(n) if n.id == [2; 16] && n.icon == Some(key))
    );

    // Within the window: held, the latest goes out when it ends.
    let start = Instant::now();
    assert_eq!(
        sender
            .posted(note(1, "a"), Some(icon.clone()), false)
            .await
            .unwrap(),
        Outcome::Held
    );
    assert_eq!(
        sender
            .posted(note(1, "b"), Some(icon.clone()), false)
            .await
            .unwrap(),
        Outcome::Held
    );
    let late = next(shown).await;
    assert!(
        matches!(&late, Message::Posted(n) if n.text.as_deref() == Some("b")),
        "{late:?}"
    );
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_millis(800) && waited < Duration::from_millis(2500),
        "{waited:?}"
    );

    // Removed only for what was sent.
    assert_eq!(sender.removed([9; 16]).await.unwrap(), Outcome::Dropped);
    assert_eq!(sender.removed([2; 16]).await.unwrap(), Outcome::Sent);
    assert_eq!(next(shown).await, Message::Removed { id: [2; 16] });

    // Not a PNG: left out, no icon key.
    assert_eq!(
        sender
            .posted(note(3, "x"), Some(vec![0xff, 0xd8, 0xff]), false)
            .await
            .unwrap(),
        Outcome::Sent
    );
    assert!(matches!(next(shown).await, Message::Posted(n) if n.id == [3; 16] && n.icon.is_none()));

    // The PC withdraws notify/1 (focus mode, WN-45): nothing goes; back on, the icon is sent again.
    assert!(p.pc.set_accepted(Channel::Notify, false).await.unwrap());
    let deadline = Instant::now() + T;
    let mut id = 10;
    loop {
        id += 1;
        let outcome = sender
            .posted(note(id, "cicho"), Some(icon.clone()), false)
            .await
            .unwrap();
        if outcome == Outcome::NotAccepted {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the withdrawal reaches the phone"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(p.pc.set_accepted(Channel::Notify, true).await.unwrap());
    let deadline = Instant::now() + T;
    loop {
        id += 1;
        if sender
            .posted(note(id, "znów"), Some(icon.clone()), false)
            .await
            .unwrap()
            == Outcome::Sent
        {
            break;
        }
        assert!(Instant::now() < deadline, "back on reaches the phone");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Messages sent before the withdrawal reached the PC may still be queued; skip to the new ones.
    let mut got = Vec::new();
    loop {
        let m = next(shown).await;
        let done = matches!(&m, Message::Posted(n) if n.id == [id; 16]);
        got.push(m);
        if done {
            break;
        }
    }
    let n = got.len();
    assert_eq!(
        got[n - 2],
        Message::Icon { key, png: icon },
        "the icon again after re-enabling"
    );
}

#[tokio::test]
async fn nothing_without_the_capability() {
    let p = pair(false).await;
    let sender = Sender::new(p.phone.clone());
    assert_eq!(
        sender
            .posted(note(1, "x"), Some(PNG.to_vec()), false)
            .await
            .unwrap(),
        Outcome::NotAccepted
    );
    assert_eq!(
        sender.removed([1; 16]).await.unwrap(),
        Outcome::Dropped,
        "never went out"
    );
    drop(p.pc);
}
