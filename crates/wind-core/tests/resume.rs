//! Keep-alive, path migration, 0-RTT resumption and reconnect backoff
//! (WC-12): after a network change the connection is back in < 300 ms.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::protocol::cbor::Value;
use wind_core::protocol::{Channel, DeviceKind, RawMessage};
use wind_core::session::{LocalInfo, Router, Session};
use wind_core::transport::{
    Backoff, ConnectError, Endpoint, PinnedKeys, IDLE_TIMEOUT, KEEP_ALIVE_INTERVAL,
};

const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const BUDGET: Duration = Duration::from_millis(300);
const T: Duration = Duration::from_secs(10);

struct Device {
    id: DeviceIdentity,
    pins: Arc<PinnedKeys>,
    endpoint: Endpoint,
}

impl Device {
    fn new() -> Self {
        let id = DeviceIdentity::generate().unwrap();
        let pins = Arc::new(PinnedKeys::new());
        let endpoint = Endpoint::bind(LOCALHOST, &id, pins.clone()).unwrap();
        Device { id, pins, endpoint }
    }
    fn key(&self) -> PublicKey {
        self.id.public_key()
    }
    fn addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap()
    }
}

fn pair(a: &Device, b: &Device) {
    a.pins.pin(b.key());
    b.pins.pin(a.key());
}

fn info(name: &str) -> LocalInfo {
    LocalInfo {
        name: name.into(),
        kind: DeviceKind::Computer,
        app: None,
    }
}

/// Serves sessions on `server` until the test ends: each accepted
/// connection gets a session (which answers Ping by itself).
fn serve(server: &Device) {
    let ep = server.endpoint.clone();
    tokio::spawn(async move {
        while let Some(conn) = ep.accept().await {
            let Ok(conn) = conn else { continue };
            tokio::spawn(async move {
                if let Ok((session, _events)) =
                    Session::accept(conn, info("server"), Router::new()).await
                {
                    let _ = session.closed().await;
                }
            });
        }
    });
}

/// A second loopback address if this system routes one (127.0.0.2), so a
/// migration also changes the IP, not only the port.
fn other_local_ip() -> IpAddr {
    let candidate = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
    match std::net::UdpSocket::bind((candidate, 0)) {
        Ok(_) => candidate,
        Err(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
    }
}

#[test]
fn keep_alive_fits_the_idle_timeout() {
    assert!(KEEP_ALIVE_INTERVAL * 3 <= IDLE_TIMEOUT);
}

#[test]
fn backoff_grows_with_jitter_caps_and_resets() {
    let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(2));
    let delays: Vec<Duration> = (0..8).map(|_| b.next_delay()).collect();
    for (i, d) in delays.iter().enumerate() {
        let step = Duration::from_millis(100 * (1 << i)).min(Duration::from_secs(2));
        assert!(
            *d >= step / 2 && *d <= step,
            "attempt {i}: {d:?} vs step {step:?}"
        );
    }
    b.reset();
    assert!(b.next_delay() <= Duration::from_millis(100));
}

/// Acceptance (WC-12): the network changes under the client (new socket,
/// new address); the same session works again within 300 ms, without a new
/// handshake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_migration_brings_the_session_back_within_300_ms() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    let ep = server.endpoint.clone();
    let accepted = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        let mut router = Router::new();
        let clip = router.route(Channel::Clipboard, 1, 8);
        let (s, _e) = Session::accept(conn, info("server"), router).await.unwrap();
        (s, clip)
    });
    let (session, _events) = Session::dial(
        &client.endpoint,
        server.addr(),
        server.key(),
        info("client"),
        Router::new(),
    )
    .await
    .unwrap();
    let (server_session, mut clip) = accepted.await.unwrap();
    session.ping().await.unwrap();
    let old_addr = server_session.connection().quinn().remote_address();

    let new_ip = other_local_ip();
    let start = Instant::now();
    let new_local = client.endpoint.rebind(SocketAddr::new(new_ip, 0)).unwrap();
    let rtt = session.ping().await.unwrap();
    let back = start.elapsed();
    println!("migrated {old_addr} -> {new_local}: first ping after {back:?} (RTT {rtt:?})");
    assert!(back < BUDGET, "back after {back:?}");

    // The server sees the new path; data keeps flowing on the same session.
    let msg = RawMessage::new(1, Value::Map(vec![(Value::Int(0), Value::Int(9))]));
    session.send(Channel::Clipboard, &msg).await.unwrap();
    let got = tokio::time::timeout(T, clip.recv()).await.unwrap().unwrap();
    assert_eq!(got.message, msg);
    assert_eq!(
        server_session.connection().quinn().remote_address(),
        new_local
    );
}

/// WN-84: an endpoint on a fixed port keeps that port after a network
/// change. `refresh_path` binds nothing new (a second socket on the port
/// would fail with `EADDRINUSE`), yet every connection probes its path at
/// once and the session goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_path_keeps_a_fixed_port_and_probes_the_peer() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    serve(&server);
    let (session, _events) = Session::dial(
        &client.endpoint,
        server.addr(),
        server.key(),
        info("client"),
        Router::new(),
    )
    .await
    .unwrap();
    session.ping().await.unwrap();
    let before = client.addr();
    let pings = session.connection().quinn().stats().frame_tx.ping;

    assert_eq!(client.endpoint.refresh_path().unwrap(), before);
    let probe = Instant::now();
    while session.connection().quinn().stats().frame_tx.ping == pings {
        assert!(probe.elapsed() < BUDGET, "no path probe after refresh_path");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let start = Instant::now();
    session.ping().await.unwrap();
    assert!(start.elapsed() < BUDGET, "back after {:?}", start.elapsed());
    assert_eq!(client.addr(), before);
}

/// After the connection is gone (sleep, the other side restarted its
/// network), a new one resumes the TLS session: `Hello` rides in 0-RTT and
/// the session is up within 300 ms.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_resumes_with_0rtt_within_300_ms() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    serve(&server);

    // First contact: full handshake, and the server sends session tickets.
    let (first, _e) = Session::dial(
        &client.endpoint,
        server.addr(),
        server.key(),
        info("client"),
        Router::new(),
    )
    .await
    .unwrap();
    first.ping().await.unwrap();
    first.connection().quinn().close(0u32.into(), b"");

    let start = Instant::now();
    let dialing = client.endpoint.dial(server.addr(), server.key()).unwrap();
    assert!(dialing.early().is_some(), "no TLS session to resume");
    drop(dialing);
    let (second, _e) = Session::dial(
        &client.endpoint,
        server.addr(),
        server.key(),
        info("client"),
        Router::new(),
    )
    .await
    .unwrap();
    let up = start.elapsed();
    second.ping().await.unwrap();
    println!("resumed session up after {up:?}");
    assert!(up < BUDGET, "up after {up:?}");
    assert_eq!(second.peer().hello.name, "server");
}

/// A resumed TLS session skips the certificate check, so an unpinned key
/// must be refused anyway.
#[tokio::test]
async fn resumption_does_not_bypass_unpinning() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    let ep = server.endpoint.clone();
    let accepts = tokio::spawn(async move {
        let first = ep.accept().await.unwrap();
        let second = ep.accept().await.unwrap();
        (first.map(|c| c.peer_key()), second.map(|c| c.peer_key()))
    });

    let conn = client
        .endpoint
        .connect(server.addr(), server.key())
        .await
        .unwrap();
    // Wait for the session ticket, then leave.
    tokio::time::sleep(Duration::from_millis(100)).await;
    conn.quinn().close(0u32.into(), b"");

    server.pins.unpin(&client.key());
    let dialing = client.endpoint.dial(server.addr(), server.key()).unwrap();
    assert!(dialing.early().is_some(), "expected a resumed session");
    let result = dialing.established().await;
    let (first, second) = accepts.await.unwrap();
    assert_eq!(first.unwrap(), client.key());
    let refused = second.expect_err("server accepted an unpinned key on resumption");
    assert!(
        matches!(refused, ConnectError::Unpinned(k) if k == client.key()),
        "{refused}"
    );
    // The client is closed with UNTRUSTED_PEER (or never gets going).
    if let Ok((conn, _)) = result {
        let reason = tokio::time::timeout(T, conn.quinn().closed())
            .await
            .unwrap();
        assert!(
            wind_core::transport::is_untrusted_close(&reason),
            "{reason}"
        );
    }
}

/// No network: attempts back off; when the network comes back and the
/// device is told so, the session is up at once (< 300 ms), not at the next
/// backoff step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_backs_off_and_wakes_on_network_change() {
    let client = Device::new();
    // A port with nobody answering on it for now. The socket stays bound
    // until the server takes the port, so that no other test running in
    // parallel can be given it meanwhile (WN-85): a stranger's endpoint
    // there would refuse our key and end the reconnect early.
    let probe = std::net::UdpSocket::bind(LOCALHOST).unwrap();
    let addr = probe.local_addr().unwrap();
    let server_id = DeviceIdentity::generate().unwrap();
    client.pins.pin(server_id.public_key());

    let network = Arc::new(Notify::new());
    let ep = client.endpoint.clone();
    let wake = network.clone();
    let key = server_id.public_key();
    let reconnecting = tokio::spawn(async move {
        // Long steps, so only the wake-up can explain a quick connection.
        let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(30));
        let session = Session::reconnect(
            &ep,
            addr,
            key,
            &info("client"),
            Router::new,
            &mut backoff,
            &wake,
        )
        .await;
        (session, Instant::now())
    });

    // The first attempt fails (nobody listens) and the backoff starts.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert!(!reconnecting.is_finished());

    // The network is back: the peer listens again, and we are told.
    let server_pins = Arc::new(PinnedKeys::new());
    server_pins.pin(client.key());
    drop(probe);
    let server = Device {
        endpoint: Endpoint::bind(addr, &server_id, server_pins.clone()).unwrap(),
        id: server_id,
        pins: server_pins,
    };
    serve(&server);
    let told = Instant::now();
    network.notify_one();
    let (session, at) = tokio::time::timeout(T, reconnecting)
        .await
        .unwrap()
        .unwrap();
    let (session, _events) = session.unwrap();
    let after = at - told;
    println!("session up {after:?} after the network change");
    assert!(after < BUDGET, "{after:?}");
    assert_eq!(session.peer().key, key);
    session.ping().await.unwrap();
}

#[tokio::test]
async fn reconnect_gives_up_on_a_peer_that_refuses_us() {
    let client = Device::new();
    let server = Device::new();
    // The server doesn't know the client.
    client.pins.pin(server.key());
    serve(&server);
    let mut backoff = Backoff::default();
    let err = tokio::time::timeout(
        T,
        Session::reconnect(
            &client.endpoint,
            server.addr(),
            server.key(),
            &info("client"),
            Router::new,
            &mut backoff,
            &Notify::new(),
        ),
    )
    .await
    .expect("kept retrying a peer that refuses us")
    .unwrap_err();
    assert!(err.is_untrusted_peer(), "{err}");
}

// --- Two processes: migration across a process boundary --------------------
//
// The server is another process (as on a second machine); the client
// migrates to a new socket and measures how long until the session answers.

const PEER_ENV: &str = "WINDCONNECT_RESUME_PEER";

fn hex(key: &PublicKey) -> String {
    key.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> PublicKey {
    let mut raw = [0u8; 32];
    for (i, b) in raw.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    PublicKey::from_bytes(&raw).unwrap()
}

#[test]
fn two_processes_migrate_within_300_ms() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "resume_peer_process",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PEER_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut next = move || -> Vec<String> {
        loop {
            let line = lines.next().expect("peer process ended").unwrap();
            if let Some(at) = line.find("WC ") {
                return line[at + 3..].split(' ').map(str::to_string).collect();
            }
        }
    };
    let hello = next();
    assert_eq!(hello[0], "KEY");
    let peer_key = unhex(&hello[1]);
    let peer_addr: SocketAddr = (Ipv4Addr::LOCALHOST, hello[2].parse::<u16>().unwrap()).into();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let back = rt.block_on(async {
        let me = Device::new();
        me.pins.pin(peer_key);
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, "PIN {}", hex(&me.key())).unwrap();
        stdin.flush().unwrap();

        let (session, _e) = Session::dial(
            &me.endpoint,
            peer_addr,
            peer_key,
            info("client"),
            Router::new(),
        )
        .await
        .unwrap();
        session.ping().await.unwrap();
        let start = Instant::now();
        me.endpoint
            .rebind(SocketAddr::new(other_local_ip(), 0))
            .unwrap();
        session.ping().await.unwrap();
        let back = start.elapsed();
        session
            .goodbye(wind_core::protocol::ErrorCode::GoingAway, None)
            .await;
        let _ = tokio::time::timeout(T, me.endpoint.wait_idle()).await;
        back
    });
    println!("two processes: back {back:?} after migration");
    assert!(back < BUDGET, "{back:?}");
    assert_eq!(next(), ["DONE"]);
    assert!(child.wait().unwrap().success());
}

/// The server of the two-process test. Does nothing elsewhere.
#[test]
#[ignore = "runs only as the peer of two_processes_migrate_within_300_ms"]
fn resume_peer_process() {
    if std::env::var_os(PEER_ENV).is_none() {
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let device = Device::new();
        println!("WC KEY {} {}", hex(&device.key()), device.addr().port());
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        device
            .pins
            .pin(unhex(line.trim().strip_prefix("PIN ").expect("PIN line")));
        let conn = tokio::time::timeout(T, device.endpoint.accept())
            .await
            .expect("no connection")
            .unwrap()
            .unwrap();
        let (session, _e) = Session::accept(conn, info("server"), Router::new())
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(20), session.closed()).await;
        println!("WC DONE");
    });
}

/// The 0-RTT data of a resumed connection is accepted by the peer (not just
/// sent and then repeated after the handshake).
#[tokio::test]
async fn zero_rtt_data_is_accepted() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    let ep = server.endpoint.clone();
    let server_side = tokio::spawn(async move {
        let _first = ep.accept().await.unwrap().unwrap();
        let second = ep.accept().await.unwrap().unwrap();
        let (_s, mut r) = second.quinn().accept_bi().await.unwrap();
        r.read_to_end(64).await.unwrap()
    });
    let first = client
        .endpoint
        .connect(server.addr(), server.key())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await; // session tickets
    first.quinn().close(0u32.into(), b"");

    let dialing = client.endpoint.dial(server.addr(), server.key()).unwrap();
    let early = dialing.early().expect("resumable").clone();
    let (mut send, _recv) = early.open_bi().await.unwrap();
    send.write_all(b"early hello").await.unwrap();
    send.finish().unwrap();
    let (conn, accepted) = dialing.established().await.unwrap();
    assert!(accepted, "0-RTT data refused");
    assert_eq!(conn.peer_key(), server.key());
    assert_eq!(server_side.await.unwrap(), b"early hello");
}

/// The peer restarted and lost its TLS session cache: our resumption and
/// its 0-RTT data are refused, `Hello` goes again after the handshake, and
/// the session must work fully: repeated pings, a routed message, the
/// right peer key. Covers the refused-0-RTT branch of `Session::dial`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_0rtt_falls_back_to_a_working_session() {
    let client = Device::new();
    let server = Device::new();
    pair(&client, &server);
    let addr = server.addr();
    serve(&server);
    let (first, _e) = Session::dial(
        &client.endpoint,
        addr,
        server.key(),
        info("client"),
        Router::new(),
    )
    .await
    .unwrap();
    first.ping().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await; // session tickets
    first.connection().quinn().close(0u32.into(), b"");

    // The peer restarts: same identity, same address, empty session cache.
    let Device { id, pins, endpoint } = server;
    endpoint.close();
    let _ = tokio::time::timeout(T, endpoint.wait_idle()).await;
    drop(endpoint);
    let key = id.public_key();
    // We still hold a ticket, so resumption and 0-RTT will be tried (and
    // refused by the restarted peer). Checked before the peer listens, so
    // this probe never reaches it.
    let probe = client.endpoint.dial(addr, key).unwrap();
    assert!(probe.early().is_some(), "expected a resumption attempt");
    drop(probe);
    let mut restarted = None;
    for _ in 0..40 {
        match Endpoint::bind(addr, &id, pins.clone()) {
            Ok(ep) => {
                restarted = Some(ep);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    let endpoint = restarted.expect("address still taken");
    let ep = endpoint.clone();
    let served = tokio::spawn(async move {
        // The dropped probe's handshake packets may still arrive once the
        // peer listens again, as a connection that then dies (WN-85): skip
        // whatever fails until the real session is up.
        loop {
            let Ok(conn) = ep.accept().await.expect("endpoint open") else {
                continue;
            };
            let mut router = Router::new();
            let clip = router.route(Channel::Clipboard, 1, 8);
            if let Ok((s, _e)) = Session::accept(conn, info("server"), router).await {
                break (s, clip);
            }
        }
    });

    let (session, _e) = Session::dial(&client.endpoint, addr, key, info("client"), Router::new())
        .await
        .unwrap();
    assert_eq!(session.peer().key, key);
    for _ in 0..3 {
        session.ping().await.unwrap();
    }
    let (_server_session, mut clip) = served.await.unwrap();
    let msg = RawMessage::new(1, Value::Map(vec![(Value::Int(0), Value::Int(5))]));
    session.send(Channel::Clipboard, &msg).await.unwrap();
    let got = tokio::time::timeout(T, clip.recv()).await.unwrap().unwrap();
    assert_eq!(got.message, msg);
    drop(endpoint);
}

/// A network change while an attempt is still dialing: that attempt is
/// dropped and a fresh one starts at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_change_during_an_attempt_retries_at_once() {
    let client = Device::new();
    // Held until the server takes the port (see the test above).
    let probe = std::net::UdpSocket::bind(LOCALHOST).unwrap();
    let addr = probe.local_addr().unwrap();
    let server_id = DeviceIdentity::generate().unwrap();
    client.pins.pin(server_id.public_key());

    let network = Arc::new(Notify::new());
    let ep = client.endpoint.clone();
    let wake = network.clone();
    let key = server_id.public_key();
    let reconnecting = tokio::spawn(async move {
        let mut backoff = Backoff::new(Duration::from_secs(5), Duration::from_secs(30));
        let session = Session::reconnect(
            &ep,
            addr,
            key,
            &info("client"),
            Router::new,
            &mut backoff,
            &wake,
        )
        .await;
        (session, Instant::now())
    });

    // The first attempt is still dialing nobody (it would wait up to 3 s).
    tokio::time::sleep(Duration::from_millis(100)).await;
    let server_pins = Arc::new(PinnedKeys::new());
    server_pins.pin(client.key());
    drop(probe);
    let server = Device {
        endpoint: Endpoint::bind(addr, &server_id, server_pins.clone()).unwrap(),
        id: server_id,
        pins: server_pins,
    };
    serve(&server);
    let told = Instant::now();
    network.notify_one();
    let (session, at) = tokio::time::timeout(T, reconnecting)
        .await
        .unwrap()
        .unwrap();
    let (session, _events) = session.unwrap();
    let after = at - told;
    println!("session up {after:?} after a change during an attempt");
    assert!(after < BUDGET, "{after:?}");
    session.ping().await.unwrap();

    // The dropped attempt leaves nothing behind: only the live connection.
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.endpoint.open_connections() > 1 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(client.endpoint.open_connections(), 1);
}
