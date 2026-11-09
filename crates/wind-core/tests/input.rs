//! Input over a session (WC-13): pointer and scroll as unreliable QUIC
//! datagrams, buttons on the `input` stream, the receiver in between.
//! Acceptance: datagram latency < 5 ms p95.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use wind_core::identity::DeviceIdentity;
use wind_core::input::InputEvent;
use wind_core::protocol::cbor::Value;
use wind_core::protocol::{Button, Channel, DeviceKind, RawMessage};
use wind_core::session::{write_raw, LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Endpoint, EndpointOptions, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Phone,
        app: None,
    }
}

/// A session pair: the client sends input; the server (whose router is
/// given) receives it. `server_datagrams`: whether the server accepts
/// datagrams.
async fn pair(server_router: Router, server_datagrams: bool) -> (Session, Session) {
    let (ic, is) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pc, ps) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pc.pin(is.public_key());
    ps.pin(ic.public_key());
    let client = Endpoint::bind(LOCALHOST, &ic, pc).unwrap();
    let server = Endpoint::bind_with(
        LOCALHOST,
        &is,
        ps,
        EndpointOptions {
            datagrams: server_datagrams,
        },
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    let accept = {
        let server = server.clone();
        tokio::spawn(async move {
            let conn = server.accept().await.unwrap().unwrap();
            Session::accept(conn, info("pc"), server_router)
                .await
                .unwrap()
                .0
        })
    };
    let (c, _e) = Session::dial(&client, addr, is.public_key(), info("phone"), Router::new())
        .await
        .unwrap();
    let s = accept.await.unwrap();
    // Keep the endpoints for the test's duration.
    std::mem::forget((client, server));
    (c, s)
}

async fn next(rx: &mut mpsc::Receiver<InputEvent>) -> InputEvent {
    tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap()
}

/// Folds events into (pointer, scroll, buttons), until `done` holds.
async fn collect(
    rx: &mut mpsc::Receiver<InputEvent>,
    mut done: impl FnMut(&(i64, i64), &(i64, i64), &[(Button, bool, (i64, i64))]) -> bool,
) -> ((i64, i64), (i64, i64), Vec<(Button, bool, (i64, i64))>) {
    let (mut p, mut s, mut b) = ((0, 0), (0, 0), Vec::new());
    while !done(&p, &s, &b) {
        match next(rx).await {
            InputEvent::Pointer { dx, dy, .. } => p = (p.0 + dx, p.1 + dy),
            InputEvent::Scroll { dx, dy, .. } => s = (s.0 + dx, s.1 + dy),
            InputEvent::Button { button, down } => b.push((button, down, p)),
        }
    }
    (p, s, b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drag_and_scroll_arrive_as_the_sender_made_them() {
    let mut router = Router::new();
    let mut rx = router.route_input(256);
    let (phone, _pc) = pair(router, true).await;
    let input = phone.input().unwrap();
    assert!(input.uses_datagrams());

    input.pointer(100, 20).await.unwrap();
    input.button(Button::Left, true).await.unwrap();
    for _ in 0..10 {
        input.pointer(10, 0).await.unwrap();
    }
    input.button(Button::Left, false).await.unwrap();
    input.scroll(0, 240).await.unwrap();
    input.pointer(-50, 5).await.unwrap();

    let (p, s, b) = collect(&mut rx, |p, s, b| {
        b.len() == 2 && *p == (150, 25) && *s == (0, 240)
    })
    .await;
    assert_eq!(
        b,
        [
            (Button::Left, true, (100, 20)),
            (Button::Left, false, (200, 20)),
        ]
    );
    assert_eq!((p, s), ((150, 25), (0, 240)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_datagrams_motion_goes_on_the_stream() {
    // §8.7: the receiver doesn't accept datagrams, so everything goes on
    // the stream, in order.
    let mut router = Router::new();
    let mut rx = router.route_input(256);
    let (phone, _pc) = pair(router, false).await;
    let input = phone.input().unwrap();
    assert!(!input.uses_datagrams());
    input.pointer(5, 5).await.unwrap();
    input.button(Button::Right, true).await.unwrap();
    input.pointer(5, 0).await.unwrap();
    input.button(Button::Right, false).await.unwrap();
    let (p, _s, b) = collect(&mut rx, |p, _, b| b.len() == 2 && *p == (10, 5)).await;
    assert_eq!(p, (10, 5));
    assert_eq!(
        b,
        [
            (Button::Right, true, (5, 5)),
            (Button::Right, false, (10, 5))
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pressed_button_is_released_when_the_connection_ends() {
    let mut router = Router::new();
    let mut rx = router.route_input(64);
    let (phone, _pc) = pair(router, true).await;
    let input = phone.input().unwrap();
    input.button(Button::Left, true).await.unwrap();
    assert_eq!(
        next(&mut rx).await,
        InputEvent::Button {
            button: Button::Left,
            down: true
        }
    );
    phone.connection().quinn().close(0u32.into(), b"");
    assert_eq!(
        next(&mut rx).await,
        InputEvent::Button {
            button: Button::Left,
            down: false
        }
    );
}

#[tokio::test]
async fn no_input_without_the_capability() {
    let (phone, _pc) = pair(Router::new(), true).await;
    assert!(matches!(
        phone.input(),
        Err(SessionError::CapabilityMissing(_))
    ));
}

/// Acceptance (WC-13): a pointer datagram reaches the receiving module in
/// < 5 ms p95. Each move adds 1 to x, so the running x says which move an
/// event completes; latency is measured on one clock (same process).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datagram_latency_is_under_5_ms_p95() {
    latency_test(false).await;
}

/// The same while a large `files` transfer fills the connection: input
/// datagrams must not queue behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn datagram_latency_stays_under_5_ms_p95_during_a_transfer() {
    latency_test(true).await;
}

async fn latency_test(with_transfer: bool) {
    const MOVES: usize = 300;
    let mut router = Router::new();
    let mut rx = router.route_input(4096);
    let mut files = router.route_files(1, 4);
    let (phone, _pc) = pair(router, true).await;
    let input = phone.input().unwrap();
    assert!(input.uses_datagrams());

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let moved = Arc::new(std::sync::atomic::AtomicU64::new(0));
    if with_transfer {
        let sink_moved = moved.clone();
        tokio::spawn(async move {
            let mut s = files.recv().await.unwrap();
            while let Ok(Some(m)) = s.recv.next_raw().await {
                if let Value::Map(p) = &m.body {
                    if let Value::Bytes(b) = &p[0].1 {
                        sink_moved.fetch_add(b.len() as u64, Ordering::Relaxed);
                    }
                }
            }
        });
        let mut out = phone.open_files().await.unwrap();
        let chunk = RawMessage::new(
            0,
            Value::Map(vec![(Value::Int(0), Value::Bytes(vec![7; 1 << 19]))]),
        );
        let stop = stop.clone();
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) {
                if write_raw(&mut out.send, Channel::Files, &chunk)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = out.send.finish();
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    let moved_before = moved.load(Ordering::Relaxed);

    let sent_at: Arc<std::sync::Mutex<Vec<Option<Instant>>>> =
        Arc::new(std::sync::Mutex::new(vec![None; MOVES + 1]));
    let receiver = {
        let sent_at = sent_at.clone();
        tokio::spawn(async move {
            let mut x = 0i64;
            let mut latencies = Vec::with_capacity(MOVES);
            let mut skipped = 0usize;
            while (x as usize) < MOVES {
                if let InputEvent::Pointer { dx, .. } = next(&mut rx).await {
                    let now = Instant::now();
                    skipped += (dx - 1) as usize; // moves folded in (lost datagrams)
                    x += dx;
                    let at = sent_at.lock().unwrap()[x as usize].expect("sent");
                    latencies.push(now.duration_since(at).as_secs_f64() * 1000.0);
                }
            }
            (latencies, skipped)
        })
    };
    // A finger on a 240 Hz touch screen: a move about every 4 ms.
    for i in 1..=MOVES {
        sent_at.lock().unwrap()[i] = Some(Instant::now());
        input.pointer(1, 0).await.unwrap();
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
    let (mut lat, skipped) = tokio::time::timeout(T, receiver).await.unwrap().unwrap();
    stop.store(true, Ordering::Relaxed);
    let transferred = moved.load(Ordering::Relaxed) - moved_before;
    if with_transfer {
        println!("transfer moved {} MB meanwhile", transferred / 1_000_000);
        assert!(transferred > 20_000_000, "the transfer barely ran");
    }
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| lat[((lat.len() - 1) as f64 * p).round() as usize];
    println!(
        "datagram latency over {} moves: p50 {:.3} ms, p95 {:.3} ms, p99 {:.3} ms, max {:.3} ms; \
         {skipped} moves folded into later datagrams",
        lat.len(),
        pct(0.5),
        pct(0.95),
        pct(0.99),
        pct(1.0)
    );
    assert!(pct(0.95) < 5.0, "p95 {:.3} ms", pct(0.95));
}
