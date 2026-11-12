//! A protocol session over a pinned QUIC connection (PROTOCOL.md §3, §6,
//! §7; WC-11): the `Hello` / `HelloAck` exchange, the `control` channel,
//! one stream per channel with send priorities, and the [`Router`] that
//! hands each channel's messages to its feature module.
//!
//! Sending: [`Session::send`] keeps at most one open uni stream per channel
//! (§3.1, rule 1) and [`Session::open_files`] opens the bidi streams of a
//! transfer (one or more each, §9.1; see [`crate::files`]).
//! Every stream gets the §3 priority, so `control` and `input` are sent
//! before bulk channels, and `files` streams share what is left.
//!
//! Receiving: uni streams are taken in stream-number order; a stream's
//! messages reach its module only after every earlier stream's header is in
//! and every earlier stream of the same channel is done (§3.1, rule 3).
//! Each channel is read by its own task, so a slow module or a large
//! transfer holds back only its own channel.
//!
//! §3.1 rule 5: no stream is refused for how many of a channel are open.
//! Each channel reads its streams ahead of its module, into a buffer of
//! [`READ_AHEAD_BYTES`] (rule 5.6, WN-81): a finished stream read into it
//! gives its place in QUIC's shared stream limit
//! ([`MAX_UNI_STREAMS`](crate::transport::MAX_UNI_STREAMS)) back at once, so
//! a slow module holds back no other channel while its backlog fits. Past
//! the buffer, and for a sender that opens streams without finishing them
//! (against rule 1), streams keep their place and the sender waits for
//! `MAX_STREAMS`; nothing is lost.
//!
//! Reconnecting: [`Session::dial`] sends `Hello` as 0-RTT data when a TLS
//! session with the peer is remembered; keep-alive, path migration and
//! retry backoff are the transport's (WC-12).
//!
//! Changing what this side accepts mid-connection (§6, rule 5, WN-74):
//! [`Session::set_accepted`] withdraws or re-enables a routed channel and
//! sends `Capabilities` with the full new list. From then on a withdrawn
//! channel takes nothing: its streams get `STOP_SENDING` (`CANCELLED`), and
//! new ones `CAPABILITY_MISSING` (§3, rule 3); re-enabled, it takes the
//! peer's next, new stream. Withdrawing `input` releases pressed buttons and
//! ends its session (§8.2). Withdrawing `files` stops the streams already
//! handed to its module: their next read stops them with `CANCELLED` and
//! fails with [`RecvError::Withdrawn`] (WN-86).

mod framed;
mod router;

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

pub use framed::{write_raw, FramedRecv, RecvError};

use crate::input::{InputAnnouncement, InputEvent, InputReceiver, InputSender};
use crate::protocol::InputMessage;
pub use router::{
    capability_name, is_uni, ActionStream, BidiStream, ChannelMessage, FileStream, FindStream,
    Router,
};

use crate::identity::PublicKey;
use crate::protocol::{
    decode_stream_header, encode_frame, stream_header, Capability, Channel, ControlMessage,
    DeviceKind, ErrorCode, ErrorMessage, FrameError, Goodbye, Hello, Message, RawMessage, Version,
    PROTOCOL_MAJOR, PROTOCOL_MINOR,
};
use crate::transport::{
    is_untrusted_close, Backoff, Connection, Endpoint, CONNECT_ATTEMPT_TIMEOUT,
};

/// Wait for `Hello` / `HelloAck` (§7.1, rule 4).
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// Wait for a uni stream's header once a later stream is in (§3.1, rule 4).
pub const HEADER_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for a `Pong`.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait for the peer to take `Goodbye` before closing.
pub const GOODBYE_FLUSH: Duration = Duration::from_secs(1);

/// The QUIC send priority of a channel: §3's table turned around, since
/// quinn sends higher numbers first (control 0 → 7, files 7 → 0).
pub fn send_priority(channel: Channel) -> i32 {
    7 - match channel {
        Channel::Control => 0,
        Channel::Input => 1,
        Channel::Clipboard => 2,
        Channel::Notify => 3,
        Channel::Media | Channel::MediaControl => 4,
        Channel::Actions | Channel::Find => 5,
        Channel::Battery => 6,
        Channel::Files => 7,
    }
}

pub(crate) fn varint_code(code: ErrorCode) -> quinn::VarInt {
    quinn::VarInt::from_u64(code.code()).unwrap_or(quinn::VarInt::from_u32(0x0a))
}

/// This device as it presents itself in `Hello` / `HelloAck`.
#[derive(Debug, Clone)]
pub struct LocalInfo {
    /// 1–64 UTF-8 bytes; for display only.
    pub name: String,
    pub kind: DeviceKind,
    /// Application version, 1–32 UTF-8 bytes.
    pub app: Option<String>,
}

/// What the peer said about itself, and its pinned key.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub key: PublicKey,
    pub hello: Hello,
}

/// Something the peer did on `control` that the application may care about.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// The peer changed what it accepts (§6, rule 5).
    Capabilities(Vec<Capability>),
    /// A non-fatal error from the peer (§7.6).
    Error(ErrorMessage),
    /// The peer is leaving (§7.7); the connection closes next.
    Goodbye(Goodbye),
    /// The connection is closed.
    Closed(String),
}

/// Why a session couldn't start or an operation failed.
#[derive(Debug)]
pub enum SessionError {
    /// The handshake broke the protocol; the connection was closed with
    /// this code.
    Protocol(ErrorCode, &'static str),
    Connection(quinn::ConnectionError),
    Write(quinn::WriteError),
    Recv(RecvError),
    Frame(FrameError),
    /// The peer doesn't accept this channel (§6).
    CapabilityMissing(Channel),
    /// Not a channel this call sends on.
    WrongChannel(Channel),
    /// The peer stopped our stream (`STOP_SENDING`) with this code.
    Stopped(u64),
    /// Dialing failed (see [`crate::transport::ConnectError`]).
    Connect(crate::transport::ConnectError),
    Timeout,
    /// The connection was in notice mode (the peer's key is revoked, WC-28,
    /// §12.10): no session started; `Goodbye` `UNPAIRED` was sent and the
    /// connection closed. `delivered` when the peer acknowledged it: then
    /// the notice is settled (`PairedDevicesFile::settle`), otherwise it is
    /// owed again on the next connection.
    Unpaired {
        peer: PublicKey,
        delivered: bool,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Protocol(code, why) => write!(f, "{why} ({code:?})"),
            SessionError::Connection(e) => write!(f, "connection: {e}"),
            SessionError::Write(e) => write!(f, "write: {e}"),
            SessionError::Recv(e) => write!(f, "read: {e}"),
            SessionError::Frame(e) => write!(f, "frame: {e:?}"),
            SessionError::CapabilityMissing(ch) => write!(f, "peer doesn't accept {ch:?}"),
            SessionError::WrongChannel(ch) => write!(f, "{ch:?} is not sent this way"),
            SessionError::Stopped(code) => write!(f, "peer stopped the stream ({code})"),
            SessionError::Connect(e) => write!(f, "{e}"),
            SessionError::Timeout => f.write_str("timed out"),
            SessionError::Unpaired { delivered, .. } => write!(
                f,
                "unpair notice {}",
                if *delivered {
                    "delivered"
                } else {
                    "not delivered"
                }
            ),
        }
    }
}

impl SessionError {
    /// Failed because a key isn't pinned, on our side or the peer's.
    pub fn is_untrusted_peer(&self) -> bool {
        match self {
            SessionError::Connect(e) => e.is_untrusted_peer(),
            SessionError::Connection(e) => is_untrusted_close(e),
            _ => false,
        }
    }
}

impl std::error::Error for SessionError {}

impl From<quinn::ConnectionError> for SessionError {
    fn from(e: quinn::ConnectionError) -> Self {
        SessionError::Connection(e)
    }
}

impl From<quinn::WriteError> for SessionError {
    fn from(e: quinn::WriteError) -> Self {
        match e {
            quinn::WriteError::Stopped(code) => SessionError::Stopped(code.into_inner()),
            quinn::WriteError::ConnectionLost(c) => SessionError::Connection(c),
            other => SessionError::Write(other),
        }
    }
}

impl From<RecvError> for SessionError {
    fn from(e: RecvError) -> Self {
        match e {
            RecvError::Read(quinn::ReadError::ConnectionLost(c)) => SessionError::Connection(c),
            other => SessionError::Recv(other),
        }
    }
}

struct Inner {
    conn: Connection,
    peer: PeerInfo,
    peer_caps: RwLock<Vec<Capability>>,
    control: tokio::sync::Mutex<quinn::SendStream>,
    /// The open uni stream of each channel (§3.1, rule 1).
    uni: HashMap<u8, tokio::sync::Mutex<Option<quinn::SendStream>>>,
    pings: Mutex<HashMap<u64, oneshot::Sender<()>>>,
    next_nonce: AtomicU64,
    goodbye_received: AtomicBool,
    events: mpsc::UnboundedSender<SessionEvent>,
    /// The peer's announcements so far (`Hello` / `HelloAck` is 0, each
    /// `Capabilities` one more) and, while it accepts input, the input
    /// announcements (§8.2).
    peer_announcements: AtomicU64,
    peer_input_announcement: Mutex<Option<InputAnnouncement>>,
    input: std::sync::OnceLock<InputSender>,
    /// What this side accepts: every routed channel, enabled or withdrawn
    /// (§6, rule 5). Changed only with `control` locked, so each
    /// `Capabilities` carries the list as it was when sent.
    accepts: Mutex<Vec<Accept>>,
    /// Our announcements so far: `Hello` / `HelloAck` is 0, each
    /// `Capabilities` one more (§8.2).
    local_announcements: AtomicU64,
    /// The gate of each uni channel we route (and `input`, `files`).
    gates: HashMap<u8, watch::Sender<Gate>>,
    /// The input actor, when we route input.
    input_cmds: Option<mpsc::Sender<InputCmd>>,
}

/// A channel this side routes, and whether it accepts it now.
#[derive(Debug, Clone, Copy)]
struct Accept {
    channel: Channel,
    version: u64,
    enabled: bool,
}

/// What the readers of one channel go by: whether it is accepted, and how
/// many times it has been withdrawn. A stream is tagged with the count when
/// it is taken; once the count moves on, the stream is over, even if the
/// channel was enabled again meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gate {
    enabled: bool,
    withdrawals: u64,
}

fn capabilities_of(accepts: &[Accept]) -> Vec<Capability> {
    let mut caps: Vec<Capability> = accepts
        .iter()
        .filter(|a| a.enabled)
        // A routed version N accepts every version up to N (§6, rule 2:
        // clipboard/2 extends clipboard/1), so each is announced.
        .flat_map(|a| (1..=a.version).map(move |v| (a.channel, v)))
        .filter_map(|(channel, v)| Capability::new(capability_name(channel)?, v))
        .collect();
    caps.sort_by_key(|c| c.to_string());
    caps
}

impl Inner {
    /// The gate of `channel` now: open, and its withdrawal count.
    fn gate(&self, channel: Channel) -> Option<Gate> {
        self.gates.get(&channel.byte()).map(|g| *g.borrow())
    }
}

/// A running session. Cheap to clone; every clone is the same session.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("peer", &self.inner.peer.hello.name)
            .finish_non_exhaustive()
    }
}

fn close(conn: &Connection, code: ErrorCode, why: &'static str) -> SessionError {
    conn.close(code, why.as_bytes());
    SessionError::Protocol(code, why)
}

/// Opens `control` and sends the header and `Hello` (§7.1).
async fn open_control(
    conn: &quinn::Connection,
    local: &LocalInfo,
    router: &Router,
) -> Result<(quinn::SendStream, quinn::RecvStream), SessionError> {
    let (mut send, recv) = conn.open_bi().await?;
    let _ = send.set_priority(send_priority(Channel::Control));
    let frame = encode_frame(&Message::Control(ControlMessage::Hello(hello_for(
        local, router,
    ))))
    .map_err(SessionError::Frame)?;
    let mut out = stream_header(Channel::Control).to_vec();
    out.extend_from_slice(&frame);
    send.write_all(&out).await?;
    Ok((send, recv))
}

fn hello_for(local: &LocalInfo, router: &Router) -> Hello {
    Hello {
        version: Version {
            major: u64::from(PROTOCOL_MAJOR),
            minor: PROTOCOL_MINOR,
        },
        name: local.name.clone(),
        kind: local.kind,
        caps: router.capabilities(),
        app: local.app.clone(),
    }
}

fn check_version(conn: &Connection, hello: &Hello) -> Result<(), SessionError> {
    if hello.version.major != u64::from(PROTOCOL_MAJOR) {
        return Err(close(
            conn,
            ErrorCode::ProtocolViolation,
            "Hello major version differs from ALPN",
        ));
    }
    Ok(())
}

/// The first frame on `control` within the hello deadline.
async fn first_message(
    conn: &Connection,
    recv: &mut FramedRecv,
    deadline: Instant,
) -> Result<ControlMessage, SessionError> {
    let msg = tokio::time::timeout_at(deadline.into(), recv.next_message())
        .await
        .map_err(|_| close(conn, ErrorCode::HelloTimeout, "no Hello / HelloAck in time"))?;
    match msg {
        Ok(Some(Message::Control(m))) => Ok(m),
        Ok(Some(_)) | Ok(None) => Err(close(
            conn,
            ErrorCode::ProtocolViolation,
            "control stream ended before Hello",
        )),
        Err(RecvError::Frame(e)) => Err(close(conn, e.code(), "bad first control frame")),
        Err(e) => Err(e.into()),
    }
}

impl Session {
    /// Starts a session as the client (the side that dialed): opens
    /// `control`, sends `Hello`, waits for `HelloAck` (§7.1).
    pub async fn connect(
        conn: Connection,
        local: LocalInfo,
        router: Router,
    ) -> Result<(Session, mpsc::UnboundedReceiver<SessionEvent>), SessionError> {
        let deadline = Instant::now() + HELLO_TIMEOUT;
        let router = if conn.is_notice() {
            Router::new()
        } else {
            router
        };
        let control = open_control(conn.quinn(), &local, &router).await?;
        Self::finish_client(conn, control, router, deadline).await
    }

    /// Dials `peer` at `addr` and starts a session as the client. With a
    /// remembered TLS session (a reconnect) `Hello` goes out as 0-RTT data,
    /// in the first flight (PROTOCOL.md §2, WC-12); if the peer refuses the
    /// 0-RTT data, `Hello` is sent again after the handshake.
    pub async fn dial(
        endpoint: &Endpoint,
        addr: SocketAddr,
        peer: PublicKey,
        local: LocalInfo,
        router: Router,
    ) -> Result<(Session, mpsc::UnboundedReceiver<SessionEvent>), SessionError> {
        let deadline = Instant::now() + HELLO_TIMEOUT;
        let dialing = endpoint.dial(addr, peer).map_err(SessionError::Connect)?;
        let early = match dialing.early() {
            Some(conn) => open_control(conn, &local, &router).await.ok(),
            None => None,
        };
        let (conn, accepted) = dialing.established().await.map_err(SessionError::Connect)?;
        // A notice dial has no 0-RTT (`Endpoint::dial`), so `early` is None.
        let router = if conn.is_notice() {
            Router::new()
        } else {
            router
        };
        let control = match (early, accepted) {
            (Some(control), true) => control,
            (rejected, _) => {
                // The refused 0-RTT streams are gone with their data. Their
                // handles are released before the new control stream is
                // opened; quinn already ignores handles of refused 0-RTT
                // streams, so this is only tidy cleanup.
                drop(rejected);
                open_control(conn.quinn(), &local, &router).await?
            }
        };
        Self::finish_client(conn, control, router, deadline).await
    }

    /// Dials until a session is up, retrying with `backoff` (WC-12): after
    /// a failed attempt it waits for the next delay, or less if
    /// `network_changed` is notified (then it resets the backoff and tries
    /// at once). Each attempt gets a fresh router from `make_router`. Gives
    /// up only when the peer isn't (or is no longer) paired on either side,
    /// which only the session can tell: TLS 1.3 lets a refused client finish
    /// its handshake, and `HelloAck` is the peer's acceptance.
    #[allow(clippy::too_many_arguments)]
    pub async fn reconnect(
        endpoint: &Endpoint,
        addr: SocketAddr,
        peer: PublicKey,
        local: &LocalInfo,
        mut make_router: impl FnMut() -> Router,
        backoff: &mut Backoff,
        network_changed: &tokio::sync::Notify,
    ) -> Result<(Session, mpsc::UnboundedReceiver<SessionEvent>), SessionError> {
        loop {
            let attempt = tokio::time::timeout(
                CONNECT_ATTEMPT_TIMEOUT,
                Self::dial(endpoint, addr, peer, local.clone(), make_router()),
            );
            // A network change during an attempt drops it (its connection
            // closes with it) and starts a fresh one at once.
            let outcome = tokio::select! {
                r = attempt => Some(r),
                _ = network_changed.notified() => None,
            };
            match outcome {
                None => {
                    backoff.reset();
                    continue;
                }
                Some(Ok(Ok(session))) => {
                    backoff.reset();
                    return Ok(session);
                }
                Some(Ok(Err(e))) if e.is_untrusted_peer() => return Err(e),
                Some(Ok(Err(_))) | Some(Err(_)) => {}
            }
            let delay = backoff.next_delay();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = network_changed.notified() => backoff.reset(),
            }
        }
    }

    async fn finish_client(
        conn: Connection,
        (send, recv): (quinn::SendStream, quinn::RecvStream),
        router: Router,
        deadline: Instant,
    ) -> Result<(Session, mpsc::UnboundedReceiver<SessionEvent>), SessionError> {
        let mut recv = FramedRecv::new(recv, Channel::Control);
        let peer_hello = match first_message(&conn, &mut recv, deadline).await? {
            ControlMessage::HelloAck(h) => h,
            _ => {
                return Err(close(
                    &conn,
                    ErrorCode::ProtocolViolation,
                    "first frame from the server is not HelloAck",
                ))
            }
        };
        check_version(&conn, &peer_hello)?;
        let mut send = send;
        if conn.is_notice() {
            return Err(notice(&conn, &mut send).await);
        }
        Ok(Self::start(conn, peer_hello, send, recv, router))
    }

    /// Starts a session as the server (the side that accepted): takes the
    /// `control` stream, reads `Hello`, answers `HelloAck` (§7.1).
    pub async fn accept(
        conn: Connection,
        local: LocalInfo,
        router: Router,
    ) -> Result<(Session, mpsc::UnboundedReceiver<SessionEvent>), SessionError> {
        let deadline = Instant::now() + HELLO_TIMEOUT;
        let accepted = tokio::time::timeout_at(deadline.into(), conn.quinn().accept_bi())
            .await
            .map_err(|_| close(&conn, ErrorCode::HelloTimeout, "no control stream in time"))?;
        let (mut send, mut recv) = accepted?;
        let mut header = [0u8; 2];
        match tokio::time::timeout_at(deadline.into(), recv.read_exact(&mut header)).await {
            Err(_) => return Err(close(&conn, ErrorCode::HelloTimeout, "no Hello in time")),
            Ok(Err(_)) => {
                return Err(close(
                    &conn,
                    ErrorCode::ProtocolViolation,
                    "control stream ended before its header",
                ))
            }
            Ok(Ok(())) => {}
        }
        match decode_stream_header(header) {
            Ok(Channel::Control) => {}
            Ok(_) => {
                return Err(close(
                    &conn,
                    ErrorCode::ProtocolViolation,
                    "first stream is not control",
                ))
            }
            Err(e) => return Err(close(&conn, e.code(), "bad control stream header")),
        }
        let mut recv = FramedRecv::new(recv, Channel::Control);
        let peer_hello = match first_message(&conn, &mut recv, deadline).await? {
            ControlMessage::Hello(h) => h,
            _ => {
                return Err(close(
                    &conn,
                    ErrorCode::ProtocolViolation,
                    "first frame from the client is not Hello",
                ))
            }
        };
        check_version(&conn, &peer_hello)?;

        // CCS/1 SEC:notice mode (revoked key, §12.10): HelloAck with our name and no capabilities,
        // no router, no stream loops; only Goodbye UNPAIRED, then close.
        let router = if conn.is_notice() {
            Router::new()
        } else {
            router
        };
        let _ = send.set_priority(send_priority(Channel::Control));
        let ack = encode_frame(&Message::Control(ControlMessage::HelloAck(hello_for(
            &local, &router,
        ))))
        .map_err(SessionError::Frame)?;
        send.write_all(&ack).await?;
        if conn.is_notice() {
            return Err(notice(&conn, &mut send).await);
        }
        Ok(Self::start(conn, peer_hello, send, recv, router))
    }

    fn start(
        conn: Connection,
        peer_hello: Hello,
        control_send: quinn::SendStream,
        control_recv: FramedRecv,
        router: Router,
    ) -> (Session, mpsc::UnboundedReceiver<SessionEvent>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let uni = [
            Channel::Clipboard,
            Channel::Notify,
            Channel::Media,
            Channel::Battery,
            Channel::MediaControl,
        ]
        .into_iter()
        .map(|ch| (ch.byte(), tokio::sync::Mutex::new(None)))
        .collect();
        let input_announced = has_input(&peer_hello.caps).then_some(InputAnnouncement {
            last: 0,
            enabled: 0,
        });
        let open = Gate {
            enabled: true,
            withdrawals: 0,
        };
        let mut accepts: Vec<Accept> = router
            .uni
            .values()
            .map(|(ch, v, _)| (*ch, *v))
            .chain(router.files.as_ref().map(|(v, _)| (Channel::Files, *v)))
            .chain(router.actions.as_ref().map(|(v, _)| (Channel::Actions, *v)))
            .chain(router.find.as_ref().map(|_| (Channel::Find, 1)))
            .chain(router.input.as_ref().map(|_| (Channel::Input, 1)))
            .map(|(channel, version)| Accept {
                channel,
                version,
                enabled: !router.disabled.contains(&channel.byte()),
            })
            .collect();
        accepts.sort_by_key(|a| a.channel.byte());
        let gates = accepts
            .iter()
            .filter(|a| a.channel != Channel::Control)
            .map(|a| {
                let gate = Gate {
                    enabled: a.enabled,
                    ..open
                };
                (a.channel.byte(), watch::Sender::new(gate))
            })
            .collect();
        let input = router.input.map(|out| {
            let (tx, rx) = mpsc::channel(INPUT_COMMANDS);
            tokio::spawn(input_actor(rx, out));
            tx
        });
        let inner = Arc::new(Inner {
            peer: PeerInfo {
                key: conn.peer_key(),
                hello: peer_hello.clone(),
            },
            peer_caps: RwLock::new(peer_hello.caps),
            conn,
            control: tokio::sync::Mutex::new(control_send),
            uni,
            pings: Mutex::new(HashMap::new()),
            next_nonce: AtomicU64::new(1),
            goodbye_received: AtomicBool::new(false),
            events,
            peer_announcements: AtomicU64::new(0),
            peer_input_announcement: Mutex::new(input_announced),
            input: std::sync::OnceLock::new(),
            accepts: Mutex::new(accepts),
            local_announcements: AtomicU64::new(0),
            gates,
            input_cmds: input.clone(),
        });
        if let Some(cmds) = &input {
            tokio::spawn(datagram_loop(inner.conn.quinn().clone(), cmds.clone()));
        }
        tokio::spawn(control_loop(inner.clone(), control_recv));
        tokio::spawn(uni_loop(inner.clone(), router.uni, input));
        tokio::spawn(bidi_loop(
            inner.clone(),
            router.files.map(|(_, tx)| tx),
            router.actions.map(|(_, tx)| tx),
            router.find,
        ));
        (Session { inner }, events_rx)
    }

    pub fn peer(&self) -> &PeerInfo {
        &self.inner.peer
    }

    /// Sends input to the peer (§8): pointer and scroll as datagrams (or on
    /// the stream when datagrams are off), buttons on the `input` stream.
    /// Fails with `CapabilityMissing` while the peer doesn't accept input.
    pub fn input(&self) -> Result<InputSender, SessionError> {
        if !self.peer_accepts(Channel::Input) {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        Ok(self
            .inner
            .input
            .get_or_init(|| {
                let inner = Arc::downgrade(&self.inner);
                InputSender::new(
                    self.inner.conn.quinn().clone(),
                    Arc::new(move || {
                        let inner = inner.upgrade()?;
                        let a = *inner
                            .peer_input_announcement
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        a
                    }),
                )
            })
            .clone())
    }

    /// What the peer accepts now (from `HelloAck` / `Hello`, then
    /// `Capabilities`).
    pub fn peer_capabilities(&self) -> Vec<Capability> {
        self.inner
            .peer_caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn connection(&self) -> &Connection {
        &self.inner.conn
    }

    /// What this side accepts now: what `Hello` / `HelloAck` announced,
    /// changed by [`set_accepted`](Self::set_accepted).
    pub fn accepted_capabilities(&self) -> Vec<Capability> {
        capabilities_of(&self.inner.accepts.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Withdraws (`accept = false`) or re-enables a channel this side routes
    /// (§6, rule 5), and tells the peer with `Capabilities` carrying the full
    /// new list. `Ok(false)` when it was already so; nothing is sent then.
    ///
    /// Withdrawn, the channel takes nothing from the moment `Capabilities`
    /// goes out: its streams are stopped with `CANCELLED`, messages still
    /// arriving on them are dropped, and new streams get
    /// `CAPABILITY_MISSING`. For `input`, pressed buttons are released and
    /// the session ends (§8.2). Re-enabled, the channel takes the peer's
    /// next stream, which the peer opens anew.
    ///
    /// Only a channel routed when the session started can be re-enabled
    /// (its module is the router's). `control` is not switched: an
    /// unrouted channel or `control` is `WrongChannel`. For `files`, streams
    /// already taken fail their next read with [`RecvError::Withdrawn`]
    /// (stopped with `CANCELLED`); the module ends its transfers.
    pub async fn set_accepted(&self, channel: Channel, accept: bool) -> Result<bool, SessionError> {
        let gate = self
            .inner
            .gates
            .get(&channel.byte())
            .ok_or(SessionError::WrongChannel(channel))?;
        let mut control = self.inner.control.lock().await;
        let caps = {
            let mut accepts = self.inner.accepts.lock().unwrap_or_else(|e| e.into_inner());
            let Some(entry) = accepts.iter_mut().find(|a| a.channel == channel) else {
                return Err(SessionError::WrongChannel(channel));
            };
            if entry.enabled == accept {
                return Ok(false);
            }
            entry.enabled = accept;
            capabilities_of(&accepts)
        };
        let number = self
            .inner
            .local_announcements
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        if !accept {
            // Closed before `Capabilities` leaves: from the moment it is
            // sent, nothing more is taken.
            if channel == Channel::Input {
                if let Some(cmds) = &self.inner.input_cmds {
                    let _ = cmds.send(InputCmd::Withdraw).await;
                }
            }
            gate.send_modify(|g| {
                g.enabled = false;
                g.withdrawals += 1;
            });
        }
        let frame = encode_frame(&Message::Control(ControlMessage::Capabilities { caps }))
            .map_err(SessionError::Frame);
        let sent = match frame {
            Ok(frame) => control.write_all(&frame).await.map_err(SessionError::from),
            Err(e) => Err(e),
        };
        if accept {
            // The peer opens new streams only once it has this
            // announcement, so opening the gate after writing it is in time.
            if channel == Channel::Input {
                if let Some(cmds) = &self.inner.input_cmds {
                    let _ = cmds.send(InputCmd::Enable(number)).await;
                }
            }
            gate.send_modify(|g| g.enabled = true);
        }
        sent.map(|()| true)
    }

    fn peer_accepts(&self, channel: Channel) -> bool {
        let Some(name) = capability_name(channel) else {
            return true;
        };
        // This implementation sends version 1 of every channel (§6, rule 2).
        self.inner
            .peer_caps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|c| c.name() == name && c.version() == 1)
    }

    /// Sends one message on a uni channel, on its open stream (opening one
    /// if needed). Messages of one channel arrive in the order sent.
    pub async fn send(&self, channel: Channel, msg: &RawMessage) -> Result<(), SessionError> {
        let slot = self
            .inner
            .uni
            .get(&channel.byte())
            .ok_or(SessionError::WrongChannel(channel))?;
        if !self.peer_accepts(channel) {
            return Err(SessionError::CapabilityMissing(channel));
        }
        let frame = crate::protocol::encode_raw_frame(channel, msg).map_err(SessionError::Frame)?;
        let mut slot = slot.lock().await;
        let result = match slot.as_mut() {
            Some(stream) => stream.write_all(&frame).await,
            None => {
                let mut stream = self.inner.conn.quinn().open_uni().await?;
                let _ = stream.set_priority(send_priority(channel));
                // The header goes with the first frame (§3.1, rule 2).
                let mut out = stream_header(channel).to_vec();
                out.extend_from_slice(&frame);
                let r = stream.write_all(&out).await;
                *slot = Some(stream);
                r
            }
        };
        if let Err(e) = result {
            // A stopped or broken stream is done; the next message opens a
            // new one.
            *slot = None;
            return Err(e.into());
        }
        Ok(())
    }

    /// Ends the open stream of a uni channel (FIN); the next message opens
    /// a new one.
    pub async fn finish(&self, channel: Channel) -> Result<(), SessionError> {
        let slot = self
            .inner
            .uni
            .get(&channel.byte())
            .ok_or(SessionError::WrongChannel(channel))?;
        if let Some(mut stream) = slot.lock().await.take() {
            let _ = stream.finish();
        }
        Ok(())
    }

    /// Opens a `files` stream of a transfer (lowest priority; transfers
    /// share the link equally).
    pub async fn open_files(&self) -> Result<FileStream, SessionError> {
        if !self.peer_accepts(Channel::Files) {
            return Err(SessionError::CapabilityMissing(Channel::Files));
        }
        let (mut send, recv) = self.inner.conn.quinn().open_bi().await?;
        let _ = send.set_priority(send_priority(Channel::Files));
        send.write_all(&stream_header(Channel::Files)).await?;
        Ok(FileStream {
            send,
            recv: FramedRecv::new(recv, Channel::Files),
        })
    }

    /// Opens an `actions` stream for one request (§9.2); see
    /// [`crate::actions::request`].
    pub async fn open_actions(&self) -> Result<ActionStream, SessionError> {
        if !self.peer_accepts(Channel::Actions) {
            return Err(SessionError::CapabilityMissing(Channel::Actions));
        }
        let (mut send, recv) = self.inner.conn.quinn().open_bi().await?;
        let _ = send.set_priority(send_priority(Channel::Actions));
        send.write_all(&stream_header(Channel::Actions)).await?;
        Ok(ActionStream {
            send,
            recv: FramedRecv::new(recv, Channel::Actions),
        })
    }

    /// Opens a `find` stream: one ring of the peer (§9.5); see
    /// [`crate::find::ring`].
    pub async fn open_find(&self) -> Result<FindStream, SessionError> {
        if !self.peer_accepts(Channel::Find) {
            return Err(SessionError::CapabilityMissing(Channel::Find));
        }
        let (mut send, recv) = self.inner.conn.quinn().open_bi().await?;
        let _ = send.set_priority(send_priority(Channel::Find));
        send.write_all(&stream_header(Channel::Find)).await?;
        Ok(FindStream {
            send,
            recv: FramedRecv::new(recv, Channel::Find),
        })
    }

    async fn send_control(&self, msg: ControlMessage) -> Result<(), SessionError> {
        send_control(&self.inner, msg).await
    }

    /// Round trip of `Ping` / `Pong` on `control` (§7.4).
    pub async fn ping(&self) -> Result<Duration, SessionError> {
        let nonce = self.inner.next_nonce.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner
            .pings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(nonce, tx);
        let start = Instant::now();
        let result = async {
            self.send_control(ControlMessage::Ping { nonce }).await?;
            match tokio::time::timeout(PING_TIMEOUT, rx).await {
                Ok(Ok(())) => Ok(start.elapsed()),
                Ok(Err(_)) => Err(SessionError::Connection(
                    quinn::ConnectionError::LocallyClosed,
                )),
                Err(_) => Err(SessionError::Timeout),
            }
        }
        .await;
        self.inner
            .pings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&nonce);
        result
    }

    /// Says goodbye (§7.7) and closes the connection with `code`.
    ///
    /// A QUIC close drops data not yet sent, so `Goodbye` is followed by the
    /// end of `control` and a short wait for the peer to take both, before
    /// the close. True when the peer acknowledged it in time (all of
    /// `control` acked after its end), which an unpair notice needs before
    /// it counts as delivered (§12.10).
    pub async fn goodbye(&self, code: ErrorCode, message: Option<String>) -> bool {
        let delivered = {
            let mut control = self.inner.control.lock().await;
            send_goodbye(&mut control, code, message).await
        };
        self.inner.conn.close(code, b"");
        delivered
    }

    /// Waits until the connection is closed.
    pub async fn closed(&self) -> quinn::ConnectionError {
        self.inner.conn.quinn().closed().await
    }

    /// Whether the connection is closed already.
    pub fn is_closed(&self) -> bool {
        self.inner.conn.quinn().close_reason().is_some()
    }
}

/// What to do with a new session when one device may have a session at a
/// time (WC-28: several phones paired with one PC).
#[derive(Debug)]
pub enum Admit {
    /// No live session: take the new one.
    Take,
    /// The same device came back (e.g. after a network change): the new
    /// session replaces its old one, which the caller closes.
    Replace,
    /// Another device has the live session and keeps it. The caller sends
    /// the newcomer `Goodbye` `BUSY`; it doesn't retry on its own until its
    /// network changes or its user asks (§12.10). Replacing is never
    /// automatic, so two phones can't keep throwing each other off.
    Busy,
}

/// CCS/1 RULE:the active session stays; only the same key replaces it (WC-28).
pub fn admit(active: Option<&Session>, incoming: &Session) -> Admit {
    match active {
        Some(a) if !a.is_closed() && a.peer().key != incoming.peer().key => Admit::Busy,
        Some(a) if !a.is_closed() => Admit::Replace,
        _ => Admit::Take,
    }
}

/// How long a session refused with BUSY is watched for the peer's own
/// `Goodbye UNPAIRED` before the answer (§12.10): a phone delivering an
/// unpair notice sends it right after `HelloAck`.
pub const NOTICE_GRACE: Duration = Duration::from_millis(500);

/// How a refused session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnedAway {
    /// The peer only came to say it removed the pairing: the caller drops
    /// its key (`Unpairing::unpaired_by`), the one from this session's TLS.
    Unpaired,
    /// Told `BUSY` (§12.10) and closed.
    Busy,
}

/// Answers a session [`admit`] refused (or one the user sent away).
/// CCS/1 RULE:every accepted session, also a refused one, honours the peer's UNPAIRED (§12.10).
/// Its `Goodbye UNPAIRED` (or a close with that code) within [`NOTICE_GRACE`]
/// wins over `BUSY`, so a phone that removed this computer while another
/// phone held the session doesn't count a notice as delivered that nobody read.
pub async fn turn_away(
    session: &Session,
    events: &mut mpsc::UnboundedReceiver<SessionEvent>,
) -> TurnedAway {
    let unpaired = tokio::time::timeout(NOTICE_GRACE, async {
        loop {
            match events.recv().await {
                Some(SessionEvent::Goodbye(g)) => return g.code == ErrorCode::Unpaired,
                Some(SessionEvent::Closed(_)) | None => {
                    let end = session.closed().await;
                    return close_code(&end) == Some(ErrorCode::Unpaired);
                }
                Some(_) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    if unpaired {
        session.inner.conn.close(ErrorCode::NoError, b"");
        return TurnedAway::Unpaired;
    }
    // WN-110: the notice may land after the grace (high RTT, relay §13) while
    // `Goodbye BUSY` goes out: its sender counts it delivered on our transport's
    // ack. So look once more before the close, and close BUSY only if it isn't there.
    {
        let mut control = session.inner.control.lock().await;
        send_goodbye(&mut control, ErrorCode::Busy, None).await;
    }
    if late_unpaired(session, events) {
        session.inner.conn.close(ErrorCode::NoError, b"");
        return TurnedAway::Unpaired;
    }
    session.inner.conn.close(ErrorCode::Busy, b"");
    TurnedAway::Busy
}

/// Whether the peer said `Goodbye UNPAIRED` (or closed with that code) by now,
/// among the events not read yet.
fn late_unpaired(session: &Session, events: &mut mpsc::UnboundedReceiver<SessionEvent>) -> bool {
    while let Ok(event) = events.try_recv() {
        if matches!(event, SessionEvent::Goodbye(g) if g.code == ErrorCode::Unpaired) {
            return true;
        }
    }
    session
        .inner
        .conn
        .quinn()
        .close_reason()
        .is_some_and(|end| close_code(&end) == Some(ErrorCode::Unpaired))
}

/// The §10 code the peer (or we) closed `error`'s connection with, if it
/// was an application close: e.g. `UNPAIRED` or `BUSY` after `Goodbye`.
pub fn close_code(error: &quinn::ConnectionError) -> Option<ErrorCode> {
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => {
            Some(ErrorCode::from_code(close.error_code.into_inner()))
        }
        _ => None,
    }
}

/// Writes `Goodbye`, ends `control`, and waits up to [`GOODBYE_FLUSH`] for
/// the peer to acknowledge all of it. True only if it did.
async fn send_goodbye(
    control: &mut quinn::SendStream,
    code: ErrorCode,
    message: Option<String>,
) -> bool {
    let Ok(frame) = encode_frame(&Message::Control(ControlMessage::Goodbye(Goodbye {
        code,
        message,
    }))) else {
        return false;
    };
    if control.write_all(&frame).await.is_err() || control.finish().is_err() {
        return false;
    }
    matches!(
        tokio::time::timeout(GOODBYE_FLUSH, control.stopped()).await,
        Ok(Ok(None))
    )
}

/// Ends a notice-mode connection (§12.10): `Goodbye` `UNPAIRED`, then close.
async fn notice(conn: &Connection, control: &mut quinn::SendStream) -> SessionError {
    let delay = testing::NOTICE_DELAY_MS.load(Ordering::Relaxed);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    let mut delivered = send_goodbye(control, ErrorCode::Unpaired, None).await;
    if delivered {
        // WN-110: the peer's transport acked the notice, but its app may have
        // gone the BUSY way already (the notice came after its grace): then it
        // closes with BUSY and kept our key, so the notice is still owed.
        // A peer that read it closes otherwise, or leaves the close to us.
        let end = tokio::time::timeout(GOODBYE_FLUSH, conn.quinn().closed()).await;
        delivered = !matches!(end, Ok(e) if close_code(&e) == Some(ErrorCode::Busy));
    }
    conn.close(ErrorCode::Unpaired, b"");
    SessionError::Unpaired {
        peer: conn.peer_key(),
        delivered,
    }
}

/// Hooks for tests only; not part of the API.
#[doc(hidden)]
pub mod testing {
    use std::sync::atomic::AtomicU64;

    /// Delay (ms) before a notice-mode connection sends `Goodbye UNPAIRED`:
    /// a slow phone or a high RTT (WN-110). Process-wide; 0 in production.
    pub static NOTICE_DELAY_MS: AtomicU64 = AtomicU64::new(0);
}

async fn send_control(inner: &Inner, msg: ControlMessage) -> Result<(), SessionError> {
    let frame = encode_frame(&Message::Control(msg)).map_err(SessionError::Frame)?;
    inner.control.lock().await.write_all(&frame).await?;
    Ok(())
}

/// Reads `control` after the handshake: answers `Ping`, records `Pong`,
/// `Capabilities`, `Error` and `Goodbye`, and closes on violations.
async fn control_loop(inner: Arc<Inner>, mut recv: FramedRecv) {
    let mut last_unknown_error: Option<Instant> = None;
    let reason = loop {
        match recv.next_message().await {
            Ok(Some(Message::Control(msg))) => match msg {
                ControlMessage::Ping { nonce } => {
                    let _ = send_control(&inner, ControlMessage::Pong { nonce }).await;
                }
                ControlMessage::Pong { nonce } => {
                    let waiter = inner
                        .pings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&nonce);
                    if let Some(tx) = waiter {
                        let _ = tx.send(());
                    }
                }
                ControlMessage::Capabilities { caps } => {
                    let number = inner.peer_announcements.fetch_add(1, Ordering::SeqCst) + 1;
                    {
                        let mut a = inner
                            .peer_input_announcement
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *a = has_input(&caps).then(|| InputAnnouncement {
                            last: number,
                            // Still enabled: the session base stays; enabled
                            // again: this announcement is the new base.
                            enabled: a.map_or(number, |prev| prev.enabled),
                        });
                    }
                    *inner.peer_caps.write().unwrap_or_else(|e| e.into_inner()) = caps.clone();
                    withdraw_streams(&inner, &caps).await;
                    let _ = inner.events.send(SessionEvent::Capabilities(caps));
                }
                ControlMessage::Error(e) => {
                    let _ = inner.events.send(SessionEvent::Error(e));
                }
                ControlMessage::Goodbye(g) => {
                    inner.goodbye_received.store(true, Ordering::SeqCst);
                    let _ = inner.events.send(SessionEvent::Goodbye(g));
                }
                ControlMessage::Hello(_) | ControlMessage::HelloAck(_) => {
                    close(
                        &inner.conn,
                        ErrorCode::ProtocolViolation,
                        "second Hello / HelloAck",
                    );
                }
            },
            Ok(Some(_)) => unreachable!("control frames decode as control messages"),
            Ok(None) => {
                if !inner.goodbye_received.load(Ordering::SeqCst) {
                    close(
                        &inner.conn,
                        ErrorCode::ProtocolViolation,
                        "control stream ended",
                    );
                }
                break inner.conn.quinn().closed().await.to_string();
            }
            Err(RecvError::Frame(FrameError::UnknownMessage { channel, ty })) => {
                // §4.4: skip it; answer at most once a second.
                if last_unknown_error.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                    last_unknown_error = Some(Instant::now());
                    let _ = send_control(
                        &inner,
                        ControlMessage::Error(ErrorMessage {
                            code: ErrorCode::UnknownMessage,
                            message: None,
                            channel: Some(u64::from(channel.byte())),
                            ty: Some(ty),
                        }),
                    )
                    .await;
                }
            }
            Err(RecvError::Frame(e)) => {
                close(&inner.conn, e.code(), "bad control frame");
                break format!("{e:?}");
            }
            Err(RecvError::Withdrawn) => break "control withdrawn".into(),
            Err(RecvError::Read(quinn::ReadError::ConnectionLost(e))) => {
                // Already closed: closing again would replace the reason.
                break e.to_string();
            }
            Err(RecvError::Read(e)) => {
                if !inner.goodbye_received.load(Ordering::SeqCst) {
                    close(
                        &inner.conn,
                        ErrorCode::ProtocolViolation,
                        "control stream ended",
                    );
                }
                break e.to_string();
            }
        }
    };
    // Wake every ping waiter.
    inner
        .pings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let _ = inner.events.send(SessionEvent::Closed(reason));
}

/// The peer stopped accepting some channels: end our streams of them
/// (`RESET_STREAM` with `CANCELLED`, §6 rule 5).
async fn withdraw_streams(inner: &Inner, caps: &[Capability]) {
    for (byte, slot) in &inner.uni {
        let Ok(channel) = Channel::from_byte(*byte) else {
            continue;
        };
        let name = capability_name(channel);
        let accepted = caps
            .iter()
            .any(|c| Some(c.name()) == name && c.version() == 1);
        if !accepted {
            if let Some(mut stream) = slot.lock().await.take() {
                let _ = stream.reset(varint_code(ErrorCode::Cancelled));
            }
        }
    }
}

/// Takes the peer's uni streams in stream-number order, reads each header,
/// and hands each stream to its channel's task (§3, §3.1).
async fn uni_loop(
    inner: Arc<Inner>,
    routes: std::collections::BTreeMap<u8, (Channel, u64, mpsc::Sender<ChannelMessage>)>,
    input: Option<mpsc::Sender<InputCmd>>,
) {
    let mut channels: HashMap<u8, mpsc::UnboundedSender<Tagged>> = HashMap::new();
    for (byte, (channel, _, out)) in routes {
        let (tx, rx) = mpsc::unbounded_channel();
        let gate = inner.gates[&byte].subscribe();
        tokio::spawn(channel_task(channel, rx, out, gate));
        channels.insert(byte, tx);
    }
    if let Some(cmds) = input {
        let (tx, rx) = mpsc::unbounded_channel();
        let gate = inner.gates[&Channel::Input.byte()].subscribe();
        tokio::spawn(input_stream_task(rx, cmds, gate));
        channels.insert(Channel::Input.byte(), tx);
    }

    let conn = inner.conn.quinn().clone();
    let mut next: Option<quinn::RecvStream> = None;
    loop {
        let mut stream = match next.take() {
            Some(s) => s,
            None => match conn.accept_uni().await {
                Ok(s) => s,
                Err(_) => return,
            },
        };

        // Read the header. A later stream starts the 2 s clock (§3.1 rule 4).
        let mut header = [0u8; 2];
        let outcome = {
            let read = stream.read_exact(&mut header);
            tokio::pin!(read);
            let mut deadline: Option<Pin<Box<tokio::time::Sleep>>> = None;
            loop {
                tokio::select! {
                    r = &mut read => break Some(r),
                    s = conn.accept_uni(), if next.is_none() => match s {
                        Ok(s) => {
                            next = Some(s);
                            deadline = Some(Box::pin(tokio::time::sleep(HEADER_TIMEOUT)));
                        }
                        Err(_) => return,
                    },
                    _ = async { deadline.as_mut().expect("guarded").await }, if deadline.is_some() => break None,
                }
            }
        };
        match outcome {
            None => {
                let _ = stream.stop(varint_code(ErrorCode::ProtocolViolation));
                continue;
            }
            Some(Ok(())) => {}
            Some(Err(quinn::ReadExactError::FinishedEarly(_))) => {
                let _ = stream.stop(varint_code(ErrorCode::MalformedFrame));
                continue;
            }
            // Reset before its header: counts as finished.
            Some(Err(quinn::ReadExactError::ReadError(quinn::ReadError::Reset(_)))) => continue,
            Some(Err(_)) => return,
        }

        let channel = match decode_stream_header(header) {
            Ok(ch) => ch,
            Err(e) => {
                let _ = stream.stop(varint_code(e.code()));
                continue;
            }
        };
        if !is_uni(channel) {
            close(
                &inner.conn,
                ErrorCode::ProtocolViolation,
                "control or files on a uni stream",
            );
            return;
        }
        // §3 rule 3: taken only while we announce the channel.
        let (Some(route), Some(gate)) = (channels.get(&channel.byte()), inner.gate(channel)) else {
            let _ = stream.stop(varint_code(ErrorCode::CapabilityMissing));
            continue;
        };
        if !gate.enabled {
            let _ = stream.stop(varint_code(ErrorCode::CapabilityMissing));
            continue;
        }
        if route
            .send(Tagged {
                stream,
                withdrawals: gate.withdrawals,
            })
            .is_err()
        {
            return;
        }
    }
}

/// A stream taken for a channel, with the channel's withdrawal count at
/// that moment.
struct Tagged {
    stream: quinn::RecvStream,
    withdrawals: u64,
}

/// Whether `gate` still admits a stream tagged `withdrawals`: no withdrawal
/// since it was taken.
fn admits(gate: &watch::Receiver<Gate>, withdrawals: u64) -> bool {
    let g = *gate.borrow();
    g.enabled && g.withdrawals == withdrawals
}

/// Bytes of one channel's messages read ahead of its module (§3.1, rule
/// 5.6): finished streams are read into memory up to this budget, so they
/// give their place in the shared stream limit back while the module is
/// slow, and other channels keep flowing.
pub const READ_AHEAD_BYTES: usize = 1024 * 1024;
/// What each read-ahead message costs on top of its payload, so that many
/// tiny messages are bounded too.
const READ_AHEAD_OVERHEAD: usize = 64;

/// A message read ahead of the module, holding its share of the budget.
struct Ahead {
    message: RawMessage,
    withdrawals: u64,
    _budget: tokio::sync::OwnedSemaphorePermit,
}

/// One channel's receiving side: a reader that takes the channel's streams
/// in order and reads them ahead, within [`READ_AHEAD_BYTES`], and a
/// delivery task that hands the messages to the module in the same order.
/// A finished stream is let go as soon as it is read, so its place in the
/// shared stream limit returns at once, whatever the module does.
async fn channel_task(
    channel: Channel,
    streams: mpsc::UnboundedReceiver<Tagged>,
    out: mpsc::Sender<ChannelMessage>,
    gate: watch::Receiver<Gate>,
) {
    let budget = Arc::new(tokio::sync::Semaphore::new(READ_AHEAD_BYTES));
    let (ahead_tx, ahead_rx) = mpsc::unbounded_channel();
    tokio::spawn(deliver(channel, ahead_rx, out, gate.clone()));
    read_ahead(channel, streams, ahead_tx, budget, gate).await;
}

/// Hands read-ahead messages to the module in order. A message from before
/// a withdrawal (§6, rule 5) is dropped, also while it waits on a full
/// module queue.
async fn deliver(
    channel: Channel,
    mut ahead: mpsc::UnboundedReceiver<Ahead>,
    out: mpsc::Sender<ChannelMessage>,
    mut gate: watch::Receiver<Gate>,
) {
    while let Some(Ahead {
        message,
        withdrawals,
        _budget,
    }) = ahead.recv().await
    {
        if !admits(&gate, withdrawals) {
            continue;
        }
        let msg = ChannelMessage { channel, message };
        tokio::select! {
            r = out.send(msg) => {
                if r.is_err() {
                    // The module is gone: the reader stops the channel's
                    // streams when it finds this task gone.
                    return;
                }
            }
            _ = gate.wait_for(|g| g.withdrawals != withdrawals) => {}
        }
    }
}

/// Reads one channel's streams one after another, in order, into the
/// read-ahead queue. A withdrawal (§6, rule 5) stops the stream being read
/// and every stream queued before it, with `CANCELLED`.
async fn read_ahead(
    channel: Channel,
    mut streams: mpsc::UnboundedReceiver<Tagged>,
    ahead: mpsc::UnboundedSender<Ahead>,
    budget: Arc<tokio::sync::Semaphore>,
    mut gate: watch::Receiver<Gate>,
) {
    while let Some(Tagged {
        stream,
        withdrawals,
    }) = streams.recv().await
    {
        let mut recv = FramedRecv::new(stream, channel);
        if !admits(&gate, withdrawals) {
            recv.stop(ErrorCode::Cancelled);
            continue;
        }
        let mut lost = false;
        loop {
            // `next_raw` is cancel-safe: its only await is quinn's `read`.
            let next = tokio::select! {
                r = recv.next_raw() => r,
                changed = gate.changed() => {
                    if changed.is_err() || !admits(&gate, withdrawals) {
                        recv.stop(ErrorCode::Cancelled);
                        break;
                    }
                    continue;
                }
            };
            match next {
                Ok(Some(message)) => {
                    // A message larger than the whole budget (frames go up
                    // to 1 MiB) waits until the queue is empty, then goes.
                    let cost =
                        (recv.last_frame_len() + READ_AHEAD_OVERHEAD).min(READ_AHEAD_BYTES) as u32;
                    // Out of budget, this stream waits here and keeps its
                    // place in the stream limit (QUIC flow control holds the
                    // sender), as before read-ahead.
                    let permit = tokio::select! {
                        p = budget.clone().acquire_many_owned(cost) => p,
                        // Withdrawn while waiting: dropped (§6, rule 5).
                        _ = gate.wait_for(|g| g.withdrawals != withdrawals) => {
                            recv.stop(ErrorCode::Cancelled);
                            break;
                        }
                    };
                    let Ok(permit) = permit else {
                        return;
                    };
                    let sent = ahead.send(Ahead {
                        message,
                        withdrawals,
                        _budget: permit,
                    });
                    if sent.is_err() || !admits(&gate, withdrawals) {
                        // The module is gone, or the channel was withdrawn.
                        recv.stop(ErrorCode::Cancelled);
                        break;
                    }
                }
                Ok(None) => break,
                Err(RecvError::Frame(e)) => {
                    recv.stop(e.code());
                    break;
                }
                Err(RecvError::Read(quinn::ReadError::Reset(_))) | Err(RecvError::Withdrawn) => {
                    break
                }
                Err(RecvError::Read(_)) => {
                    lost = true;
                    break;
                }
            }
        }
        if lost {
            return;
        }
    }
}

/// Takes the peer's bidi streams after `control`: `files` and `actions` go
/// to their modules; anything else breaks §3.
async fn bidi_loop(
    inner: Arc<Inner>,
    files: Option<mpsc::Sender<FileStream>>,
    actions: Option<mpsc::Sender<ActionStream>>,
    find: Option<mpsc::Sender<FindStream>>,
) {
    let conn = inner.conn.quinn().clone();
    loop {
        let (mut send, mut recv) = match conn.accept_bi().await {
            Ok(s) => s,
            Err(_) => return,
        };
        let inner = inner.clone();
        let files = files.clone();
        let actions = actions.clone();
        let find = find.clone();
        // Each stream on its own, so a slow header holds back nothing else.
        tokio::spawn(async move {
            let mut header = [0u8; 2];
            if recv.read_exact(&mut header).await.is_err() {
                let _ = recv.stop(varint_code(ErrorCode::MalformedFrame));
                return;
            }
            let (channel, route) = match decode_stream_header(header) {
                Err(e) => {
                    let _ = recv.stop(varint_code(e.code()));
                    let _ = send.reset(varint_code(e.code()));
                    return;
                }
                Ok(Channel::Files) => (Channel::Files, files),
                Ok(Channel::Actions) => (Channel::Actions, actions),
                Ok(Channel::Find) => (Channel::Find, find),
                Ok(_) => {
                    close(
                        &inner.conn,
                        ErrorCode::ProtocolViolation,
                        "second control or a uni channel on a bidi stream",
                    );
                    return;
                }
            };
            // §3 rule 3: taken only while we announce the channel; withdrawn
            // (§6, rule 5), new streams are refused.
            let Some(tx) = route.filter(|_| inner.gate(channel).is_some_and(|g| g.enabled)) else {
                let _ = recv.stop(varint_code(ErrorCode::CapabilityMissing));
                let _ = send.reset(varint_code(ErrorCode::CapabilityMissing));
                return;
            };
            let _ = send.set_priority(send_priority(channel));
            let mut framed = FramedRecv::new(recv, channel);
            if let Some(gate) = inner.gates.get(&channel.byte()) {
                let at = gate.borrow().withdrawals;
                framed = framed.with_gate(gate.subscribe(), at);
            }
            let stream = BidiStream { send, recv: framed };
            if let Err(mpsc::error::SendError(mut s)) = tx.send(stream).await {
                s.recv.stop(ErrorCode::Cancelled);
                let _ = s.send.reset(varint_code(ErrorCode::Cancelled));
            }
        });
    }
}

fn has_input(caps: &[Capability]) -> bool {
    caps.iter().any(|c| c.name() == "input" && c.version() == 1)
}

/// Commands queued to a session's input actor, in arrival order.
enum InputCmd {
    StreamStarted,
    /// A stream frame; the reply says whether to stop the stream, and with
    /// which code.
    StreamFrame(InputMessage, oneshot::Sender<Option<ErrorCode>>),
    StreamEnded,
    Datagram(InputMessage),
    ConnectionEnded,
    /// This side withdrew `input/1`.
    Withdraw,
    /// This side enabled `input/1` again in its announcement number n.
    Enable(u64),
}

/// Commands an input actor queues before its sources wait.
const INPUT_COMMANDS: usize = 1024;

/// Owns the session's input receiver: takes one command at a time and
/// delivers that command's whole batch of events before taking the next, so
/// the module sees events in exactly the order the state machine made them,
/// however the stream and datagram sources interleave and however slowly
/// the module reads.
async fn input_actor(mut cmds: mpsc::Receiver<InputCmd>, out: mpsc::Sender<InputEvent>) {
    let mut receiver = InputReceiver::new(true);
    while let Some(cmd) = cmds.recv().await {
        let mut events = Vec::new();
        match cmd {
            InputCmd::StreamStarted => receiver.stream_started(),
            InputCmd::StreamFrame(msg, reply) => {
                let stop = receiver.stream_frame(&msg, &mut events).err();
                let _ = reply.send(stop);
            }
            InputCmd::StreamEnded => receiver.stream_ended(&mut events),
            InputCmd::Datagram(msg) => receiver.datagram(&msg, &mut events),
            InputCmd::ConnectionEnded => receiver.connection_ended(&mut events),
            InputCmd::Withdraw => receiver.withdraw(&mut events),
            InputCmd::Enable(n) => receiver.enable(n),
        }
        for e in events {
            // A gone module still lets the state machine run to the end.
            let _ = out.send(e).await;
        }
    }
}

/// Reads the peer's `input` streams one after another (each is a session,
/// §8.2) and queues their frames to the input actor.
async fn input_stream_task(
    mut streams: mpsc::UnboundedReceiver<Tagged>,
    cmds: mpsc::Sender<InputCmd>,
    mut gate: watch::Receiver<Gate>,
) {
    while let Some(Tagged {
        stream,
        withdrawals,
    }) = streams.recv().await
    {
        let mut recv = FramedRecv::new(stream, Channel::Input);
        if !admits(&gate, withdrawals) {
            recv.stop(ErrorCode::Cancelled);
            continue;
        }
        if cmds.send(InputCmd::StreamStarted).await.is_err() {
            return;
        }
        let mut lost = false;
        loop {
            // `next_message` is cancel-safe, like `next_raw`.
            let next = tokio::select! {
                r = recv.next_message() => r,
                changed = gate.changed() => {
                    if changed.is_err() || !admits(&gate, withdrawals) {
                        // Withdrawn: the actor already ended the session.
                        recv.stop(ErrorCode::Cancelled);
                        break;
                    }
                    continue;
                }
            };
            let stop = match next {
                Ok(Some(Message::Input(msg))) => {
                    let (tx, rx) = oneshot::channel();
                    if cmds.send(InputCmd::StreamFrame(msg, tx)).await.is_err() {
                        return;
                    }
                    rx.await.ok().flatten()
                }
                Ok(Some(_)) => None,
                Ok(None) => break,
                // §4.4: an unknown type is skipped.
                Err(RecvError::Frame(FrameError::UnknownMessage { .. })) => None,
                Err(RecvError::Frame(e)) => Some(e.code()),
                Err(RecvError::Read(quinn::ReadError::Reset(_))) | Err(RecvError::Withdrawn) => {
                    break
                }
                Err(RecvError::Read(_)) => {
                    lost = true;
                    break;
                }
            };
            if let Some(code) = stop {
                recv.stop(code);
                break;
            }
        }
        if cmds.send(InputCmd::StreamEnded).await.is_err() || lost {
            return;
        }
    }
}

/// Reads the peer's datagrams (§8.1): a bad one is dropped silently.
async fn datagram_loop(conn: quinn::Connection, cmds: mpsc::Sender<InputCmd>) {
    while let Ok(bytes) = conn.read_datagram().await {
        let Ok(msg) = crate::protocol::decode_datagram(&bytes, crate::protocol::MAX_DATAGRAM)
        else {
            continue;
        };
        if cmds.send(InputCmd::Datagram(msg)).await.is_err() {
            return;
        }
    }
    // The connection is gone: release whatever is still pressed, after
    // everything queued before.
    let _ = cmds.send(InputCmd::ConnectionEnded).await;
}
