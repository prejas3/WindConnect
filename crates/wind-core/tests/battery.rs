//! Battery status over real sessions (WN-49, PROTOCOL.md §9.4): the PC takes
//! `battery/1`, the phone sends only changes, in order; a PC without it gets nothing.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use wind_core::battery::{self, Sender, Status};
use wind_core::identity::DeviceIdentity;
use wind_core::protocol::{Capability, Channel, DeviceKind};
use wind_core::session::{ChannelMessage, LocalInfo, Router, Session};
use wind_core::transport::{Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(20);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: Some("test".into()),
    }
}

struct Pair {
    phone: Session,
    pc: Session,
    rx: Option<tokio::sync::mpsc::Receiver<ChannelMessage>>,
    _endpoints: (Endpoint, Endpoint),
}

/// A phone and a PC, paired; the PC routes `battery` when `pc_takes`.
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
    let mut router = Router::new();
    let rx = pc_takes.then(|| battery::route(&mut router));
    let server = tokio::spawn(Session::accept(s, info("PC"), router));
    let (phone, _) = Session::connect(c, info("Pixel"), Router::new())
        .await
        .unwrap();
    let (pc, _) = server.await.unwrap().unwrap();
    Pair {
        phone,
        pc,
        rx,
        _endpoints: (a, b),
    }
}

fn s(level: u8, charging: bool) -> Status {
    Status::new(level, charging).unwrap()
}

#[tokio::test]
async fn phone_sends_changes_only_in_order() {
    let mut p = pair(true).await;
    assert!(p
        .phone
        .peer_capabilities()
        .contains(&Capability::new("battery", 1).unwrap()));
    assert!(!p
        .pc
        .peer_capabilities()
        .iter()
        .any(|c| c.name() == "battery"));
    let sender = Sender::default();
    let sent: Vec<bool> = {
        let mut out = vec![];
        for st in [
            s(82, false),
            s(82, false),
            s(81, false),
            s(81, true),
            s(81, true),
            s(100, true),
        ] {
            out.push(sender.send(&p.phone, st).await.unwrap());
        }
        out
    };
    assert_eq!(sent, [true, false, true, true, false, true]);
    let rx = p.rx.as_mut().unwrap();
    let mut got = vec![];
    for _ in 0..4 {
        let m = tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap();
        assert_eq!(m.channel, Channel::Battery);
        got.push(Status::decode(&m.message).unwrap());
    }
    assert_eq!(got, [s(82, false), s(81, false), s(81, true), s(100, true)]);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), rx.recv())
            .await
            .is_err(),
        "nothing more"
    );
}

#[tokio::test]
async fn pc_without_battery_gets_nothing() {
    let p = pair(false).await;
    let sender = Sender::default();
    assert!(!sender.send(&p.phone, s(50, false)).await.unwrap());
    assert!(p
        .pc
        .peer_capabilities()
        .iter()
        .all(|c| c.name() != "battery"));
}
