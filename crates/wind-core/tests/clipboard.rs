//! CCS/1 PURPOSE:real QUIC clipboard messages/ack, privacy policy, convergence, capability withdrawal and no echo.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use wind_core::{
    clipboard::{
        Clipboard, Connection, ImageError, IncomingImage, Message, OutgoingImage, Sink, Status,
    },
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
    image: Mutex<Option<Arc<IncomingImage>>>,
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
    fn apply_image(&self, image: Arc<IncomingImage>) -> bool {
        if *self.fail.lock().unwrap() {
            return false;
        }
        *self.image.lock().unwrap() = Some(image);
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
    pair_versions(ha, hb, 2).await
}
/// B routes clipboard up to `b_version` (1: an app from before images).
async fn pair_versions(ha: Arc<Clipboard>, hb: Arc<Clipboard>, b_version: u64) -> Pair {
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
    let qb = rb.route(Channel::Clipboard, b_version, 4);
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
    let text = "Zażółć 👩‍💻\nhttps://example.com/a?b=ż"; // i18n: data
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

/// Bytes that stand in for a PNG (the core doesn't decode images).
fn picture(size: usize, seed: u8) -> OutgoingImage {
    OutgoingImage {
        png: (0..size)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect(),
        thumbnail: vec![seed; 4096],
        width: 1920,
        height: 1080,
    }
}
fn received(memory: &Memory) -> Arc<IncomingImage> {
    memory
        .image
        .lock()
        .unwrap()
        .clone()
        .expect("an image on the clipboard")
}

/// WN-36: the receiver has the image on its clipboard (thumbnail, size)
/// before the PNG is in, the sender learns "pasteable" then "all there",
/// and a reader gets exactly the bytes sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn image_is_pasteable_first_and_complete_later() {
    let p = pair().await;
    let image = picture(3 * 1024 * 1024 + 17, 1);
    let (png, thumbnail) = (image.png.clone(), image.thumbnail.clone());
    let applied = Arc::new(Mutex::new(None));
    let at = applied.clone();
    let started = Instant::now();
    let status =
        p.a.send_image(image, false, false, move || {
            *at.lock().unwrap() = Some(started.elapsed())
        })
        .await;
    assert_eq!(status, Status::Applied, "the whole PNG arrived");
    assert!(
        applied.lock().unwrap().is_some(),
        "pasteable was reported first"
    );
    let got = received(&p.mb);
    assert_eq!(got.thumbnail(), &thumbnail[..]);
    assert_eq!(
        (got.width(), got.height(), got.size()),
        (1920, 1080, png.len() as u64)
    );
    assert!(got.is_complete());
    assert!(got.wait_complete(Duration::from_secs(1)).unwrap() == png);
    // The same PNG again is no news (hash memo).
    assert_eq!(
        p.a.send_image(picture(3 * 1024 * 1024 + 17, 1), false, false, || {})
            .await,
        Status::Unchanged
    );
    println!(
        "image 3 MiB: pasteable after {:?}, complete after {:?}",
        applied.lock().unwrap().unwrap(),
        started.elapsed()
    );
}

/// Limits: over the sender's own limit nothing goes; over the receiver's
/// limit it is refused with that limit (so the sender can say it), nothing
/// lands on the receiver, and a later small item still wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn image_limits_on_both_sides() {
    let p = pair().await;
    p.ha.set_image_limit(1_000_000);
    assert_eq!(
        p.a.send_image(picture(1_000_001, 2), false, true, || {})
            .await,
        Status::TooLarge
    );
    p.ha.set_image_limit(16_000_000);
    p.hb.set_image_limit(2_000_000);
    assert_eq!(
        p.a.send_image(picture(2_000_001, 3), false, true, || {})
            .await,
        Status::PeerLimit(2_000_000)
    );
    assert!(p.mb.image.lock().unwrap().is_none(), "nothing applied");
    assert_eq!(
        p.a.send_image(picture(2_000_000, 4), false, true, || {})
            .await,
        Status::Applied
    );
    assert_eq!(received(&p.mb).size(), 2_000_000);
    // Sensitive images never go, manual or not.
    assert_eq!(
        p.a.send_image(picture(10, 5), true, true, || {}).await,
        Status::Sensitive
    );
}

/// A newer item while an image is still coming: the receiver's readers end
/// with an error (never a mix of two images), the sender stops with
/// Replaced, and the newer item is what's on the clipboard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_newer_item_ends_an_image_still_arriving() {
    let p = pair().await;
    p.ha.set_image_limit(64 * 1024 * 1024);
    p.hb.set_image_limit(64 * 1024 * 1024);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let a = p.a.clone();
    let sending = tokio::spawn(async move {
        a.send_image(picture(60 * 1024 * 1024, 6), false, false, move || {
            let _ = tx.send(());
        })
        .await
    });
    rx.await.unwrap();
    let image = received(&p.mb);
    // A paste reading the image as it comes.
    let reader = {
        let image = image.clone();
        std::thread::spawn(move || image.wait_complete(Duration::from_secs(5)))
    };
    assert_eq!(
        p.b.send("newer text".into(), false, false).await,
        Status::Applied
    );
    assert_eq!(sending.await.unwrap(), Status::Replaced);
    assert_eq!(reader.join().unwrap(), Err(ImageError::Failed));
    assert!(image.is_failed() && !image.is_complete());
    assert!(p.hb.current_image().is_none());
    assert_eq!(*p.ma.value.lock().unwrap(), "newer text");
}

/// An image to a peer from before clipboard/2: not sent (Unsupported),
/// while text still works.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn images_need_clipboard_2() {
    let p = pair_versions(
        Arc::new(Clipboard::default()),
        Arc::new(Clipboard::default()),
        1,
    )
    .await;
    assert_eq!(
        p.a.send_image(picture(100, 7), false, true, || {}).await,
        Status::Unsupported
    );
    assert_eq!(p.a.send("text".into(), false, false).await, Status::Applied);
    assert!(p.mb.image.lock().unwrap().is_none());
}

/// The session ends mid-image: the reader on the receiver gets an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_closed_session_fails_an_unfinished_image() {
    let p = pair().await;
    p.ha.set_image_limit(64 * 1024 * 1024);
    p.hb.set_image_limit(64 * 1024 * 1024);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let a = p.a.clone();
    let sending = tokio::spawn(async move {
        a.send_image(picture(60 * 1024 * 1024, 8), false, false, move || {
            let _ = tx.send(());
        })
        .await
    });
    rx.await.unwrap();
    let image = received(&p.mb);
    p.sa.connection().quinn().close(0u32.into(), b"");
    assert_ne!(sending.await.unwrap(), Status::Applied);
    let result = tokio::task::spawn_blocking(move || image.wait_complete(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(result, Err(ImageError::Failed));
}
