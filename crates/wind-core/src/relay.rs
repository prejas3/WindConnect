//! CCS/1 M:relay | PURPOSE:PROTOCOL.md §13 rules the relay client shares with the Worker in relay/
//! SEC:the relay carries QUIC packets already encrypted end to end (§2); it holds no device key.
//! Signing here only proves to the relay which device connects; trust stays with pinned keys.

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};

use crate::identity::{DeviceIdentity, PublicKey};
use crate::transport::Endpoint;

/// The WebSocket path: `/v1/relay?me=<64 hex>&peer=<64 hex>`.
pub const PATH: &str = "/v1/relay";
/// Largest data frame the relay accepts (a QUIC packet; larger closes with 1009).
pub const MAX_FRAME: usize = 2048;
/// Length of the relay's challenge nonce.
pub const NONCE_LEN: usize = 32;

const PAIR_CONTEXT: &[u8] = b"wc-relay-v1 pair";
const AUTH_CONTEXT: &[u8] = b"wc-relay-v1 auth";

/// The pair's rendezvous name: SHA-256("wc-relay-v1 pair" ‖ lower key ‖
/// higher key). The same for both devices.
pub fn pair_name(a: &PublicKey, b: &PublicKey) -> [u8; 32] {
    let (low, high) = if a.as_bytes() <= b.as_bytes() {
        (a, b)
    } else {
        (b, a)
    };
    let mut hash = Sha256::new();
    hash.update(PAIR_CONTEXT);
    hash.update(low.as_bytes());
    hash.update(high.as_bytes());
    hash.finalize().into()
}

/// What a device signs to authenticate: "wc-relay-v1 auth" ‖ nonce ‖ own
/// key ‖ peer key. The context keeps it apart from the TLS signatures the
/// same key makes (§2).
pub fn auth_message(nonce: &[u8; NONCE_LEN], me: &PublicKey, peer: &PublicKey) -> Vec<u8> {
    [AUTH_CONTEXT, nonce, me.as_bytes(), peer.as_bytes()].concat()
}

/// This device's answer to the relay's challenge for a connection to `peer`.
pub fn sign_auth(identity: &DeviceIdentity, nonce: &[u8; NONCE_LEN], peer: &PublicKey) -> [u8; 64] {
    identity.sign(&auth_message(nonce, &identity.public_key(), peer))
}

/// The WebSocket path and query for `me` reaching `peer`.
pub fn request_path(me: &PublicKey, peer: &PublicKey) -> String {
    format!(
        "{PATH}?me={}&peer={}",
        hex(me.as_bytes()),
        hex(peer.as_bytes())
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// The client (WN-60).

/// Longest wait for the relay to answer the WebSocket upgrade and each
/// authentication step.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often an idle link sends `{"t":"ping"}` (§13.3) so carriers and NATs
/// keep the WebSocket open.
pub const KEEP_ALIVE: Duration = Duration::from_secs(25);
/// Packets queued towards the relay; beyond this they are dropped like UDP.
const QUEUE: usize = 256;

/// Why a relay link could not be made.
#[derive(Debug)]
pub enum RelayError {
    /// Not a `wss://` URL (or `ws://` to a loopback or private IP, for local
    /// test relays), or it has a path, query or credentials.
    Url(&'static str),
    /// The WebSocket or its TLS failed.
    Connect(String),
    /// The relay refused us (§13.2) or broke §13.
    Refused(String),
    /// The local UDP stand-in could not be made.
    Io(io::Error),
    /// The relay did not answer in [`CONNECT_TIMEOUT`].
    Timeout,
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Url(why) => write!(f, "relay URL: {why}"),
            RelayError::Connect(e) => write!(f, "relay connection: {e}"),
            RelayError::Refused(why) => write!(f, "relay refused: {why}"),
            RelayError::Io(e) => write!(f, "relay link: {e}"),
            RelayError::Timeout => f.write_str("relay did not answer in time"),
        }
    }
}

impl std::error::Error for RelayError {}

/// Checks a relay base URL: `wss://host[:port]`, or `ws://` only to a
/// loopback or private IP literal (a local `wrangler dev`). The plain form
/// would show the keys in the path to anyone on the network.
pub fn check_url(url: &str) -> Result<(), RelayError> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("wss://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("ws://") {
        (false, rest)
    } else {
        return Err(RelayError::Url("must start with wss://"));
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '@', ' ']) {
        return Err(RelayError::Url(
            "must be a bare origin, e.g. wss://relay.example.workers.dev",
        ));
    }
    if secure {
        return Ok(());
    }
    let host = authority
        .rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(authority, |(host, _)| host);
    let ip: Option<IpAddr> = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok();
    match ip {
        Some(IpAddr::V4(ip)) if ip.is_loopback() || ip.is_private() => Ok(()),
        Some(IpAddr::V6(ip)) if ip.is_loopback() => Ok(()),
        _ => Err(RelayError::Url(
            "ws:// only to a loopback or private IP; use wss://",
        )),
    }
}

/// The value of `"name":"…"` or `"name":true|false` in a relay control frame
/// (the relay's own fixed JSON; no general parser needed).
fn field<'a>(json: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\":");
    let rest = json[json.find(&key)? + key.len()..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return Some(&quoted[..quoted.find('"')?]);
    }
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// A link to one paired device through the relay (§13). It stands for that
/// device at a local UDP address ([`addr`](Self::addr)): QUIC packets our
/// endpoint sends there go to the relay, and packets from the relay are
/// delivered to our endpoint as if from there. QUIC and its pinned-key TLS
/// run end to end on top; the relay sees only ciphertext.
///
/// Dropping the link closes the WebSocket.
pub struct RelayLink {
    addr: SocketAddr,
    presence: watch::Receiver<bool>,
    closed: watch::Receiver<Option<String>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl fmt::Debug for RelayLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayLink")
            .field("addr", &self.addr)
            .field("peer_present", &*self.presence.borrow())
            .finish_non_exhaustive()
    }
}

impl RelayLink {
    /// Connects to the relay at `url` (see [`check_url`]) for `identity` to
    /// reach `peer`, delivering to `endpoint`. Returns once authenticated;
    /// the peer may join later ([`presence`](Self::presence)).
    // CCS/1 FLOW:WebSocket -> challenge -> Ed25519 auth -> ready -> UDP stand-in <-> binary frames
    pub async fn connect(
        url: &str,
        identity: &DeviceIdentity,
        peer: PublicKey,
        endpoint: Endpoint,
    ) -> Result<Self, RelayError> {
        check_url(url)?;
        let full = format!(
            "{}{}",
            url.trim_end_matches('/'),
            request_path(&identity.public_key(), &peer)
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(64 * 1024))
            .max_frame_size(Some(64 * 1024));
        let connector = Connector::Rustls(Arc::new(tls_config()));
        let (mut ws, _) = timeout(
            CONNECT_TIMEOUT,
            connect_async_tls_with_config(full, Some(config), false, Some(connector)),
        )
        .await
        .map_err(|_| RelayError::Timeout)?
        .map_err(|e| RelayError::Connect(e.to_string()))?;

        let challenge = control(&mut ws).await?;
        let nonce = field(&challenge, "nonce")
            .filter(|n| n.len() == 2 * NONCE_LEN && n.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(RelayError::Refused("no challenge".into()))?;
        let mut bytes = [0; NONCE_LEN];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&nonce[i * 2..i * 2 + 2], 16).unwrap_or_default();
        }
        let sig = sign_auth(identity, &bytes, &peer);
        ws.send(Message::text(format!(
            r#"{{"t":"auth","sig":"{}"}}"#,
            hex(&sig)
        )))
        .await
        .map_err(|e| RelayError::Connect(e.to_string()))?;
        let ready = control(&mut ws).await?;
        if field(&ready, "t") != Some("ready") {
            return Err(RelayError::Refused("no ready".into()));
        }
        let present = field(&ready, "peer") == Some("true");

        let local = endpoint.local_addr().map_err(RelayError::Io)?;
        let udp = Arc::new(
            UdpSocket::bind(SocketAddr::new(deliver_ip(local.ip()), 0))
                .await
                .map_err(RelayError::Io)?,
        );
        let addr = udp.local_addr().map_err(RelayError::Io)?;
        let (presence_tx, presence) = watch::channel(present);
        let (closed_tx, closed) = watch::channel(None);
        let closed_tx = Arc::new(closed_tx);
        let (mut sink, mut stream) = ws.split();
        let (queue, mut outgoing) = mpsc::channel::<Message>(QUEUE);

        let writer = {
            let closed_tx = closed_tx.clone();
            tokio::spawn(async move {
                while let Some(message) = outgoing.recv().await {
                    if let Err(e) = sink.send(message).await {
                        closed_tx.send_replace(Some(e.to_string()));
                        break;
                    }
                }
                let _ = sink.close().await;
            })
        };
        // CCS/1 SEC:only our own endpoint's packets go up; anything else on loopback is dropped.
        let up = {
            let (udp, queue, endpoint) = (udp.clone(), queue.clone(), endpoint.clone());
            tokio::spawn(async move {
                let mut buf = vec![0; 65536];
                while let Ok((n, from)) = udp.recv_from(&mut buf).await {
                    let ours = endpoint
                        .local_addr()
                        .is_ok_and(|own| deliver_to(own) == from);
                    if ours && n <= MAX_FRAME {
                        let _ = queue.try_send(Message::binary(buf[..n].to_vec()));
                    }
                }
            })
        };
        let down = {
            let (closed_tx, endpoint) = (closed_tx.clone(), endpoint.clone());
            tokio::spawn(async move {
                let reason = loop {
                    match stream.next().await {
                        Some(Ok(Message::Binary(packet))) => {
                            // CCS/1 RULE:delivered to the endpoint's current address (WN-84 rebinds).
                            if let Ok(own) = endpoint.local_addr() {
                                let _ = udp.send_to(&packet, deliver_to(own)).await;
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            if field(&text, "t") == Some("peer") {
                                presence_tx.send_replace(field(&text, "present") == Some("true"));
                            }
                        }
                        Some(Ok(Message::Close(frame))) => {
                            break frame.map_or("closed by the relay".into(), |f| {
                                format!("closed by the relay: {} {}", u16::from(f.code), f.reason)
                            })
                        }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => break e.to_string(),
                        None => break "relay connection ended".into(),
                    }
                };
                presence_tx.send_replace(false);
                closed_tx.send_replace(Some(reason));
            })
        };
        let keep_alive = tokio::spawn(async move {
            loop {
                tokio::time::sleep(KEEP_ALIVE).await;
                if queue.send(Message::text(r#"{"t":"ping"}"#)).await.is_err() {
                    break;
                }
            }
        });
        Ok(RelayLink {
            addr,
            presence,
            closed,
            tasks: vec![writer, up, down, keep_alive],
        })
    }

    /// Where our endpoint dials (or answers) the peer through the relay.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Whether the peer is connected to the relay now; updated by the relay.
    pub fn presence(&self) -> watch::Receiver<bool> {
        self.presence.clone()
    }

    /// Why the link ended, once it has; it never comes back (make a new one).
    pub async fn closed(&self) -> String {
        let mut closed = self.closed.clone();
        let reason = match closed.wait_for(Option::is_some).await {
            Ok(reason) => reason.clone().unwrap_or_default(),
            Err(_) => "relay link dropped".into(),
        };
        reason
    }
}

impl Drop for RelayLink {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// The next control (text) frame; a close frame is the relay's refusal.
async fn control(
    ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> Result<String, RelayError> {
    loop {
        let message = timeout(CONNECT_TIMEOUT, ws.next())
            .await
            .map_err(|_| RelayError::Timeout)?
            .ok_or(RelayError::Refused("closed".into()))?
            .map_err(|e| RelayError::Connect(e.to_string()))?;
        match message {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Close(frame) => {
                return Err(RelayError::Refused(frame.map_or("closed".into(), |f| {
                    format!("{} {}", u16::from(f.code), f.reason)
                })))
            }
            Message::Ping(_) | Message::Pong(_) => {}
            _ => return Err(RelayError::Refused("data before ready".into())),
        }
    }
}

/// Where the stand-in reaches our endpoint: its own address, loopback when
/// it listens on all interfaces.
fn deliver_to(endpoint: SocketAddr) -> SocketAddr {
    SocketAddr::new(deliver_ip(endpoint.ip()), endpoint.port())
}

fn deliver_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    }
}

/// TLS 1.3 for `wss://`: rustls with ring (as the rest of the core) and the
/// Mozilla roots from webpki-roots, so it works the same on Android.
fn tls_config() -> rustls::ClientConfig {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};

    fn key(seed: u8) -> (DeviceIdentity, PublicKey) {
        let identity = DeviceIdentity::from_key(SigningKey::from_bytes(&[seed; 32])).unwrap();
        let public = identity.public_key();
        (identity, public)
    }

    #[test]
    fn relay_urls_are_wss_or_local_ws_origins() {
        for ok in [
            "wss://windconnect-relay.example.workers.dev",
            "wss://relay.example.com:8443/",
            "ws://127.0.0.1:8787",
            "ws://10.0.2.2:8787",
            "ws://192.168.1.20:8787",
            "ws://[::1]:8787",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "https://relay.example.com",
            "ws://relay.example.com",
            "ws://8.8.8.8:8787",
            "wss://",
            "wss://relay.example.com/v1/relay",
            "wss://relay.example.com?x=1",
            "wss://user@relay.example.com",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn control_fields() {
        let ready = r#"{"t":"ready","peer":true}"#;
        assert_eq!(field(ready, "t"), Some("ready"));
        assert_eq!(field(ready, "peer"), Some("true"));
        assert_eq!(
            field(r#"{"t":"peer","present":false}"#, "present"),
            Some("false")
        );
        assert_eq!(
            field(r#"{"t":"challenge","nonce":"ab12"}"#, "nonce"),
            Some("ab12")
        );
        assert_eq!(field(ready, "nonce"), None);
    }

    #[test]
    fn stand_in_delivers_to_loopback_for_wildcard_endpoints() {
        assert_eq!(
            deliver_to("0.0.0.0:47820".parse().unwrap()),
            "127.0.0.1:47820".parse().unwrap()
        );
        assert_eq!(
            deliver_to("[::]:5".parse().unwrap()),
            "[::1]:5".parse().unwrap()
        );
        assert_eq!(
            deliver_to("192.168.1.2:7".parse().unwrap()),
            "192.168.1.2:7".parse().unwrap()
        );
    }

    #[test]
    fn pair_name_is_symmetric_and_distinct() {
        let (_, a) = key(7);
        let (_, b) = key(8);
        let (_, c) = key(9);
        assert_eq!(pair_name(&a, &b), pair_name(&b, &a));
        assert_ne!(pair_name(&a, &b), pair_name(&a, &c));
    }

    #[test]
    fn auth_signature_verifies_only_for_its_nonce_and_peer() {
        let (alice, a) = key(7);
        let (_, b) = key(8);
        let nonce = [1; NONCE_LEN];
        let sig = Signature::from_bytes(&sign_auth(&alice, &nonce, &b));
        let verifying = VerifyingKey::from_bytes(a.as_bytes()).unwrap();
        assert!(verifying
            .verify(&auth_message(&nonce, &a, &b), &sig)
            .is_ok());
        assert!(verifying
            .verify(&auth_message(&[2; NONCE_LEN], &a, &b), &sig)
            .is_err());
        assert!(verifying
            .verify(&auth_message(&nonce, &a, &a), &sig)
            .is_err());
    }

    /// Fixed vectors shared with relay/test/vectors.test.ts: both sides
    /// derive the same rendezvous and accept the same signature.
    #[test]
    fn vectors_match_the_worker() {
        let (alice, a) = key(7);
        let (_, b) = key(8);
        let nonce = [1; NONCE_LEN];
        assert_eq!(
            hex(a.as_bytes()),
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
        );
        assert_eq!(
            hex(b.as_bytes()),
            "1398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca"
        );
        assert_eq!(
            hex(&pair_name(&a, &b)),
            "890e36d8d7bef70886770d7b404e97b989fd1288e71d1147fef5384822b9a79f"
        );
        assert_eq!(hex(&sign_auth(&alice, &nonce, &b)), "242a37e26274de3a5a81acf4594ced30baa0b8d1a27baee55a1b27981c41235c83965a64a828f8e0caddb58f8bfc2c3b41abb8db245dc3ff365323ff897e4a04");
    }
}
