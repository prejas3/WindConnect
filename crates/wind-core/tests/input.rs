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
