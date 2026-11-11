//! CCS/1 PURPOSE:real QUIC clipboard messages/ack, privacy policy, convergence, capability withdrawal and no echo.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use wind_core::{
    clipboard::{Clipboard, Connection, Message, Sink, Status},
    identity::DeviceIdentity,
    protocol::{Channel, DeviceKind, ErrorCode},
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
#[derive(Default)]
struct Memory {
    value: Mutex<String>,
    writes: Mutex<usize>,
    fail: Mutex<bool>,
    delay: Mutex<Duration>,
}
impl Sink for Memory {
    fn apply(&self, text: &str) -> bool {
        std::thread::sleep(*self.delay.lock().unwrap());
        if *self.fail.lock().unwrap() {
            return false;
        }
        *self.value.lock().unwrap() = text.into();
        *self.writes.lock().unwrap() += 1;
        true
    }
}
struct Pair {
    a: Arc<Connection>,
    b: Arc<Connection>,
    sa: Session,
    sb: Session,
    ha: Arc<Clipboard>,
    hb: Arc<Clipboard>,
    ma: Arc<Memory>,
    mb: Arc<Memory>,
    _endpoints: (Endpoint, Endpoint),
}
async fn pair() -> Pair {
    pair_hubs(
        Arc::new(Clipboard::default()),
        Arc::new(Clipboard::default()),
    )
    .await
}
async fn pair_hubs(ha: Arc<Clipboard>, hb: Arc<Clipboard>) -> Pair {
    let ia = DeviceIdentity::generate().unwrap();
    let ib = DeviceIdentity::generate().unwrap();
    let pa = Arc::new(PinnedKeys::new());
    let pb = Arc::new(PinnedKeys::new());
    pa.pin(ib.public_key());
    pb.pin(ia.public_key());
    let a = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &ia, pa).unwrap();
    let b = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &ib, pb).unwrap();
    let accepting = b.clone();
    let accept = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
    let client = a
        .connect(b.local_addr().unwrap(), ib.public_key())
        .await
        .unwrap();
    let server = accept.await.unwrap();
    let info = |name: &str| LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: None,
    };
    let mut ra = Router::new();
    let mut rb = Router::new();
    let qa = Clipboard::route(&mut ra);
    let qb = Clipboard::route(&mut rb);
    let accepting = tokio::spawn(Session::accept(server, info("B"), rb));
    let (sa, _) = Session::connect(client, info("A"), ra).await.unwrap();
    let (sb, _) = accepting.await.unwrap().unwrap();
    let (ma, mb) = (Arc::new(Memory::default()), Arc::new(Memory::default()));
    ha.set_sink(ma.clone());
    hb.set_sink(mb.clone());
    Pair {
        a: ha.attach(sa.clone(), qa),
        b: hb.attach(sb.clone(), qb),
        sa,
        sb,
        ha,
        hb,
        ma,
        mb,
        _endpoints: (a, b),
    }
}
#[tokio::test]
async fn reconnect_after_peer_restart_accepts_new_changes_and_stops_old_session() {
    let old = pair().await;
    for index in 0..5 {
        assert_eq!(
            old.b.send(format!("old-{index}"), false, false).await,
            Status::Applied
        );
    }
    let next = pair_hubs(old.ha.clone(), Arc::new(Clipboard::default())).await;
    assert_eq!(*next.ma.writes.lock().unwrap(), 0, "no automatic replay");
    assert_eq!(
        next.b.send("new after restart".into(), false, false).await,
        Status::Applied
    );
    assert_eq!(
        old.a.send("stale binding".into(), false, true).await,
        Status::Unavailable
    );
    let _ = old
        .sb
        .send(
            Channel::Clipboard,
            &Message::Set {
                key: (999, [9; 16]),
                text: "late old session".into(),
            }
            .encode(),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(*next.ma.value.lock().unwrap(), "new after restart");
    assert_eq!(*next.ma.writes.lock().unwrap(), 1);
}
#[tokio::test]
async fn real_bidirectional_text_links_and_echo() {
    let p = pair().await;
    let text = "Zażółć 👩‍💻\nhttps://example.com/a?b=ż";
    assert_eq!(p.a.send(text.into(), false, false).await, Status::Applied);
    assert_eq!(*p.mb.value.lock().unwrap(), text);
    assert_eq!(p.b.send(text.into(), false, false).await, Status::Unchanged);
    assert_eq!(*p.ma.writes.lock().unwrap(), 0);
    assert_eq!(p.a.send(text.into(), false, false).await, Status::Unchanged);
    assert_eq!(p.a.send(text.into(), false, true).await, Status::Applied);
    assert_eq!(
        p.b.send("z telefonu".into(), false, false).await,
        Status::Applied
    );
    assert_eq!(*p.ma.value.lock().unwrap(), "z telefonu");
}
#[tokio::test]
async fn sensitive_never_sends_including_manual() {
    let p = pair().await;
    for manual in [false, true] {
        assert_eq!(
            p.a.send("test password".into(), true, manual).await,
            Status::Sensitive
        );
    }
    assert_eq!(*p.mb.writes.lock().unwrap(), 0);
}
#[tokio::test]
async fn reenabling_receiver_preserves_lamport_order_without_replay() {
    let p = pair().await;
    p.hb.set_enabled(false);
    for index in 0..5 {
        assert_eq!(
            p.a.send(format!("not applied-{index}"), false, false).await,
            Status::Refused
        );
    }
    assert_eq!(*p.mb.writes.lock().unwrap(), 0);
    p.hb.set_enabled(true);
    assert_eq!(
        p.b.send("new local after enabling".into(), false, false)
            .await,
        Status::Applied
    );
    assert_eq!(*p.ma.value.lock().unwrap(), "new local after enabling");
}
#[tokio::test]
async fn disabled_and_failed_receiver_never_claim_applied() {
    let p = pair().await;
    p.hb.set_enabled(false);
    assert_eq!(p.a.send("one".into(), false, false).await, Status::Refused);
    assert_eq!(*p.mb.writes.lock().unwrap(), 0);
    p.hb.set_enabled(true);
    *p.mb.fail.lock().unwrap() = true;
    assert_eq!(p.a.send("two".into(), false, false).await, Status::Failed);
    p.ha.set_enabled(false);
    assert_eq!(
        p.a.send("three".into(), false, false).await,
        Status::Disabled
    );
    *p.mb.fail.lock().unwrap() = false;
    assert_eq!(p.a.send("three".into(), false, true).await, Status::Applied);
}
#[tokio::test]
async fn simultaneous_updates_converge_without_a_clock_or_echo() {
    let p = pair().await;
    *p.ma.value.lock().unwrap() = "A".into();
    *p.mb.value.lock().unwrap() = "B".into();
    let (a, b) = tokio::join!(
        p.a.send("A".into(), false, false),
        p.b.send("B".into(), false, false)
    );
    assert!(matches!(
        (a, b),
        (Status::Applied, Status::Refused) | (Status::Refused, Status::Applied)
    ));
    assert_eq!(*p.ma.value.lock().unwrap(), *p.mb.value.lock().unwrap());
    let value = p.ma.value.lock().unwrap().clone();
    assert_eq!(
        p.a.send(value.clone(), false, false).await,
        Status::Unchanged
    );
    assert_eq!(p.b.send(value, false, false).await, Status::Unchanged);
}
#[tokio::test]
async fn ack_waits_for_os_sink_and_close_ends_requests() {
    let p = pair().await;
    *p.mb.delay.lock().unwrap() = Duration::from_millis(100);
    let start = Instant::now();
    assert_eq!(
        p.a.send("slow OS".into(), false, false).await,
        Status::Applied
    );
    assert!(start.elapsed() >= Duration::from_millis(100));
    p.sb.connection().close(ErrorCode::GoingAway, b"test");
    p.sa.closed().await;
    assert_eq!(
        p.a.send("after disconnect".into(), false, true).await,
        Status::Unavailable
    );
}
#[tokio::test]
async fn duplicate_stale_invalid_and_withdrawn_sets_do_not_apply() {
    let p = pair().await;
    let set = Message::Set {
        key: (7, [1; 16]),
        text: "first".into(),
    }
    .encode();
    p.sa.send(Channel::Clipboard, &set).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while *p.mb.writes.lock().unwrap() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    p.sa.send(Channel::Clipboard, &set).await.unwrap();
    p.sa.send(
        Channel::Clipboard,
        &Message::Set {
            key: (6, [2; 16]),
            text: "old".into(),
        }
        .encode(),
    )
    .await
    .unwrap();
    p.sa.send(
        Channel::Clipboard,
        &Message::Set {
            key: (8, [3; 16]),
            text: "bad\0value".into(),
        }
        .encode(),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(*p.mb.writes.lock().unwrap(), 1);
    p.sb.set_accepted(Channel::Clipboard, false).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while p
            .sa
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "clipboard")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        p.a.send("withdrawn".into(), false, true).await,
        Status::Unavailable
    );
}
