//! QUIC with pinned keys (WC-10): a foreign key is refused; two processes
//! connect locally.

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::transport::{
    certificate_key, is_untrusted_close, ConnectError, Connection, Endpoint, PinnedKeys,
};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const TIMEOUT: Duration = Duration::from_secs(10);

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

/// Echoes one bi stream, then waits for the peer to close.
async fn echo_once(conn: &Connection) {
    let (mut send, mut recv) = conn.quinn().accept_bi().await.unwrap();
    let data = recv.read_to_end(1024).await.unwrap();
    send.write_all(&data).await.unwrap();
    send.finish().unwrap();
    let _ = conn.quinn().closed().await;
}

async fn round_trip(conn: &Connection, msg: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let (mut send, mut recv) = conn.quinn().open_bi().await?;
    send.write_all(msg).await?;
    send.finish()?;
    Ok(recv.read_to_end(1024).await?)
}

/// A client whose key the server refuses: TLS 1.3 sends the client
/// certificate in the client's last flight, so `connect` may finish before
/// the server's verdict; the connection is then closed at once. Either way
/// the outcome must be "untrusted" and no data may pass.
async fn assert_client_refused(result: Result<Connection, ConnectError>) {
    match result {
        Err(e) => assert!(e.is_untrusted_peer(), "{e}"),
        Ok(conn) => {
            let sent = round_trip(&conn, b"must not pass").await;
            assert!(sent.is_err(), "data passed a refused connection");
            let reason = tokio::time::timeout(TIMEOUT, conn.quinn().closed())
                .await
                .unwrap();
            assert!(is_untrusted_close(&reason), "{reason}");
        }
    }
}

#[tokio::test]
async fn paired_devices_connect_both_ways() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b);

    for (client, server) in [(&a, &b), (&b, &a)] {
        let server_side = {
            let ep = server.endpoint.clone();
            tokio::spawn(async move {
                let conn = ep.accept().await.unwrap().unwrap();
                let peer = conn.peer_key();
                echo_once(&conn).await;
                peer
            })
        };
        let conn = tokio::time::timeout(
            TIMEOUT,
            client.endpoint.connect(server.addr(), server.key()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(conn.peer_key(), server.key());
        assert_eq!(round_trip(&conn, b"hello").await.unwrap(), b"hello");
        conn.quinn().close(0u32.into(), b"");
        assert_eq!(server_side.await.unwrap(), client.key());
    }
}

#[tokio::test]
async fn server_refuses_unpinned_client() {
    let client = Device::new();
    let server = Device::new();
    // The client trusts the server, the server doesn't know the client.
    client.pins.pin(server.key());

    let server_side = {
        let ep = server.endpoint.clone();
        tokio::spawn(async move { ep.accept().await.unwrap() })
    };
    let result = tokio::time::timeout(
        TIMEOUT,
        client.endpoint.connect(server.addr(), server.key()),
    )
    .await
    .unwrap();
    assert_client_refused(result).await;
    let accepted = server_side.await.unwrap();
    let err = accepted.expect_err("server accepted an unpinned key");
    assert!(err.is_untrusted_peer(), "{err}");
}

#[tokio::test]
async fn client_refuses_other_key_at_the_address() {
    // `m` is even a paired device, but not the one the client dials: the
    // client must get exactly the key it asked for.
    let client = Device::new();
    let wanted = Device::new();
    let m = Device::new();
    pair(&client, &wanted);
    pair(&client, &m);

    let server_side = {
        let ep = m.endpoint.clone();
        tokio::spawn(async move { ep.accept().await.unwrap() })
    };
    let err = tokio::time::timeout(TIMEOUT, client.endpoint.connect(m.addr(), wanted.key()))
        .await
        .unwrap()
        .expect_err("client accepted a key it didn't dial");
    assert!(err.is_untrusted_peer(), "{err}");
    assert!(server_side.await.unwrap().is_err());
}

#[tokio::test]
async fn client_refuses_unpinned_server() {
    let client = Device::new();
    let stranger = Device::new();
    stranger.pins.pin(client.key());
    // Dialing an unpinned key fails before anything is sent.
    let err = client
        .endpoint
        .connect(stranger.addr(), stranger.key())
        .await
        .expect_err("dialed an unpinned key");
    assert!(matches!(err, ConnectError::NotPaired(k) if k == stranger.key()));
    assert!(err.is_untrusted_peer());
}

#[tokio::test]
async fn unpinning_refuses_new_connections() {
    let a = Device::new();
    let b = Device::new();
    pair(&a, &b);
    // Works while paired.
    let ep = b.endpoint.clone();
    let accept = tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().unwrap();
        echo_once(&conn).await;
    });
    let conn = a.endpoint.connect(b.addr(), b.key()).await.unwrap();
    assert_eq!(round_trip(&conn, b"x").await.unwrap(), b"x");
    conn.quinn().close(0u32.into(), b"");
    accept.await.unwrap();

    // b unpins a: a's next connection is refused by b.
    b.pins.unpin(&a.key());
    let ep = b.endpoint.clone();
    let accept = tokio::spawn(async move { ep.accept().await.unwrap() });
    let result = tokio::time::timeout(TIMEOUT, a.endpoint.connect(b.addr(), b.key()))
        .await
        .unwrap();
    assert_client_refused(result).await;
    assert!(accept.await.unwrap().unwrap_err().is_untrusted_peer());
}

#[tokio::test]
async fn non_ed25519_certificate_is_refused() {
    // A server with a P-256 certificate (what a non-WindConnect TLS server
    // would have), speaking our ALPN.
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = rcgen::CertificateParams::new(vec!["windconnect".to_string()])
        .unwrap()
        .self_signed(&kp)
        .unwrap();
    assert_eq!(certificate_key(cert.der()), None);

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(kp.serialize_der()).into(),
        )
        .unwrap();
    tls.alpn_protocols = vec![wind_core::protocol::ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap();
    let server =
        quinn::Endpoint::server(quinn::ServerConfig::with_crypto(Arc::new(quic)), LOCALHOST)
            .unwrap();
    let server_addr = server.local_addr().unwrap();
    tokio::spawn(async move {
        if let Some(incoming) = server.accept().await {
            let _ = incoming.await;
        }
    });

    // Even with some pinned key expected at that address, the P-256 server
    // gets nowhere: we offer only Ed25519 signatures (so it can't even sign
    // the handshake), and its certificate has no Ed25519 key to match.
    let client = Device::new();
    let expected = Device::new();
    client.pins.pin(expected.key());
    let err = tokio::time::timeout(
        TIMEOUT,
        client.endpoint.connect(server_addr, expected.key()),
    )
    .await
    .unwrap()
    .expect_err("accepted a P-256 certificate");
    assert!(
        matches!(err, ConnectError::Connection(_)),
        "refused by the handshake: {err}"
    );
}

#[test]
fn certificate_key_is_the_device_key() {
    let id = DeviceIdentity::generate().unwrap();
    assert_eq!(
        certificate_key(&id.certificate_der()),
        Some(id.public_key())
    );
}

// --- Two processes ---------------------------------------------------------
//
// The test starts a copy of this test binary running `peer_process` (a
// device that listens), learns its key and port, pins our key in it, and
// connects: first from a device it doesn't know (refused), then from the
// paired one (echo).

const PEER_ENV: &str = "WINDCONNECT_PEER_PROCESS";

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
fn two_processes_connect_and_refuse_a_stranger() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "peer_process",
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
            // libtest may print "test peer_process ... " before it.
            if let Some(at) = line.find("WC ") {
                return line[at + 3..].split(' ').map(str::to_string).collect();
            }
        }
    };

    let hello = next();
    assert_eq!(hello[0], "KEY");
    let peer_key = unhex(&hello[1]);
    let peer_addr: SocketAddr = (Ipv4Addr::LOCALHOST, hello[2].parse::<u16>().unwrap()).into();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (me, stranger) = rt.block_on(async { (Device::new(), Device::new()) });
    me.pins.pin(peer_key);
    stranger.pins.pin(peer_key);
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "PIN {}", hex(&me.key())).unwrap();
    stdin.flush().unwrap();

    rt.block_on(async {
        // A device the peer doesn't know.
        let result = tokio::time::timeout(TIMEOUT, stranger.endpoint.connect(peer_addr, peer_key))
            .await
            .unwrap();
        assert_client_refused(result).await;
    });
    assert_eq!(next(), ["REJECTED", "untrusted=true"]);

    let echoed = rt.block_on(async {
        let conn = tokio::time::timeout(TIMEOUT, me.endpoint.connect(peer_addr, peer_key))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(conn.peer_key(), peer_key);
        let echoed = round_trip(&conn, b"ping across processes").await.unwrap();
        conn.quinn().close(0u32.into(), b"");
        // Let the runtime send the close before leaving block_on.
        let _ = tokio::time::timeout(TIMEOUT, me.endpoint.wait_idle()).await;
        echoed
    });
    assert_eq!(echoed, b"ping across processes");
    assert_eq!(next(), ["ACCEPTED".to_string(), hex(&me.key())]);
    assert_eq!(next(), ["DONE"]);
    assert!(child.wait().unwrap().success());
}

/// The listening device of the two-process test. Does nothing elsewhere.
#[test]
#[ignore = "runs only as the peer of two_processes_connect_and_refuse_a_stranger"]
fn peer_process() {
    if std::env::var_os(PEER_ENV).is_none() {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let device = Device::new();
        println!("WC KEY {} {}", hex(&device.key()), device.addr().port());

        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        let pin = line.trim().strip_prefix("PIN ").expect("PIN line");
        device.pins.pin(unhex(pin));

        for _ in 0..2 {
            match tokio::time::timeout(TIMEOUT, device.endpoint.accept())
                .await
                .expect("no connection")
                .expect("endpoint closed")
            {
                Err(e) => println!("WC REJECTED untrusted={}", e.is_untrusted_peer()),
                Ok(conn) => {
                    println!("WC ACCEPTED {}", hex(&conn.peer_key()));
                    echo_once(&conn).await;
                }
            }
        }
        println!("WC DONE");
        device.endpoint.close();
        let _ = tokio::time::timeout(TIMEOUT, device.endpoint.wait_idle()).await;
    });
}
