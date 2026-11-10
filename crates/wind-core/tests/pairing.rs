//! Pairing (WC-19, PROTOCOL.md §12): both paths end with both sides
//! recording and pinning each other; a man in the middle, a reused secret,
//! too many attempts and a hogged window are all refused.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::timeout;
use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::pairing::wire::{read_message, write_message, Mode, PairMessage};
use wind_core::pairing::{
    self, crypto, pair_with_offer, pair_with_sas, FailReason, PairError, PairedDevicesFile,
    PairingConfig, PairingEvent, PairingListener, PairingOffer, SasPrompt, WindowEnd,
};
use wind_core::protocol::ErrorCode;
use wind_core::transport::{Endpoint, PeerTrust, PinnedKeys};

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
const T: Duration = Duration::from_secs(10);

/// A device: identity, its paired-device file and its pins.
struct Device {
    id: DeviceIdentity,
    store: Arc<PairedDevicesFile>,
    pins: Arc<PinnedKeys>,
    _dir: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Device {
            id: DeviceIdentity::generate().unwrap(),
            store: Arc::new(PairedDevicesFile::in_dir(dir.path())),
            pins: Arc::new(PinnedKeys::new()),
            _dir: dir,
        }
    }

    fn key(&self) -> PublicKey {
        self.id.public_key()
    }

    fn paired_keys(&self) -> Vec<PublicKey> {
        let list = self.store.load().unwrap();
        assert_eq!(list.skipped, vec![]);
        list.devices.iter().map(|d| d.key).collect()
    }

    fn listener(&self, config: PairingConfig) -> PairingListener {
        PairingListener::bind(
            LOCALHOST,
            &self.id,
            Some("PC"),
            self.store.clone(),
            self.pins.clone(),
            config,
        )
        .unwrap()
    }

    async fn pair(&self, offer: &PairingOffer) -> Result<pairing::PairedDevice, PairError> {
        self.pair_named(offer, "Phone").await
    }

    async fn pair_named(
        &self,
        offer: &PairingOffer,
        name: &str,
    ) -> Result<pairing::PairedDevice, PairError> {
        pair_with_offer(
            &self.id,
            offer,
            Some(name),
            self.store.as_ref(),
            &self.pins,
            &PairingConfig::default(),
        )
        .await
    }
}

fn qr(listener: &PairingListener) -> PairingOffer {
    listener.open_qr(vec![listener.local_addr().unwrap()])
}

async fn event(listener: &mut PairingListener) -> PairingEvent {
    timeout(T, listener.next_event())
        .await
        .expect("an event in time")
        .expect("listener alive")
}

/// No event arrives for a while.
async fn quiet(listener: &mut PairingListener) {
    if let Ok(e) = timeout(Duration::from_millis(300), listener.next_event()).await {
        panic!("unexpected event {e:?}");
    }
}

async fn failed(listener: &mut PairingListener) -> (FailReason, u32) {
    match event(listener).await {
        PairingEvent::AttemptFailed { reason, failures } => (reason, failures),
        e => panic!("expected a failed attempt, got {e:?}"),
    }
}

fn refused_with(r: &Result<pairing::PairedDevice, PairError>, code: ErrorCode) {
    match r {
        Err(PairError::Refused(c)) if *c == code => {}
        other => panic!("expected refusal with {code:?}, got {other:?}"),
    }
}

/// A raw pairing connection (for scripted attackers).
async fn raw(id: &DeviceIdentity, addr: SocketAddr) -> quinn::Connection {
    let mut ep = quinn::Endpoint::client(LOCALHOST).unwrap();
    ep.set_default_client_config(pairing::client_config(id, None).unwrap());
    let conn = ep.connect(addr, "windconnect").unwrap().await.unwrap();
    std::mem::forget(ep);
    conn
}

async fn close_code(conn: &quinn::Connection) -> u64 {
    match timeout(T, conn.closed()).await.unwrap() {
        quinn::ConnectionError::ApplicationClosed(c) => c.error_code.into_inner(),
        e => panic!("expected an application close, got {e:?}"),
    }
}

fn fast() -> PairingConfig {
    PairingConfig {
        step_timeout: Duration::from_millis(300),
        ..PairingConfig::default()
    }
}

// ---------------------------------------------------------------------------
// QR.

#[tokio::test]
async fn qr_pairs_both_sides_and_then_they_connect_as_paired_devices() {
    let (pc, phone) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    // The link survives the QR code: the phone reads it back.
    let scanned = PairingOffer::parse(&offer.to_link()).unwrap();

    let got = phone
        .pair_named(&scanned, "Pixel\u{202E}9\u{0007}")
        .await
        .unwrap();
    assert_eq!(got.key, pc.key());
    assert_eq!(got.name.as_deref(), Some("PC"));
    match event(&mut listener).await {
        PairingEvent::Paired(d) => {
            assert_eq!(d.key, phone.key());
            // Shown to people: no direction or control characters.
            assert_eq!(d.name.as_deref(), Some("Pixel9"));
        }
        e => panic!("{e:?}"),
    }
    assert!(matches!(
        event(&mut listener).await,
        PairingEvent::WindowClosed(WindowEnd::Paired)
    ));
    assert_eq!(pc.paired_keys(), vec![phone.key()]);
    assert_eq!(phone.paired_keys(), vec![pc.key()]);
    assert!(pc.pins.is_trusted(&phone.key()) && phone.pins.is_trusted(&pc.key()));

    // From now on they are paired devices on the main endpoint (§2), both
    // ways, with pins loaded from the files.
    let (pc_pins, phone_pins) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
    pc.store.load().unwrap().pin_all(&pc_pins);
    phone.store.load().unwrap().pin_all(&phone_pins);
    let a = Endpoint::bind(LOCALHOST, &pc.id, pc_pins).unwrap();
    let b = Endpoint::bind(LOCALHOST, &phone.id, phone_pins).unwrap();
    let accept = tokio::spawn({
        let a = a.clone();
        async move { a.accept().await.unwrap().unwrap() }
    });
    let conn = b.connect(a.local_addr().unwrap(), pc.key()).await.unwrap();
    assert_eq!(conn.peer_key(), pc.key());
    assert_eq!(accept.await.unwrap().peer_key(), phone.key());
}

#[tokio::test]
async fn mitm_without_the_secret_is_refused() {
    let (pc, phone, mallory) = (Device::new(), Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    // Mallory sees the address and the key (mDNS), not the QR code.
    let guess = PairingOffer::new(offer.key, [0x42; 16], offer.addrs.clone(), None);
    refused_with(&mallory.pair(&guess).await, ErrorCode::UntrustedPeer);
    assert_eq!(failed(&mut listener).await, (FailReason::BadProof, 1));
    assert_eq!(pc.paired_keys(), vec![]);
    assert_eq!(mallory.paired_keys(), vec![]);
    assert!(!pc.pins.is_trusted(&mallory.key()));
    // The real phone still pairs in the same window.
    phone.pair(&offer).await.unwrap();
    assert_eq!(pc.paired_keys(), vec![phone.key()]);
}

#[tokio::test]
async fn mac_from_another_session_or_for_another_key_is_refused() {
    let (pc, mallory, phone) = (Device::new(), Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    let addr = offer.addrs[0];

    // Session A: Mallory takes its exporter value, then leaves.
    let a = raw(&mallory.id, addr).await;
    let e_a = crypto::exporter(&a).unwrap();
    a.close(0u32.into(), b"");
    assert_eq!(failed(&mut listener).await, (FailReason::Disconnected, 1));

    // Session B: a MAC made for session A (a relayed proof) is refused.
    let b = raw(&mallory.id, addr).await;
    let (mut send, _recv) = b.open_bi().await.unwrap();
    let proof = crypto::client_mac(offer.secret(), &pc.key(), &mallory.key(), &e_a);
    let hello = PairMessage::Hello {
        mode: Mode::Qr,
        name: None,
        proof,
    };
    write_message(&mut send, &hello).await.unwrap();
    assert_eq!(close_code(&b).await, ErrorCode::UntrustedPeer.code());
    assert_eq!(failed(&mut listener).await, (FailReason::BadProof, 2));

    // Session C: the right session, but the MAC names another client key
    // (the phone's): the key comes from TLS, so it doesn't match.
    let c = raw(&mallory.id, addr).await;
    let e_c = crypto::exporter(&c).unwrap();
    let (mut send, _recv) = c.open_bi().await.unwrap();
    let proof = crypto::client_mac(offer.secret(), &pc.key(), &phone.key(), &e_c);
    write_message(
        &mut send,
        &PairMessage::Hello {
            mode: Mode::Qr,
            name: None,
            proof,
        },
    )
    .await
    .unwrap();
    assert_eq!(close_code(&c).await, ErrorCode::UntrustedPeer.code());
    assert_eq!(failed(&mut listener).await, (FailReason::BadProof, 3));
    assert!(matches!(
        event(&mut listener).await,
        PairingEvent::WindowClosed(WindowEnd::TooManyAttempts)
    ));
    assert_eq!(pc.paired_keys(), vec![]);
}

#[tokio::test]
async fn server_with_another_key_than_the_link_is_refused_in_tls() {
    let (pc, impostor, phone) = (Device::new(), Device::new(), Device::new());
    let real = pc.listener(PairingConfig::default());
    let offer = qr(&real);
    // The impostor answers at an address the phone tries, with its own key.
    let mut fake = impostor.listener(PairingConfig::default());
    let _ = qr(&fake);
    let hijacked = PairingOffer::new(
        offer.key,
        *offer.secret(),
        vec![fake.local_addr().unwrap()],
        None,
    );
    let r = phone.pair(&hijacked).await;
    assert!(matches!(&r, Err(e) if e.is_wrong_server_key()), "{r:?}");
    // The impostor never got as far as an attempt.
    quiet(&mut fake).await;
    assert_eq!(phone.paired_keys(), vec![]);
    assert_eq!(impostor.paired_keys(), vec![]);
}

#[tokio::test]
async fn secret_cannot_be_used_twice() {
    let (pc, phone, other) = (Device::new(), Device::new(), Device::new());
    let listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    phone.pair(&offer).await.unwrap();
    refused_with(&phone.pair(&offer).await, ErrorCode::GoingAway);
    refused_with(&other.pair(&offer).await, ErrorCode::GoingAway);
    assert_eq!(pc.paired_keys(), vec![phone.key()]);
    assert_eq!(other.paired_keys(), vec![]);
    // A new window has a new secret; the old link stays dead.
    let fresh = qr(&listener);
    assert_ne!(fresh.secret(), offer.secret());
    refused_with(&other.pair(&offer).await, ErrorCode::UntrustedPeer);
}

#[tokio::test]
async fn three_failed_attempts_close_the_window() {
    let (pc, phone, mallory) = (Device::new(), Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    for n in 1..=3 {
        let guess = PairingOffer::new(offer.key, [n; 16], offer.addrs.clone(), None);
        refused_with(&mallory.pair(&guess).await, ErrorCode::UntrustedPeer);
        assert_eq!(
            failed(&mut listener).await,
            (FailReason::BadProof, n as u32)
        );
    }
    assert!(matches!(
        event(&mut listener).await,
        PairingEvent::WindowClosed(WindowEnd::TooManyAttempts)
    ));
    // Even the right secret is refused now, before any proof is read.
    refused_with(&phone.pair(&offer).await, ErrorCode::RateLimited);
    assert_eq!(pc.paired_keys(), vec![]);
}

#[tokio::test]
async fn silent_dropped_or_garbled_connections_count_as_attempts() {
    let (pc, phone, mallory) = (Device::new(), Device::new(), Device::new());
    let mut listener = pc.listener(fast());
    let offer = qr(&listener);
    let addr = offer.addrs[0];

    // Holds the window and says nothing.
    let silent = raw(&mallory.id, addr).await;
    assert_eq!(close_code(&silent).await, ErrorCode::HelloTimeout.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Timeout, 1));

    // Leaves right after TLS.
    let dropped = raw(&mallory.id, addr).await;
    dropped.close(0u32.into(), b"");
    assert_eq!(failed(&mut listener).await, (FailReason::Disconnected, 2));

    // Sends a frame that isn't one.
    let garbled = raw(&mallory.id, addr).await;
    let (mut send, _recv) = garbled.open_bi().await.unwrap();
    send.write_all(&[0x05, 0xff, 0xff, 0xff, 0xff, 0xff])
        .await
        .unwrap();
    assert_eq!(close_code(&garbled).await, ErrorCode::MalformedFrame.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Malformed, 3));
    assert!(matches!(
        event(&mut listener).await,
        PairingEvent::WindowClosed(WindowEnd::TooManyAttempts)
    ));
    refused_with(&phone.pair(&offer).await, ErrorCode::RateLimited);
}

#[tokio::test]
async fn second_connection_during_an_attempt_is_turned_away_without_counting() {
    let (pc, mallory) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig {
        step_timeout: Duration::from_secs(1),
        ..PairingConfig::default()
    });
    let offer = qr(&listener);
    let first = raw(&mallory.id, offer.addrs[0]).await;
    let second = raw(&mallory.id, offer.addrs[0]).await;
    assert_eq!(close_code(&second).await, ErrorCode::RateLimited.code());
    // Only the first one, when it times out, is an attempt.
    assert_eq!(close_code(&first).await, ErrorCode::HelloTimeout.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Timeout, 1));
    quiet(&mut listener).await;
}

#[tokio::test]
async fn window_expires_on_its_own_and_its_secret_is_dead() {
    let (pc, phone) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig {
        ttl: Duration::from_millis(300),
        ..PairingConfig::default()
    });
    let offer = qr(&listener);
    // No connection at all: the timer closes the window.
    assert!(matches!(
        event(&mut listener).await,
        PairingEvent::WindowClosed(WindowEnd::Expired)
    ));
    refused_with(&phone.pair(&offer).await, ErrorCode::GoingAway);
}

#[tokio::test]
async fn without_a_window_the_connection_is_closed_before_any_frame() {
    let (pc, mallory) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let conn = raw(&mallory.id, listener.local_addr().unwrap()).await;
    // We send nothing; the answer comes anyway.
    assert_eq!(close_code(&conn).await, ErrorCode::GoingAway.code());
    quiet(&mut listener).await;
}

#[tokio::test]
async fn qr_window_refuses_a_sas_hello_and_the_reverse() {
    let (pc, mallory) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let offer = qr(&listener);
    let conn = raw(&mallory.id, offer.addrs[0]).await;
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    let hello = PairMessage::Hello {
        mode: Mode::Sas,
        name: None,
        proof: [0; 32],
    };
    write_message(&mut send, &hello).await.unwrap();
    assert_eq!(close_code(&conn).await, ErrorCode::ProtocolViolation.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Violation, 1));

    listener.open_sas();
    let conn = raw(&mallory.id, offer.addrs[0]).await;
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    let hello = PairMessage::Hello {
        mode: Mode::Qr,
        name: None,
        proof: crypto::client_mac(offer.secret(), &pc.key(), &mallory.key(), &[0; 32]),
    };
    write_message(&mut send, &hello).await.unwrap();
    assert_eq!(close_code(&conn).await, ErrorCode::ProtocolViolation.code());
}

// ---------------------------------------------------------------------------
// TLS of the pairing endpoint.

mod raw_tls {
    use super::*;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::client::{ClientSessionStore, Tls12ClientSessionValue, Tls13ClientSessionValue};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, NamedGroup, SignatureScheme};

    /// Takes any server certificate (these tests look at other things).
    #[derive(Debug)]
    pub struct AnyServer;

    impl ServerCertVerifier for AnyServer {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    }

    /// Counts the session tickets a server hands out.
    #[derive(Debug, Default)]
    pub struct CountingStore {
        pub tickets: AtomicUsize,
    }

    impl ClientSessionStore for CountingStore {
        fn set_kx_hint(&self, _: ServerName<'static>, _: NamedGroup) {}
        fn kx_hint(&self, _: &ServerName<'_>) -> Option<NamedGroup> {
            None
        }
        fn set_tls12_session(&self, _: ServerName<'static>, _: Tls12ClientSessionValue) {}
        fn tls12_session(&self, _: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
            None
        }
        fn remove_tls12_session(&self, _: &ServerName<'static>) {}
        fn insert_tls13_ticket(&self, _: ServerName<'static>, _: Tls13ClientSessionValue) {
            self.tickets.fetch_add(1, Ordering::SeqCst);
        }
        fn take_tls13_ticket(&self, _: &ServerName<'static>) -> Option<Tls13ClientSessionValue> {
            None
        }
    }

    /// A client config: ALPN `alpn`, our certificate unless `anonymous`,
    /// tickets counted in `store`.
    pub fn config(
        id: &DeviceIdentity,
        alpn: &[u8],
        anonymous: bool,
        store: Arc<CountingStore>,
    ) -> quinn::ClientConfig {
        let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyServer));
        let mut tls = if anonymous {
            builder.with_no_client_auth()
        } else {
            builder
                .with_client_auth_cert(vec![id.certificate_der()], id.private_key_der().unwrap())
                .unwrap()
        };
        tls.alpn_protocols = vec![alpn.to_vec()];
        tls.resumption = rustls::client::Resumption::store(store);
        quinn::ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
        ))
    }

    pub async fn connect(
        config: quinn::ClientConfig,
        addr: SocketAddr,
    ) -> Result<quinn::Connection, quinn::ConnectionError> {
        let mut ep = quinn::Endpoint::client(LOCALHOST).unwrap();
        ep.set_default_client_config(config);
        let r = ep.connect(addr, "windconnect").unwrap().await;
        std::mem::forget(ep);
        r
    }
}

#[tokio::test]
async fn pairing_endpoint_issues_no_session_tickets() {
    let (pc, client) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let _offer = qr(&listener);
    let store = Arc::new(raw_tls::CountingStore::default());
    for _ in 0..2 {
        let conn = raw_tls::connect(
            raw_tls::config(
                &client.id,
                wind_core::protocol::PAIR_ALPN,
                false,
                store.clone(),
            ),
            listener.local_addr().unwrap(),
        )
        .await
        .unwrap();
        // Tickets come after the handshake: give them time to arrive.
        tokio::time::sleep(Duration::from_millis(200)).await;
        conn.close(0u32.into(), b"");
        let _ = failed(&mut listener).await;
    }
    assert_eq!(store.tickets.load(Ordering::SeqCst), 0);

    // Control: the same probe does see the main endpoint's tickets, so a
    // zero above means none were sent.
    let pins = Arc::new(PinnedKeys::new());
    pins.pin(client.key());
    let main = Endpoint::bind(LOCALHOST, &pc.id, pins).unwrap();
    let accept = tokio::spawn({
        let main = main.clone();
        async move { main.accept().await }
    });
    let main_store = Arc::new(raw_tls::CountingStore::default());
    let conn = raw_tls::connect(
        raw_tls::config(
            &client.id,
            wind_core::protocol::ALPN,
            false,
            main_store.clone(),
        ),
        main.local_addr().unwrap(),
    )
    .await
    .unwrap();
    let _server_side = accept.await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(main_store.tickets.load(Ordering::SeqCst) > 0);
    drop(conn);
}

#[tokio::test]
async fn client_without_a_certificate_is_refused_in_tls() {
    let (pc, client) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    let _offer = qr(&listener);
    let store = Arc::new(raw_tls::CountingStore::default());
    let r = raw_tls::connect(
        raw_tls::config(&client.id, wind_core::protocol::PAIR_ALPN, true, store),
        listener.local_addr().unwrap(),
    )
    .await;
    // TLS 1.3: the client may finish its side before the server's refusal
    // arrives; either way the connection dies with a TLS error, never an
    // application close, and no attempt is counted.
    let err = match r {
        Err(e) => e,
        Ok(conn) => timeout(T, conn.closed()).await.unwrap(),
    };
    assert!(
        matches!(
            err,
            quinn::ConnectionError::TransportError(_) | quinn::ConnectionError::ConnectionClosed(_)
        ),
        "{err:?}"
    );
    quiet(&mut listener).await;
    assert_eq!(pc.paired_keys(), vec![]);
}

// ---------------------------------------------------------------------------
// SAS.

/// The phone's side of a SAS pairing, its prompts sent to `prompts`.
fn sas_client(
    phone: &Device,
    addr: SocketAddr,
    prompts: mpsc::UnboundedSender<SasPrompt>,
) -> tokio::task::JoinHandle<Result<pairing::PairedDevice, PairError>> {
    let id = DeviceIdentity::from_pkcs8_pem(&phone.id.to_pkcs8_pem().unwrap()).unwrap();
    let (store, pins) = (phone.store.clone(), phone.pins.clone());
    tokio::spawn(async move {
        pair_with_sas(
            &id,
            addr,
            Some("Phone"),
            store.as_ref(),
            &pins,
            &PairingConfig::default(),
            move |p| prompts.send(p).unwrap(),
        )
        .await
    })
}

async fn server_prompt(listener: &mut PairingListener) -> SasPrompt {
    match event(listener).await {
        PairingEvent::Sas(p) => p,
        e => panic!("expected a code to compare, got {e:?}"),
    }
}

#[tokio::test]
async fn sas_pairs_when_both_people_accept_the_same_code() {
    let (pc, phone) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    listener.open_sas();
    let (tx, mut prompts) = mpsc::unbounded_channel();
    let client = sas_client(&phone, listener.local_addr().unwrap(), tx);

    let on_pc = server_prompt(&mut listener).await;
    let on_phone = timeout(T, prompts.recv()).await.unwrap().unwrap();
    assert_eq!(on_pc.code(), on_phone.code(), "both screens show one code");
    assert_eq!(on_pc.code_text().len(), 7);
    assert_eq!(on_pc.peer_key(), phone.key());
    assert_eq!(on_phone.peer_key(), pc.key());
    assert_eq!(on_pc.peer_name(), Some("Phone"));
    assert_eq!(on_phone.peer_name(), Some("PC"));
    on_phone.accept();
    on_pc.accept();

    let got = timeout(T, client).await.unwrap().unwrap().unwrap();
    assert_eq!(got.key, pc.key());
    assert!(matches!(event(&mut listener).await, PairingEvent::Paired(d) if d.key == phone.key()));
    assert_eq!(pc.paired_keys(), vec![phone.key()]);
    assert_eq!(phone.paired_keys(), vec![pc.key()]);
}

#[tokio::test]
async fn sas_rejected_on_either_side_pairs_nobody() {
    for reject_on_pc in [true, false] {
        let (pc, phone) = (Device::new(), Device::new());
        let mut listener = pc.listener(PairingConfig::default());
        listener.open_sas();
        let (tx, mut prompts) = mpsc::unbounded_channel();
        let client = sas_client(&phone, listener.local_addr().unwrap(), tx);
        let on_pc = server_prompt(&mut listener).await;
        let on_phone = timeout(T, prompts.recv()).await.unwrap().unwrap();
        if reject_on_pc {
            on_phone.accept();
            on_pc.reject();
        } else {
            on_phone.reject();
            // The PC person hasn't answered; the attempt ends anyway.
            std::mem::forget(on_pc);
        }
        let r = timeout(T, client).await.unwrap().unwrap();
        assert!(matches!(r, Err(PairError::Rejected)), "{r:?}");
        assert_eq!(failed(&mut listener).await, (FailReason::Rejected, 1));
        assert_eq!(pc.paired_keys(), vec![]);
        assert_eq!(phone.paired_keys(), vec![]);
    }
}

/// A scripted SAS client up to the server's nonce: (connection, send, recv,
/// N_s).
async fn sas_until_nonce(
    mallory: &Device,
    addr: SocketAddr,
    commitment: [u8; 32],
) -> (quinn::Connection, quinn::SendStream, quinn::RecvStream) {
    let conn = raw(&mallory.id, addr).await;
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let hello = PairMessage::Hello {
        mode: Mode::Sas,
        name: None,
        proof: commitment,
    };
    write_message(&mut send, &hello).await.unwrap();
    let nonce = read_message(&mut recv).await.unwrap();
    assert!(matches!(nonce, PairMessage::Nonce { .. }), "{nonce:?}");
    (conn, send, recv)
}

#[tokio::test]
async fn sas_reveal_that_does_not_open_the_commitment_is_refused() {
    // A client that would pick N_c after seeing N_s (to steer the code)
    // must reveal something other than what it committed to.
    let (pc, mallory) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    listener.open_sas();
    let committed = [1u8; 32];
    let commitment = crypto::commit(&mallory.key(), &pc.key(), &committed);
    let (conn, mut send, _recv) =
        sas_until_nonce(&mallory, listener.local_addr().unwrap(), commitment).await;
    write_message(&mut send, &PairMessage::Reveal { nonce: [2; 32] })
        .await
        .unwrap();
    assert_eq!(close_code(&conn).await, ErrorCode::ProtocolViolation.code());
    // No code was ever shown.
    assert_eq!(failed(&mut listener).await, (FailReason::Violation, 1));
}

#[tokio::test]
async fn sas_messages_out_of_order_are_refused() {
    let (pc, mallory) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig::default());
    listener.open_sas();
    let addr = listener.local_addr().unwrap();

    // A reveal before any hello.
    let conn = raw(&mallory.id, addr).await;
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    write_message(&mut send, &PairMessage::Reveal { nonce: [1; 32] })
        .await
        .unwrap();
    assert_eq!(close_code(&conn).await, ErrorCode::ProtocolViolation.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Violation, 1));

    // A confirm where the reveal belongs.
    let commitment = crypto::commit(&mallory.key(), &pc.key(), &[1; 32]);
    let (conn, mut send, _recv) = sas_until_nonce(&mallory, addr, commitment).await;
    write_message(&mut send, &PairMessage::Confirm { accepted: true })
        .await
        .unwrap();
    assert_eq!(close_code(&conn).await, ErrorCode::ProtocolViolation.code());
    assert_eq!(failed(&mut listener).await, (FailReason::Violation, 2));
}

#[tokio::test]
async fn sas_through_a_relay_shows_different_codes() {
    // Mallory sits between the phone and the PC, running the protocol
    // honestly with each: the phone pairs "with the PC" at Mallory's
    // address, and Mallory pairs with the PC. The two codes come from two
    // TLS sessions and nonces neither side of Mallory controls alone, so
    // they differ (but for a 1 in 10^6 chance), and the people see it.
    let (pc, phone, mallory) = (Device::new(), Device::new(), Device::new());
    let mut pc_listener = pc.listener(PairingConfig::default());
    pc_listener.open_sas();
    let mut mallory_listener = mallory.listener(PairingConfig::default());
    mallory_listener.open_sas();

    let (tx, mut phone_prompts) = mpsc::unbounded_channel();
    let phone_side = sas_client(&phone, mallory_listener.local_addr().unwrap(), tx);
    let (tx, mut mallory_prompts) = mpsc::unbounded_channel();
    let mallory_side = sas_client(&mallory, pc_listener.local_addr().unwrap(), tx);

    let on_phone = timeout(T, phone_prompts.recv()).await.unwrap().unwrap();
    let on_pc = server_prompt(&mut pc_listener).await;
    assert_ne!(on_phone.code(), on_pc.code());
    // And the PC sees Mallory's key, not the phone's.
    assert_eq!(on_pc.peer_key(), mallory.key());

    // The people see different codes and say no.
    on_phone.reject();
    on_pc.reject();
    let _ = server_prompt(&mut mallory_listener).await;
    let _ = timeout(T, mallory_prompts.recv()).await;
    assert!(timeout(T, phone_side).await.unwrap().unwrap().is_err());
    assert!(timeout(T, mallory_side).await.unwrap().unwrap().is_err());
    assert_eq!(pc.paired_keys(), vec![]);
    assert_eq!(phone.paired_keys(), vec![]);
}

/// WN-82 (§12.4): the listener tells its advertiser (mDNS TXT `pair`) its
/// port while a SAS window is open, and `None` however that window ends;
/// a QR window is never advertised.
#[tokio::test]
async fn sas_window_is_advertised_only_while_open() {
    let (pc, phone) = (Device::new(), Device::new());
    let mut listener = pc.listener(PairingConfig {
        ttl: Duration::from_millis(300),
        ..PairingConfig::default()
    });
    let port = Some(listener.local_addr().unwrap().port());
    let told = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = told.clone();
    listener.advertise_with(Arc::new(move |p| log.lock().unwrap().push(p)));
    let take = || std::mem::take(&mut *told.lock().unwrap());
    assert_eq!(take(), vec![], "no window: nothing to withdraw");

    listener.open_sas();
    listener.open_sas();
    assert_eq!(take(), vec![port], "told once per change");
    listener.open_qr(vec![listener.local_addr().unwrap()]);
    assert_eq!(take(), vec![None], "QR replaced SAS");
    listener.open_sas();
    listener.close_window();
    assert_eq!(take(), vec![port, None], "closed");

    listener.open_sas();
    // Earlier windows' Replaced/Closed events are still queued.
    timeout(T, async {
        while !matches!(
            event(&mut listener).await,
            PairingEvent::WindowClosed(WindowEnd::Expired)
        ) {}
    })
    .await
    .expect("the SAS window expires");
    assert_eq!(take(), vec![port, None], "expired");

    listener.open_sas();
    let (tx, mut prompts) = mpsc::unbounded_channel();
    let client = sas_client(&phone, listener.local_addr().unwrap(), tx);
    let on_pc = server_prompt(&mut listener).await;
    timeout(T, prompts.recv()).await.unwrap().unwrap().accept();
    on_pc.accept();
    timeout(T, client).await.unwrap().unwrap().unwrap();
    assert_eq!(take(), vec![port, None], "paired");

    listener.open_sas();
    let late = Arc::new(std::sync::Mutex::new(Vec::new()));
    let late_log = late.clone();
    listener.advertise_with(Arc::new(move |p| late_log.lock().unwrap().push(p)));
    assert_eq!(take(), vec![port, None], "a replaced advertiser withdraws");
    assert_eq!(
        *late.lock().unwrap(),
        vec![port],
        "a new one learns the open window"
    );
    drop(listener);
    assert_eq!(*late.lock().unwrap(), vec![port, None], "dropped listener");
}
