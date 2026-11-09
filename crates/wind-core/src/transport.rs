//! QUIC transport with pinned keys (PROTOCOL.md §2, WC-10).
//!
//! Every device is both server and client on one [`Endpoint`]. TLS 1.3 is
//! mutual: each side presents its self-signed device certificate (WC-07), and
//! a certificate is accepted only when its Ed25519 key is pinned, that is,
//! the key of a paired device ([`PeerTrust`]). Chains, names and dates of
//! certificates play no part; the TLS handshake signature proves that the
//! peer holds the pinned key.
//!
//! An unpinned key is refused during the TLS handshake, before any
//! application data, so the peer sees a TLS alert, not an application close
//! code. Locally that shows as [`ConnectError::is_untrusted_peer`].
//!
//! Cryptography is rustls (with ring) and quinn; this module only decides
//! which keys to trust.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

use crate::identity::{DeviceIdentity, IdentityError, PublicKey, CERT_SERVER_NAME};
use crate::protocol::{ErrorCode, ALPN};

/// QUIC keep-alive: an idle connection sends a PING this often, so NAT
/// bindings stay open and a dead path is noticed (WC-12). Battery tuning is
/// WC-30's.
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
/// A connection with no packets from the peer for this long is gone (three
/// missed keep-alives).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// One reconnection attempt (`Session::reconnect`) gives up after this.
pub const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
/// TLS sessions remembered per peer, for resumption and 0-RTT.
const SESSIONS_PER_PEER: usize = 4;

/// Endpoint settings that may differ between devices.
#[derive(Debug, Clone, Copy)]
pub struct EndpointOptions {
    /// Accept QUIC datagrams (RFC 9221) and advertise
    /// `max_datagram_frame_size` (PROTOCOL.md §2: SHOULD, with `input/1`).
    /// Without them, input motion goes on the `input` stream (§8.7).
    pub datagrams: bool,
}

impl Default for EndpointOptions {
    fn default() -> Self {
        EndpointOptions { datagrams: true }
    }
}

/// Transport settings both sides use.
fn transport_config(options: EndpointOptions) -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    if !options.datagrams {
        t.datagram_receive_buffer_size(None);
    }
    t.keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
    t.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(IDLE_TIMEOUT).expect("45 s fits a QUIC idle timeout"),
    ));
    Arc::new(t)
}

/// Which peer keys are pinned (paired devices). Pairing (WC-19) and the
/// device list (WC-28) fill it; this module only asks.
pub trait PeerTrust: Send + Sync + 'static {
    fn is_trusted(&self, key: &PublicKey) -> bool;
}

/// A set of pinned keys in memory.
#[derive(Debug, Default)]
pub struct PinnedKeys {
    keys: RwLock<HashSet<PublicKey>>,
}

impl PinnedKeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins `key`; true if it wasn't pinned yet.
    pub fn pin(&self, key: PublicKey) -> bool {
        self.keys
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key)
    }

    /// Unpins `key`; new connections with it are refused from now on.
    pub fn unpin(&self, key: &PublicKey) -> bool {
        self.keys
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key)
    }
}

impl PeerTrust for PinnedKeys {
    fn is_trusted(&self, key: &PublicKey) -> bool {
        self.keys
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(key)
    }
}

/// The Ed25519 key in a certificate, if it has one.
pub fn certificate_key(cert: &CertificateDer<'_>) -> Option<PublicKey> {
    let ee = webpki::EndEntityCert::try_from(cert).ok()?;
    let spki = ee.subject_public_key_info();
    let spki = spki.as_ref();
    let prefix_len = spki.len().checked_sub(32)?;
    let mut raw = [0u8; 32];
    raw.copy_from_slice(&spki[prefix_len..]);
    let key = PublicKey::from_bytes(&raw)?;
    // Exactly the Ed25519 SPKI (RFC 8410): any other algorithm differs here.
    (key.spki_der() == spki).then_some(key)
}

/// Only Ed25519 handshake signatures: the only key type a device has.
static ED25519_ONLY: WebPkiSupportedAlgorithms = WebPkiSupportedAlgorithms {
    all: &[webpki::ring::ED25519],
    mapping: &[(SignatureScheme::ED25519, &[webpki::ring::ED25519])],
};

fn untrusted() -> rustls::Error {
    rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

/// Decides on a peer certificate: its key must be pinned (and, when we
/// dial, be the key we meant to reach).
#[derive(Clone)]
struct PinnedVerifier {
    trust: Arc<dyn PeerTrust>,
    expected: Option<PublicKey>,
}

impl fmt::Debug for PinnedVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedVerifier")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl PinnedVerifier {
    fn check(&self, end_entity: &CertificateDer<'_>) -> Result<(), rustls::Error> {
        let key = certificate_key(end_entity).ok_or_else(untrusted)?;
        if self.expected.is_some_and(|e| e != key) || !self.trust.is_trusted(&key) {
            return Err(untrusted());
        }
        Ok(())
    }

    fn tls13(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &ED25519_ONLY)
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        self.check(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // TLS 1.2 is never offered.
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.tls13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

impl ClientCertVerifier for PinnedVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.check(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls13RequiredForQuic,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.tls13(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// Why a connection couldn't be made or accepted.
#[derive(Debug)]
pub enum ConnectError {
    /// We were asked to dial a key that isn't pinned; nothing was sent.
    NotPaired(PublicKey),
    /// A resumed session (which skips certificate checks) came from a key
    /// that is no longer pinned; closed with `UNTRUSTED_PEER`.
    Unpinned(PublicKey),
    /// The connection failed or was refused (by us or the peer).
    Connection(quinn::ConnectionError),
    /// The connection could not start (address, endpoint closed).
    Connect(quinn::ConnectError),
    /// The handshake finished without a usable peer key (should not happen).
    NoPeerKey,
    /// Our own TLS configuration was refused.
    Tls(rustls::Error),
}

/// TLS alert `access_denied` (49) as a QUIC crypto error (0x100 + alert):
/// what [`PinnedVerifier`] sends for an unpinned key.
const UNTRUSTED_ALERT: u64 = 0x100 + 49;

impl ConnectError {
    /// The handshake failed because a key wasn't pinned, on either side.
    pub fn is_untrusted_peer(&self) -> bool {
        match self {
            ConnectError::NotPaired(_) | ConnectError::Unpinned(_) => true,
            ConnectError::Connection(e) => is_untrusted_close(e),
            _ => false,
        }
    }
}

/// A connection closed because a key wasn't pinned, by us or the peer.
///
/// A refused client may see this after `connect` returned: TLS 1.3 sends
/// the client certificate in the client's last flight, so the server's
/// verdict can come after the client has finished its handshake. The server
/// never accepts such a connection, and no application data reaches it.
pub fn is_untrusted_close(e: &quinn::ConnectionError) -> bool {
    match e {
        quinn::ConnectionError::TransportError(t) => u64::from(t.code) == UNTRUSTED_ALERT,
        quinn::ConnectionError::ConnectionClosed(c) => u64::from(c.error_code) == UNTRUSTED_ALERT,
        quinn::ConnectionError::ApplicationClosed(c) => {
            c.error_code.into_inner() == ErrorCode::UntrustedPeer.code()
        }
        _ => false,
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::NotPaired(k) => write!(f, "peer {} is not paired", k.fingerprint()),
            ConnectError::Unpinned(k) => {
                write!(f, "peer {} is no longer paired", k.fingerprint())
            }
            ConnectError::Connection(e) if is_untrusted_close(e) => {
                write!(f, "peer key not pinned: {e}")
            }
            ConnectError::Connection(e) => write!(f, "connection failed: {e}"),
            ConnectError::Connect(e) => write!(f, "can't connect: {e}"),
            ConnectError::NoPeerKey => f.write_str("peer presented no device key"),
            ConnectError::Tls(e) => write!(f, "TLS setup: {e}"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl From<quinn::ConnectionError> for ConnectError {
    fn from(e: quinn::ConnectionError) -> Self {
        ConnectError::Connection(e)
    }
}

/// Why an endpoint couldn't be set up.
#[derive(Debug)]
pub enum EndpointError {
    Identity(IdentityError),
    Tls(rustls::Error),
    Io(std::io::Error),
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EndpointError::Identity(e) => write!(f, "{e}"),
            EndpointError::Tls(e) => write!(f, "TLS setup: {e}"),
            EndpointError::Io(e) => write!(f, "socket: {e}"),
        }
    }
}

impl std::error::Error for EndpointError {}

/// A QUIC connection to a device whose key is pinned.
#[derive(Debug, Clone)]
pub struct Connection {
    inner: quinn::Connection,
    peer: PublicKey,
}

impl Connection {
    /// The peer's device key, checked against the pinned keys.
    pub fn peer_key(&self) -> PublicKey {
        self.peer
    }

    pub fn quinn(&self) -> &quinn::Connection {
        &self.inner
    }

    /// Closes the connection with a protocol error code (§10).
    pub fn close(&self, code: ErrorCode, reason: &[u8]) {
        self.inner
            .close(quinn::VarInt::from_u32(code.code() as u32), reason);
    }

    /// The peer's key, checked again against the pinned keys: a resumed
    /// TLS session skips the certificate verifier, so a key unpinned since
    /// the ticket was issued is caught here (closed with `UNTRUSTED_PEER`).
    fn from_quinn(inner: quinn::Connection, trust: &dyn PeerTrust) -> Result<Self, ConnectError> {
        let peer = inner
            .peer_identity()
            .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
            .and_then(|certs| certs.first().and_then(certificate_key))
            .ok_or(ConnectError::NoPeerKey)?;
        if !trust.is_trusted(&peer) {
            inner.close(
                quinn::VarInt::from_u32(ErrorCode::UntrustedPeer.code() as u32),
                b"",
            );
            return Err(ConnectError::Unpinned(peer));
        }
        Ok(Connection { inner, peer })
    }
}

/// A device's QUIC endpoint: it listens and dials, with its identity and
/// the pinned keys.
#[derive(Clone)]
pub struct Endpoint {
    inner: quinn::Endpoint,
    trust: Arc<dyn PeerTrust>,
    certs: Vec<CertificateDer<'static>>,
    key: Arc<rustls_pki_types::PrivateKeyDer<'static>>,
    provider: Arc<CryptoProvider>,
    /// One client configuration per peer, each with its own TLS session
    /// store, so a ticket is only ever offered to the peer that issued it
    /// (all peers share one server name, which keeps the name in the
    /// ClientHello free of any device identifier).
    clients: Arc<Mutex<HashMap<PublicKey, quinn::ClientConfig>>>,
    options: EndpointOptions,
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("local_addr", &self.inner.local_addr().ok())
            .finish_non_exhaustive()
    }
}

impl Endpoint {
    /// An endpoint on `addr` (port 0 picks one; the default is
    /// [`crate::protocol::DEFAULT_PORT`]). Must be called inside a Tokio
    /// runtime.
    pub fn bind(
        addr: SocketAddr,
        identity: &DeviceIdentity,
        trust: Arc<dyn PeerTrust>,
    ) -> Result<Self, EndpointError> {
        Self::bind_with(addr, identity, trust, EndpointOptions::default())
    }

    /// [`bind`](Self::bind) with explicit options.
    pub fn bind_with(
        addr: SocketAddr,
        identity: &DeviceIdentity,
        trust: Arc<dyn PeerTrust>,
        options: EndpointOptions,
    ) -> Result<Self, EndpointError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let certs = vec![identity.certificate_der()];
        let key = identity
            .private_key_der()
            .map_err(EndpointError::Identity)?;

        let verifier = Arc::new(PinnedVerifier {
            trust: trust.clone(),
            expected: None,
        });
        let mut tls = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(EndpointError::Tls)?
            .with_client_cert_verifier(verifier)
            .with_single_cert(certs.clone(), key.clone_key())
            .map_err(EndpointError::Tls)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        // 0-RTT (PROTOCOL.md §2): QUIC allows only 0 or u32::MAX here.
        tls.max_early_data_size = u32::MAX;
        let quic = QuicServerConfig::try_from(tls)
            .map_err(|e| EndpointError::Tls(rustls::Error::General(e.to_string())))?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(quic));
        server.transport_config(transport_config(options));
        // A peer whose address changes (network switch) keeps its connection.
        server.migration(true);

        let inner = quinn::Endpoint::server(server, addr).map_err(EndpointError::Io)?;
        Ok(Endpoint {
            inner,
            trust,
            certs,
            key: Arc::new(key),
            provider,
            clients: Arc::new(Mutex::new(HashMap::new())),
            options,
        })
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// The client configuration for `peer`, made once and kept, so its TLS
    /// sessions can be resumed.
    fn client_config(&self, peer: PublicKey) -> Result<quinn::ClientConfig, ConnectError> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(config) = clients.get(&peer) {
            return Ok(config.clone());
        }
        let verifier = Arc::new(PinnedVerifier {
            trust: self.trust.clone(),
            expected: Some(peer),
        });
        let mut tls = rustls::ClientConfig::builder_with_provider(self.provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("ring supports TLS 1.3")
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(self.certs.clone(), self.key.clone_key())
            .map_err(ConnectError::Tls)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        tls.resumption = rustls::client::Resumption::in_memory_sessions(SESSIONS_PER_PEER);
        tls.enable_early_data = true;
        let quic = QuicClientConfig::try_from(tls).expect("TLS 1.3 with an initial suite");
        let mut config = quinn::ClientConfig::new(Arc::new(quic));
        config.transport_config(transport_config(self.options));
        clients.insert(peer, config.clone());
        Ok(config)
    }

    /// Starts connecting to the device with key `peer` at `addr`. With a
    /// remembered TLS session the connection is usable for 0-RTT data at
    /// once ([`Dialing::early`]); see [`Dialing::established`].
    pub fn dial(&self, addr: SocketAddr, peer: PublicKey) -> Result<Dialing, ConnectError> {
        if !self.trust.is_trusted(&peer) {
            return Err(ConnectError::NotPaired(peer));
        }
        let connecting = self
            .inner
            .connect_with(self.client_config(peer)?, addr, CERT_SERVER_NAME)
            .map_err(ConnectError::Connect)?;
        let state = match connecting.into_0rtt() {
            Ok((conn, accepted)) => DialState::ZeroRtt(conn, accepted),
            Err(connecting) => DialState::Full(connecting),
        };
        Ok(Dialing {
            state,
            trust: self.trust.clone(),
        })
    }

    /// Connects to the device with key `peer` at `addr`. The peer must be
    /// pinned, and must present exactly that key. If the peer doesn't pin
    /// our key, the connection may close right after this returns; see
    /// [`is_untrusted_close`].
    pub async fn connect(
        &self,
        addr: SocketAddr,
        peer: PublicKey,
    ) -> Result<Connection, ConnectError> {
        Ok(self.dial(addr, peer)?.established().await?.0)
    }

    /// Moves the endpoint to a new UDP socket on `addr` (port 0 picks one),
    /// e.g. after the network changed. Every connection migrates to the new
    /// path (RFC 9000 §9) without a new handshake. Returns the new address.
    pub fn rebind(&self, addr: SocketAddr) -> std::io::Result<SocketAddr> {
        let socket = std::net::UdpSocket::bind(addr)?;
        self.inner.rebind(socket)?;
        self.inner.local_addr()
    }

    /// The next incoming connection: `None` once the endpoint is closed. A
    /// refused or failed handshake is `Some(Err(_))`; keep accepting.
    pub async fn accept(&self) -> Option<Result<Connection, ConnectError>> {
        let incoming = self.inner.accept().await?;
        Some(match incoming.await {
            Ok(conn) => Connection::from_quinn(conn, self.trust.as_ref()),
            Err(e) => Err(e.into()),
        })
    }

    /// Closes every connection with `GOING_AWAY` and stops listening.
    pub fn close(&self) {
        self.inner.close(
            quinn::VarInt::from_u32(ErrorCode::GoingAway.code() as u32),
            b"",
        );
    }

    /// Connections the endpoint still holds (including ones closing).
    pub fn open_connections(&self) -> usize {
        self.inner.open_connections()
    }

    /// Waits until every connection is closed.
    pub async fn wait_idle(&self) {
        self.inner.wait_idle().await;
    }
}

enum DialState {
    ZeroRtt(quinn::Connection, quinn::ZeroRttAccepted),
    Full(quinn::Connecting),
}

/// A connection being set up by [`Endpoint::dial`].
pub struct Dialing {
    state: DialState,
    trust: Arc<dyn PeerTrust>,
}

impl fmt::Debug for Dialing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dialing")
            .field("zero_rtt", &self.early().is_some())
            .finish_non_exhaustive()
    }
}

impl Dialing {
    /// The connection for 0-RTT data, when a TLS session was resumed. Only
    /// replay-safe data may go here (PROTOCOL.md §2: the `control` header,
    /// `Hello`, `Ping`); the peer's key isn't confirmed yet.
    pub fn early(&self) -> Option<&quinn::Connection> {
        match &self.state {
            DialState::ZeroRtt(conn, _) => Some(conn),
            DialState::Full(_) => None,
        }
    }

    /// Waits for the handshake and checks the peer's key. The flag says
    /// whether the peer accepted the 0-RTT data; if not, it was lost and
    /// must be sent again.
    pub async fn established(self) -> Result<(Connection, bool), ConnectError> {
        let (conn, accepted) = match self.state {
            DialState::Full(connecting) => (connecting.await?, false),
            DialState::ZeroRtt(conn, accepted) => {
                let accepted = accepted.await;
                if let Some(e) = conn.close_reason() {
                    return Err(e.into());
                }
                (conn, accepted)
            }
        };
        Ok((Connection::from_quinn(conn, self.trust.as_ref())?, accepted))
    }
}

/// Delays between reconnection attempts: exponential from `base` up to
/// `max`, each with random jitter (50-100 % of the step) so devices that
/// lost the network together don't retry in step.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Backoff {
            base,
            max: max.max(base),
            attempt: 0,
        }
    }

    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        use rand_core::{OsRng, RngCore};
        let step = self
            .base
            .saturating_mul(1u32 << self.attempt.min(20))
            .min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        let jitter = 0.5 + f64::from(OsRng.next_u32()) / f64::from(u32::MAX) / 2.0;
        step.mul_f64(jitter)
    }

    /// Back to the shortest delay (after a success or a network change).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

impl Default for Backoff {
    /// 100 ms doubling up to 30 s.
    fn default() -> Self {
        Backoff::new(Duration::from_millis(100), Duration::from_secs(30))
    }
}
