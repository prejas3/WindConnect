//! WN-59 (PROTOCOL.md §13): two devices hold a real session through the relay
//! Worker, and nothing the relay receives contains the plaintext.
//!
//! Needs the Worker running locally: `npm run dev` in relay/ (wrangler dev),
//! then `RELAY_URL=ws://127.0.0.1:8787 cargo test -p wind-core --test relay -- --ignored`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::protocol::cbor::Value;
use wind_core::protocol::{Channel, DeviceKind, RawMessage};
use wind_core::relay;
use wind_core::session::{LocalInfo, Router, Session};
use wind_core::transport::{Endpoint, PinnedKeys};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const T: Duration = Duration::from_secs(10);
/// Sent through the relay inside the session; must never appear in a relayed frame.
const SECRET: &str = "WN59 clipboard secret: correct horse battery staple";
const NAMES: [&str; 2] = ["relay-test-alice-laptop", "relay-test-bob-phone"];

struct Device {
    id: DeviceIdentity,
    pins: Arc<PinnedKeys>,
    endpoint: Endpoint,
}

impl Device {
    fn new() -> Self {
        let id = DeviceIdentity::generate().unwrap();
        let pins = Arc::new(PinnedKeys::new());
        let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &id, pins.clone()).unwrap();
        Device { id, pins, endpoint }
    }
}

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: None,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The string value of `"field":"…"` in a relay control frame.
fn field<'a>(json: &'a str, name: &str) -> &'a str {
    let start = json.find(&format!("\"{name}\":\"")).unwrap() + name.len() + 4;
    &json[start..start + json[start..].find('"').unwrap()]
}

async fn text(ws: &mut Ws) -> String {
    loop {
        match tokio::time::timeout(T, ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return text.to_string(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected a control frame, got {other:?}"),
        }
    }
}

/// Connects `me` towards `peer` and answers the challenge; returns the
/// socket and whether the peer was already there.
async fn join(url: &str, me: &DeviceIdentity, peer: PublicKey) -> (Ws, bool) {
    let (mut ws, _) = connect_async(format!(
        "{url}{}",
        relay::request_path(&me.public_key(), &peer)
    ))
    .await
    .expect("is the relay running (npm run dev in relay/)?");
    let challenge = text(&mut ws).await;
    let nonce: [u8; relay::NONCE_LEN] = (0..relay::NONCE_LEN)
        .map(|i| u8::from_str_radix(&field(&challenge, "nonce")[i * 2..i * 2 + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let sig = relay::sign_auth(me, &nonce, &peer);
    ws.send(Message::text(format!(
        r#"{{"t":"auth","sig":"{}"}}"#,
        hex(&sig)
    )))
    .await
    .unwrap();
    let ready = text(&mut ws).await;
    assert!(ready.contains(r#""t":"ready""#), "{ready}");
    (ws, ready.contains(r#""peer":true"#))
}

/// A local UDP address that stands for the other device: what the QUIC
/// endpoint at `local` sends there goes to the relay (and is recorded as
/// what the relay sees); data frames from the relay come back to `local`.
async fn shim(ws: Ws, local: SocketAddr) -> (SocketAddr, Arc<Mutex<Vec<Vec<u8>>>>) {
    let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = udp.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (mut sink, mut stream) = ws.split();
    let (out, record) = (udp.clone(), seen.clone());
    tokio::spawn(async move {
        let mut buf = vec![0; 65536];
        while let Ok((n, _)) = out.recv_from(&mut buf).await {
            assert!(n <= relay::MAX_FRAME, "QUIC packet of {n} B");
            record.lock().unwrap().push(buf[..n].to_vec());
            if sink.send(Message::binary(buf[..n].to_vec())).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        while let Some(Ok(message)) = stream.next().await {
            if let Message::Binary(packet) = message {
                let _ = udp.send_to(&packet, local).await;
            }
        }
    });
    (address, seen)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the relay Worker running locally (wrangler dev) and RELAY_URL"]
async fn session_through_the_relay_shows_it_only_ciphertext() {
    let url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:8787".into());
    let (alice, bob) = (Device::new(), Device::new());
    alice.pins.pin(bob.id.public_key());
    bob.pins.pin(alice.id.public_key());

    let (bob_ws, early) = join(&url, &bob.id, alice.id.public_key()).await;
    assert!(!early);
    let (alice_ws, present) = join(&url, &alice.id, bob.id.public_key()).await;
    assert!(present, "both devices named each other: one rendezvous");
    let (bob_via_relay, alice_sent) = shim(alice_ws, alice.endpoint.local_addr().unwrap()).await;
    let (_, bob_sent) = shim(bob_ws, bob.endpoint.local_addr().unwrap()).await;

    let endpoint = bob.endpoint.clone();
    let accepted = tokio::spawn(async move {
        let conn = endpoint.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let clip = router.route(Channel::Clipboard, 1, 8);
        let (session, _events) = Session::accept(conn, info(NAMES[1]), router).await.unwrap();
        (session, clip)
    });
    let (session, _events) = tokio::time::timeout(
        T,
        Session::dial(
            &alice.endpoint,
            bob_via_relay,
            bob.id.public_key(),
            info(NAMES[0]),
            Router::new(),
        ),
    )
    .await
    .expect("session through the relay")
    .unwrap();
    let (bob_session, mut clip) = accepted.await.unwrap();
    assert_eq!(session.peer().hello.name, NAMES[1]);
    assert_eq!(bob_session.peer().hello.name, NAMES[0]);
    let rtt = session.ping().await.unwrap();

    let message = RawMessage::new(
        1,
        Value::Map(vec![
            (Value::Int(0), Value::Text(SECRET.into())),
            (Value::Int(1), Value::Bytes(SECRET.as_bytes().to_vec())),
        ]),
    );
    session.send(Channel::Clipboard, &message).await.unwrap();
    let got = tokio::time::timeout(T, clip.recv()).await.unwrap().unwrap();
    assert_eq!(got.message, message, "the peer reads it");
    session.ping().await.unwrap();

    // What the relay received, in both directions.
    let frames: Vec<Vec<u8>> = [alice_sent, bob_sent]
        .iter()
        .flat_map(|seen| seen.lock().unwrap().clone())
        .collect();
    let bytes: usize = frames.iter().map(Vec::len).sum();
    println!(
        "through the relay: {} frames, {bytes} B, RTT {rtt:?}",
        frames.len()
    );
    assert!(
        frames.len() >= 6,
        "handshake and data went through the relay"
    );
    for frame in &frames {
        assert!(
            !contains(frame, SECRET.as_bytes()),
            "plaintext in a relayed frame"
        );
        assert!(
            !contains(frame, &SECRET.as_bytes()[..12]),
            "part of the plaintext"
        );
        for name in NAMES {
            assert!(
                !contains(frame, name.as_bytes()),
                "device name {name} in clear"
            );
        }
    }
}

/// WN-60: the client's own link ([`relay::RelayLink`]) on both sides: a
/// session over the relay, presence, and the link's end when the peer's
/// side goes (dropped link, closed WebSocket).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the relay Worker running locally (wrangler dev) and RELAY_URL"]
async fn relay_links_carry_a_session_and_report_presence() {
    let url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:8787".into());
    let (phone, pc) = (Device::new(), Device::new());
    phone.pins.pin(pc.id.public_key());
    pc.pins.pin(phone.id.public_key());

    // The PC waits on the relay for its paired phone.
    let pc_link =
        relay::RelayLink::connect(&url, &pc.id, phone.id.public_key(), pc.endpoint.clone())
            .await
            .unwrap();
    let mut pc_sees = pc_link.presence();
    assert!(!*pc_sees.borrow());
    let endpoint = pc.endpoint.clone();
    let accepted = tokio::spawn(async move {
        let conn = endpoint.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let clip = router.route(Channel::Clipboard, 1, 8);
        let (session, _events) = Session::accept(conn, info(NAMES[0]), router).await.unwrap();
        (session, clip)
    });

    let phone_link =
        relay::RelayLink::connect(&url, &phone.id, pc.id.public_key(), phone.endpoint.clone())
            .await
            .unwrap();
    assert!(
        *phone_link.presence().borrow(),
        "the PC was already waiting"
    );
    tokio::time::timeout(T, pc_sees.wait_for(|present| *present))
        .await
        .unwrap()
        .unwrap();
    let start = std::time::Instant::now();
    let (session, _events) = tokio::time::timeout(
        T,
        Session::dial(
            &phone.endpoint,
            phone_link.addr(),
            pc.id.public_key(),
            info(NAMES[1]),
            Router::new(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let up = start.elapsed();
    let (_pc_session, mut clip) = accepted.await.unwrap();
    let message = RawMessage::new(
        1,
        Value::Map(vec![(Value::Int(0), Value::Text(SECRET.into()))]),
    );
    session.send(Channel::Clipboard, &message).await.unwrap();
    let got = tokio::time::timeout(T, clip.recv()).await.unwrap().unwrap();
    assert_eq!(got.message, message);
    let rtt = session.ping().await.unwrap();
    println!("session through RelayLink in {up:?}, RTT {rtt:?}");

    drop(pc_link);
    let mut phone_sees = phone_link.presence();
    tokio::time::timeout(T, phone_sees.wait_for(|present| !*present))
        .await
        .expect("the relay tells the phone the PC left")
        .unwrap();
}

/// A refused link reports why: a stranger's key is not refused by the relay
/// (it cannot know pairings), but a wrong URL and a non-relay server are.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the relay Worker running locally (wrangler dev) and RELAY_URL"]
async fn relay_link_errors_are_reported() {
    let url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:8787".into());
    let device = Device::new();
    let other = DeviceIdentity::generate().unwrap().public_key();
    for bad in ["ws://example.com", "http://127.0.0.1:8787"] {
        let error = relay::RelayLink::connect(bad, &device.id, other, device.endpoint.clone())
            .await
            .unwrap_err();
        assert!(matches!(error, relay::RelayError::Url(_)), "{bad}: {error}");
    }
    let error = relay::RelayLink::connect(
        &format!("{url}/"),
        &device.id,
        device.id.public_key(),
        device.endpoint.clone(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, relay::RelayError::Connect(_)),
        "own key as peer: {error}"
    );
}
