//! Input against a hand-driven peer (WC-13): event order under
//! backpressure, and a sender that follows STOP_SENDING and capability
//! changes.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use wind_core::identity::DeviceIdentity;
use wind_core::input::InputEvent;
use wind_core::protocol::{
    decode_datagram, encode_datagram, encode_frame, stream_header, Button, Capability, Channel,
    ControlMessage, DeviceKind, ErrorCode, Hello, InputMessage, InputSession, Message, Motion,
    PointerButton, Version, MAX_DATAGRAM,
};
use wind_core::session::{FramedRecv, LocalInfo, Router, Session, SessionError};
use wind_core::transport::{Connection, Endpoint, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: None,
    }
}

fn hello(caps: &[&str]) -> Hello {
    Hello {
        version: Version { major: 1, minor: 0 },
        name: "raw".into(),
        kind: DeviceKind::Phone,
        caps: caps.iter().map(|c| Capability::parse(c).unwrap()).collect(),
        app: None,
    }
}

fn frame(m: Message) -> Vec<u8> {
    encode_frame(&m).unwrap()
}

/// Connected endpoints: (client side, server side).
async fn connections() -> (Connection, Connection) {
    let (ia, ib) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let (pa, pb) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pa.pin(ib.public_key());
    pb.pin(ia.public_key());
    let a = Endpoint::bind(LOCALHOST, &ia, pa).unwrap();
    let b = Endpoint::bind(LOCALHOST, &ib, pb).unwrap();
    let addr = b.local_addr().unwrap();
    let accept = {
        let b = b.clone();
        tokio::spawn(async move { b.accept().await.unwrap().unwrap() })
    };
    let client = a.connect(addr, ib.public_key()).await.unwrap();
    let server = accept.await.unwrap();
    std::mem::forget((a, b));
    (client, server)
}

async fn next(rx: &mut mpsc::Receiver<InputEvent>) -> InputEvent {
    tokio::time::timeout(T, rx.recv()).await.unwrap().unwrap()
}

fn motion(seq: u64, x: i64, b: u64) -> Vec<u8> {
    encode_datagram(&InputMessage::PointerMotion(Motion {
        seq,
        x,
        y: 0,
        t: None,
        s: 0,
        b,
    }))
    .unwrap()
}

fn ptr(dx: i64) -> InputEvent {
    InputEvent::Pointer { dx, dy: 0, t: None }
}

fn btn(down: bool) -> InputEvent {
    InputEvent::Button {
        button: Button::Left,
        down,
    }
}

/// A raw sender (client) against a real session (server) whose input
/// queue holds one event. Returns the raw connection, its `input` stream
/// (session 0 opened) and the module's queue.
async fn raw_sender() -> (
    Connection,
    quinn::SendStream,
    mpsc::Receiver<InputEvent>,
    Session,
) {
    let (c, s) = connections().await;
    let mut router = Router::new();
    let rx = router.route_input(1);
    let server = tokio::spawn(Session::accept(s, info("pc"), router));
    let (mut send, recv) = c.quinn().open_bi().await.unwrap();
    let mut out = stream_header(Channel::Control).to_vec();
    out.extend(frame(Message::Control(ControlMessage::Hello(hello(&[])))));
    send.write_all(&out).await.unwrap();
    let mut recv = FramedRecv::new(recv, Channel::Control);
    assert!(matches!(
        recv.next_message().await.unwrap(),
        Some(Message::Control(ControlMessage::HelloAck(_)))
    ));
    std::mem::forget((send, recv));
    let (session, _e) = server.await.unwrap().unwrap();
    let mut input = c.quinn().open_uni().await.unwrap();
    let mut out = stream_header(Channel::Input).to_vec();
    out.extend(frame(Message::Input(InputMessage::InputSession(
        InputSession { s: 0, g: 0 },
    ))));
    input.write_all(&out).await.unwrap();
    (c, input, rx, session)
}

/// Codex's case: the module's queue (capacity 1) is full; the stream
/// brings a button whose pre-button motion was lost, then a datagram
/// overtakes it. The press must land at x=100, then the move to x=200.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_keep_the_state_machines_order_under_backpressure() {
    let (c, mut input, mut rx, _server) = raw_sender().await;
    c.quinn().send_datagram(motion(1, 10, 0).into()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await; // +10 now fills the queue
    let press = frame(Message::Input(InputMessage::PointerButton(PointerButton {
        seq: 2,
        button: Button::Left,
        down: true,
        x: 100,
        y: 0,
        b: 1,
        sx: 0,
        sy: 0,
    })));
    input.write_all(&press).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await; // [+90, down] waits
    c.quinn().send_datagram(motion(3, 200, 1).into()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut got = Vec::new();
    for _ in 0..4 {
        got.push(next(&mut rx).await);
    }
    assert_eq!(got, [ptr(10), ptr(90), btn(true), ptr(100)]);
}

/// The release at the end of the connection comes after everything queued
/// before it, however slowly the module reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_at_connection_end_comes_last_under_backpressure() {
    let (c, mut input, mut rx, _server) = raw_sender().await;
    let press = frame(Message::Input(InputMessage::PointerButton(PointerButton {
        seq: 1,
        button: Button::Left,
        down: true,
        x: 0,
        y: 0,
        b: 1,
        sx: 0,
        sy: 0,
    })));
    input.write_all(&press).await.unwrap();
    for (i, x) in (1..=5).zip([10, 20, 30, 40, 50]) {
        c.quinn().send_datagram(motion(1 + i, x, 1).into()).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.quinn().close(0u32.into(), b"");

    let mut got = Vec::new();
    loop {
        let e = next(&mut rx).await;
        got.push(e);
        if e == btn(false) {
            break;
        }
    }
    assert_eq!(got.first(), Some(&btn(true)));
    assert_eq!(got.iter().filter(|e| **e == btn(false)).count(), 1);
    let moved: i64 = got
        .iter()
        .map(|e| match e {
            InputEvent::Pointer { dx, .. } => *dx,
            _ => 0,
        })
        .sum();
    assert_eq!(moved, 50, "{got:?}");
}

/// A raw receiver (server) announcing `input/1`, driving a real session's
/// sender (client).
struct RawReceiver {
    conn: Connection,
    control: quinn::SendStream,
}

async fn raw_receiver() -> (RawReceiver, Session) {
    let (c, s) = connections().await;
    let server = tokio::spawn(async move {
        let (mut send, mut recv) = s.quinn().accept_bi().await.unwrap();
        let mut header = [0u8; 2];
        recv.read_exact(&mut header).await.unwrap();
        let mut recv = FramedRecv::new(recv, Channel::Control);
        recv.next_message().await.unwrap().unwrap();
        send.write_all(&frame(Message::Control(ControlMessage::HelloAck(hello(
            &["input/1"],
        )))))
        .await
        .unwrap();
        std::mem::forget(recv);
        RawReceiver {
            conn: s,
            control: send,
        }
    });
    let (session, _e) = Session::connect(c, info("phone"), Router::new())
        .await
        .unwrap();
    (server.await.unwrap(), session)
}

impl RawReceiver {
    /// The next `input` stream: its `InputSession`, and the stream.
    async fn input_session(&self) -> (InputSession, FramedRecv) {
        let mut stream = tokio::time::timeout(T, self.conn.quinn().accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(header, [1, 4]);
        let mut recv = FramedRecv::new(stream, Channel::Input);
        match recv.next_message().await.unwrap() {
            Some(Message::Input(InputMessage::InputSession(s))) => (s, recv),
            other => panic!("{other:?}"),
        }
    }

    async fn next_datagram(&self) -> Motion {
        let bytes = tokio::time::timeout(T, self.conn.quinn().read_datagram())
            .await
            .unwrap()
            .unwrap();
        match decode_datagram(&bytes, MAX_DATAGRAM).unwrap() {
            InputMessage::PointerMotion(m) | InputMessage::Scroll(m) => m,
            other => panic!("{other:?}"),
        }
    }

    async fn announce(&mut self, caps: &[&str]) {
        let caps = caps.iter().map(|c| Capability::parse(c).unwrap()).collect();
        self.control
            .write_all(&frame(Message::Control(ControlMessage::Capabilities {
                caps,
            })))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The receiver stops a session while the sender only moves (datagrams):
/// the next move opens a new session at once, with fresh counters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopped_session_is_replaced_for_a_datagram_only_sender() {
    let (raw, phone) = raw_receiver().await;
    let input = phone.input().unwrap();
    input.pointer(5, 0).await.unwrap();
    let (first, mut stream) = raw.input_session().await;
    assert_eq!(first, InputSession { s: 0, g: 0 });
    assert_eq!(raw.next_datagram().await.s, 0);

    stream.stop(ErrorCode::Cancelled);
    tokio::time::sleep(Duration::from_millis(100)).await;
    input.pointer(7, 0).await.unwrap();
    let (second, _stream) = tokio::time::timeout(Duration::from_millis(300), raw.input_session())
        .await
        .expect("no new session within 300 ms");
    assert_eq!(second, InputSession { s: 1, g: 0 });
    // Skip repeats of session 0 still in flight.
    let mut m = raw.next_datagram().await;
    while m.s == 0 {
        m = raw.next_datagram().await;
    }
    assert_eq!((m.s, m.seq, m.x), (1, 1, 7), "fresh counters");
}

/// Withdrawal ends the session and refuses input; re-enabling (announcement
/// 2) starts a new session based on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sender_follows_withdrawal_and_reenabling() {
    let (mut raw, phone) = raw_receiver().await;
    let input = phone.input().unwrap();
    input.pointer(1, 0).await.unwrap();
    let (first, mut stream) = raw.input_session().await;
    assert_eq!(first.s, 0);

    raw.announce(&[]).await; // announcement 1: no input
    assert!(matches!(
        input.pointer(1, 0).await,
        Err(SessionError::CapabilityMissing(Channel::Input))
    ));
    // Our stream of session 0 was reset.
    let ended = tokio::time::timeout(T, stream.next_message())
        .await
        .unwrap();
    assert!(ended.is_err(), "{ended:?}");

    raw.announce(&["input/1"]).await; // announcement 2
    input.pointer(3, 0).await.unwrap();
    let (second, _s) = raw.input_session().await;
    assert_eq!(second, InputSession { s: 1, g: 2 });
}

/// An announcement that keeps `input/1` (here it adds `clipboard/1`) leaves
/// the session alone: a drag in progress goes on in the same session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn announcement_keeping_input_keeps_the_session_and_drag() {
    let (mut raw, phone) = raw_receiver().await;
    let input = phone.input().unwrap();
    input.button(Button::Left, true).await.unwrap();
    let (first, mut stream) = raw.input_session().await;
    assert_eq!(first, InputSession { s: 0, g: 0 });
    assert!(matches!(
        stream.next_message().await.unwrap(),
        Some(Message::Input(InputMessage::PointerButton(b))) if b.down && b.b == 1
    ));

    raw.announce(&["input/1", "clipboard/1"]).await; // announcement 1
    input.pointer(4, 0).await.unwrap();
    let m = raw.next_datagram().await;
    assert_eq!((m.s, m.b), (0, 1), "same session, same drag");
    input.button(Button::Left, false).await.unwrap();
    assert!(matches!(
        stream.next_message().await.unwrap(),
        Some(Message::Input(InputMessage::PointerButton(b))) if !b.down && b.x == 4 && b.b == 2
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), raw.conn.quinn().accept_uni())
            .await
            .is_err(),
        "a new session was opened"
    );
}

/// The same list announced again during a drag changes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identical_announcement_keeps_the_session_and_drag() {
    let (mut raw, phone) = raw_receiver().await;
    let input = phone.input().unwrap();
    input.button(Button::Left, true).await.unwrap();
    let (first, mut stream) = raw.input_session().await;
    assert_eq!(first.s, 0);
    stream.next_message().await.unwrap();

    raw.announce(&["input/1"]).await; // announcement 1, same list
    raw.announce(&["input/1"]).await; // announcement 2, same list
    input.pointer(6, 0).await.unwrap();
    let m = raw.next_datagram().await;
    assert_eq!((m.s, m.b, m.x), (0, 1, 6));
    input.button(Button::Left, false).await.unwrap();
    assert!(matches!(
        stream.next_message().await.unwrap(),
        Some(Message::Input(InputMessage::PointerButton(b))) if !b.down && b.b == 2
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), raw.conn.quinn().accept_uni())
            .await
            .is_err(),
        "a new session was opened"
    );
}

/// Off and on again with no input sent in between: the next event still
/// starts a fresh session based on the re-enabling announcement, and the
/// old session's stream is ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_and_on_without_events_between_starts_a_fresh_session() {
    let (mut raw, phone) = raw_receiver().await;
    let input = phone.input().unwrap();
    input.button(Button::Left, true).await.unwrap();
    let (first, mut stream) = raw.input_session().await;
    assert_eq!(first, InputSession { s: 0, g: 0 });
    stream.next_message().await.unwrap();

    raw.announce(&[]).await; // 1: off
    raw.announce(&["input/1"]).await; // 2: on again
    input.pointer(2, 0).await.unwrap();
    let (second, _s) = raw.input_session().await;
    assert_eq!(second, InputSession { s: 1, g: 2 });
    let ended = tokio::time::timeout(T, stream.next_message())
        .await
        .unwrap();
    assert!(ended.is_err(), "old session not ended: {ended:?}");
    let mut m = raw.next_datagram().await;
    while m.s == 0 {
        m = raw.next_datagram().await;
    }
    assert_eq!((m.s, m.seq, m.b, m.x), (1, 1, 0, 2), "fresh counters");
}
