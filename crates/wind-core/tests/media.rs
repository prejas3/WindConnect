//! Media over real sessions (WN-46, PROTOCOL.md §9.6): the phone's player shown on the PC
//! (media/1) and controlled from it (media-control/1); nothing without the capability.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use wind_core::identity::DeviceIdentity;
use wind_core::media::{self, allow, art_key, Action, Message, Now, Sender};
use wind_core::protocol::{Channel, DeviceKind};
use wind_core::session::{ChannelMessage, LocalInfo, Router, Session};
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

struct Pair {
    pc: Session,
    phone: Session,
    /// The PC's queue of the phone's player (media/1).
    shown: Option<Receiver<ChannelMessage>>,
    /// The phone's queue of commands (media-control/1).
    commands: Option<Receiver<ChannelMessage>>,
    _endpoints: (Endpoint, Endpoint),
}

async fn pair(pc_shows: bool, phone_controlled: bool) -> Pair {
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
    let shown = pc_shows.then(|| media::route_player(&mut pc_router));
    let server = tokio::spawn(Session::accept(s, info("PC"), pc_router));
    let mut phone_router = Router::new();
    let commands = phone_controlled.then(|| media::route_control(&mut phone_router));
    let (phone, _) = Session::connect(c, info("Pixel"), phone_router)
        .await
        .unwrap();
    let (pc, _) = server.await.unwrap().unwrap();
    Pair {
        pc,
        phone,
        shown,
        commands,
        _endpoints: (a, b),
    }
}

fn song(position_ms: u64) -> Now {
    Now {
        title: "Kaonashi".into(),
        artist: Some("Daria Zawiałow".into()),
        album: None,
        app: "Spotify".into(),
        duration_ms: Some(200_000),
        position_ms,
        playing: true,
        speed_permille: 1000,
        actions: allow::PLAY | allow::PAUSE | allow::NEXT,
        art: None,
    }
}

async fn next(rx: &mut Receiver<ChannelMessage>) -> ChannelMessage {
    tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn player_shown_and_controlled() {
    let mut p = pair(true, true).await;
    let has = |s: &Session, name: &str| s.peer_capabilities().iter().any(|c| c.name() == name);
    assert!(
        has(&p.phone, "media") && !has(&p.phone, "media-control"),
        "the PC shows"
    );
    assert!(
        has(&p.pc, "media-control") && !has(&p.pc, "media"),
        "the phone is controlled"
    );

    let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 9, 9];
    let sender = Sender::default();
    assert!(sender
        .now(&p.phone, song(30_000), Some(&jpeg))
        .await
        .unwrap());
    // Only the position moved, as expected: nothing sent.
    assert!(!sender
        .now(&p.phone, song(30_500), Some(&jpeg))
        .await
        .unwrap());
    // Paused: a change, the art isn't sent again.
    let paused = Now {
        playing: false,
        ..song(31_000)
    };
    assert!(sender
        .now(&p.phone, paused.clone(), Some(&jpeg))
        .await
        .unwrap());
    assert!(sender.gone(&p.phone).await.unwrap());
    assert!(!sender.gone(&p.phone).await.unwrap(), "once");

    let shown = p.shown.as_mut().unwrap();
    let got: Vec<Message> = {
        let mut v = vec![];
        for _ in 0..4 {
            v.push(Message::decode(&next(shown).await.message).unwrap());
        }
        v
    };
    assert_eq!(
        got[0],
        Message::Art {
            key: art_key(&jpeg),
            image: jpeg.clone()
        }
    );
    assert_eq!(
        got[1],
        Message::Now(Now {
            art: Some(art_key(&jpeg)),
            ..song(30_000)
        })
    );
    assert_eq!(
        got[2],
        Message::Now(Now {
            art: Some(art_key(&jpeg)),
            ..paused
        })
    );
    assert_eq!(got[3], Message::Gone);

    for action in [Action::Toggle, Action::Next, Action::Seek(61_000)] {
        media::command(&p.pc, action).await.unwrap();
        let m = next(p.commands.as_mut().unwrap()).await;
        assert_eq!(m.channel, Channel::MediaControl);
        assert_eq!(Action::decode(&m.message), Some(action));
    }
}

#[tokio::test]
async fn nothing_without_the_capability() {
    let p = pair(false, false).await;
    let sender = Sender::default();
    assert!(
        !sender.now(&p.phone, song(0), None).await.unwrap(),
        "the PC doesn't show players"
    );
    assert!(matches!(
        media::command(&p.pc, Action::Play).await,
        Err(wind_core::session::SessionError::CapabilityMissing(
            Channel::MediaControl
        ))
    ));
}

#[tokio::test]
async fn control_started_disabled_then_allowed() {
    let mut router = Router::new();
    let _ = media::route_control(&mut router);
    router.start_disabled(Channel::MediaControl);
    assert!(router
        .capabilities()
        .iter()
        .all(|c| c.name() != "media-control"));
}
