//! Input over a session (WC-13): pointer and scroll as unreliable QUIC
//! datagrams, buttons on the `input` stream, the receiver in between.
//! The latency budget (< 5 ms p95) is in tests/input_latency.rs.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use wind_core::identity::DeviceIdentity;
use wind_core::input::InputEvent;
use wind_core::protocol::{Button, DeviceKind};
use wind_core::session::{LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Endpoint, EndpointOptions, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

#[tokio::test]
async fn keyboard_v3_real_session_and_old_peer_refusal() {
    use wind_core::protocol::{Channel, KeyboardAction};
    for version in [1, 2, 3] {
        let mut router = Router::new();
        let mut rx = router.route_input_version(version, 8);
        let (phone, pc) = pair(router, true).await;
        let input = phone.input().unwrap();
        let action = KeyboardAction::Text("Zażółć gęślą jaźń 😀".into());
        assert_eq!(input.supports_keyboard(), version == 3);
        if version < 3 {
            assert!(input.keyboard(action).await.is_err());
            assert!(rx.try_recv().is_err());
            continue;
        }
        input.pointer(123, 0).await.unwrap();
        input.keyboard(action.clone()).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Begin(0));
        assert!(matches!(
            next(&mut rx).await,
            InputEvent::Pointer { dx: 123, .. }
        ));
        assert_eq!(next(&mut rx).await, InputEvent::Keyboard(action));
        input
            .keyboard(KeyboardAction::Key {
                code: 67,
                modifiers: 1,
            })
            .await
            .unwrap();
        assert_eq!(
            next(&mut rx).await,
            InputEvent::Keyboard(KeyboardAction::Key {
                code: 67,
                modifiers: 1
            })
        );
        pc.set_accepted(Channel::Input, false).await.unwrap();
        tokio::time::timeout(T, async {
            while input.supports_keyboard() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(input
            .keyboard(KeyboardAction::Text("never replay".into()))
            .await
            .is_err());
    }
}

/// CCS/1 TEST:input/4 laser over a real session; only a v4 receiver is offered it.
#[tokio::test]
async fn laser_v4_real_session_and_old_peer_refusal() {
    for version in [3, 4] {
        let mut router = Router::new();
        let mut rx = router.route_input_version(version, 8);
        let (phone, _pc) = pair(router, true).await;
        let input = phone.input().unwrap();
        assert!(input.supports_keyboard());
        assert_eq!(input.supports_laser(), version == 4);
        if version < 4 {
            assert!(input.laser(true).await.is_err());
            assert!(rx.try_recv().is_err());
            continue;
        }
        input.laser(true).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Begin(0));
        assert_eq!(next(&mut rx).await, InputEvent::Laser(true));
        input.pointer(250, -40).await.unwrap();
        assert!(matches!(
            next(&mut rx).await,
            InputEvent::Pointer {
                dx: 250,
                dy: -40,
                ..
            }
        ));
        input.laser(false).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Laser(false));
    }
}

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

/// CCS/1 RULE:WN-51 integration keeps gallery/1 alongside input/1 and input/2 in both router and session negotiation.
#[tokio::test]
async fn gallery_and_touchpad_are_negotiated_together() {
    use wind_core::protocol::{ErrorCode, Gesture};
    let mut router = Router::new();
    let mut input_events = router.route_input_version(2, 8);
    let mut gallery_streams = router.route_gallery(4);
    let names = |caps: Vec<wind_core::protocol::Capability>| {
        caps.into_iter().map(|c| c.to_string()).collect::<Vec<_>>()
    };
    for name in ["gallery/1", "input/1", "input/2"] {
        assert!(names(router.capabilities()).iter().any(|c| c == name));
    }
    let (phone, pc) = pair(router, true).await;
    for caps in [phone.peer_capabilities(), pc.accepted_capabilities()] {
        for name in ["gallery/1", "input/1", "input/2"] {
            assert!(names(caps.clone()).iter().any(|c| c == name));
        }
    }
    let input = phone.input().unwrap();
    assert!(input.supports_gestures());
    input.gesture(Gesture::TaskView).await.unwrap();
    assert_eq!(next(&mut input_events).await, InputEvent::Begin(0));
    assert_eq!(
        next(&mut input_events).await,
        InputEvent::Gesture(Gesture::TaskView)
    );
    let gallery = phone.open_gallery().await.unwrap();
    let received = tokio::time::timeout(T, gallery_streams.recv())
        .await
        .unwrap()
        .unwrap();
    drop((gallery, received));
    phone
        .connection()
        .close(ErrorCode::GoingAway, b"integration test complete");
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
            InputEvent::Gesture(_)
            | InputEvent::Keyboard(_)
            | InputEvent::Laser(_)
            | InputEvent::Begin(_)
            | InputEvent::End(_)
            | InputEvent::Withdrawn => {
                panic!("input/1 emitted a v2 event")
            }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_gestures_with_datagrams_and_stream_fallback() {
    use wind_core::protocol::Gesture;
    for datagrams in [true, false] {
        let mut router = Router::new();
        let mut rx = router.route_input_version(2, 64);
        let (phone, pc) = pair(router, datagrams).await;
        let input = phone.input().unwrap();
        assert!(input.supports_gestures());
        input.pointer(123, 456).await.unwrap();
        input.gesture(Gesture::ShowDesktop).await.unwrap();
        let mut pointer = (0, 0);
        assert_eq!(next(&mut rx).await, InputEvent::Begin(0));
        loop {
            match next(&mut rx).await {
                InputEvent::Pointer { dx, dy, .. } => {
                    pointer.0 += dx;
                    pointer.1 += dy;
                }
                InputEvent::Gesture(Gesture::ShowDesktop) => break,
                event => panic!("unexpected {event:?}"),
            }
        }
        assert_eq!(pointer, (123, 456));
        pc.set_accepted(wind_core::protocol::Channel::Input, false)
            .await
            .unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::End(0));
        assert_eq!(next(&mut rx).await, InputEvent::Withdrawn);
        tokio::time::timeout(T, async {
            while input.supports_gestures() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(input.gesture(Gesture::TaskView).await.is_err());
    }
}

/// CCS/1 PERF:explicit software proof; real authenticated QUIC, common PC interpolator, one monotonic clock. No OS input injection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes latency CSV to WIND_TOUCHPAD_SAMPLES; run explicitly on an idle machine"]
async fn touchpad_latency_with_interpolation_idle_and_bulk() {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };
    use std::time::Instant;
    use wind_core::protocol::cbor::Value;
    use wind_core::{
        input::Interpolator,
        protocol::{Channel, RawMessage},
        session::write_raw,
    };
    let root = std::env::var_os("WIND_TOUCHPAD_SAMPLES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&root).unwrap();
    for bulk in [false, true] {
        const COUNT: usize = 360;
        let mut router = Router::new();
        let mut rx = router.route_input_version(2, 128);
        let mut files = router.route_files(1, 4);
        let (phone, pc) = pair(router, true).await;
        let running = Arc::new(AtomicBool::new(true));
        let producer = if bulk {
            let phone = phone.clone();
            let running = running.clone();
            Some(tokio::spawn(async move {
                let mut stream = phone.open_files().await.unwrap();
                let message = RawMessage::new(
                    0,
                    Value::Map(vec![(Value::Int(0), Value::Bytes(vec![0x5a; 64 * 1024]))]),
                );
                let mut bytes = 0;
                while running.load(Ordering::SeqCst) {
                    write_raw(&mut stream.send, Channel::Files, &message)
                        .await
                        .unwrap();
                    bytes += 64 * 1024;
                }
                stream.send.finish().unwrap();
                bytes
            }))
        } else {
            None
        };
        let consumer = if bulk {
            Some(tokio::spawn(async move {
                let mut stream = files.recv().await.unwrap();
                while stream.recv.next_raw().await.unwrap().is_some() {}
            }))
        } else {
            None
        };
        let sent = Arc::new(Mutex::new(VecDeque::<Instant>::new()));
        let stamps = sent.clone();
        let receiving = tokio::spawn(async move {
            let mut smooth = Interpolator::default();
            let mut deadline = None;
            let mut samples = Vec::new();
            while samples.len() < COUNT {
                let events = tokio::select! {
                    event=rx.recv()=>{let event=event.unwrap();let events=smooth.push(event);if smooth.pending()&&deadline.is_none(){deadline=Some(tokio::time::Instant::now()+Duration::from_micros(8_333));}events}
                    _=async{match deadline{Some(at)=>tokio::time::sleep_until(at).await,None=>std::future::pending().await}}=>{deadline=None;smooth.flush()}
                };
                for event in events {
                    if let InputEvent::Pointer { dx, .. } = event {
                        let now = Instant::now();
                        for _ in 0..dx {
                            samples.push(
                                now.duration_since(stamps.lock().unwrap().pop_front().unwrap())
                                    .as_micros() as u64,
                            );
                        }
                    }
                }
            }
            samples
        });
        let input = phone.input().unwrap();
        let mut clock = tokio::time::interval(Duration::from_nanos(1_000_000_000 / 120));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        for _ in 0..COUNT {
            clock.tick().await;
            sent.lock().unwrap().push_back(Instant::now());
            input.pointer(1, 0).await.unwrap();
        }
        let samples = tokio::time::timeout(T, receiving).await.unwrap().unwrap();
        input.end_session().await;
        running.store(false, Ordering::SeqCst);
        let bytes = if let Some(task) = producer {
            task.await.unwrap()
        } else {
            0
        };
        if let Some(task) = consumer {
            task.await.unwrap();
        }
        let mode = if bulk { "bulk" } else { "idle" };
        let mut csv = String::from("sample,latency_us\n");
        for (i, t) in samples.iter().enumerate() {
            csv.push_str(&format!("{i},{t}\n"));
        }
        std::fs::write(root.join(format!("wn51-{mode}.csv")), csv).unwrap();
        let mut sorted = samples;
        sorted.sort_unstable();
        let p95 = sorted[(COUNT * 95).div_ceil(100) - 1];
        println!("WN51 {mode} interpolation=ON samples={COUNT} p95_us={p95} max_us={} bulk_bytes={bytes}",sorted[COUNT-1]);
        assert!(p95 < 20_000, "software p95 must fit 20 ms");
        drop(pc);
    }
}
