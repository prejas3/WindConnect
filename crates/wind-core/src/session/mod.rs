//! A protocol session over a pinned QUIC connection (PROTOCOL.md §3, §6,
//! §7; WC-11): the `Hello` / `HelloAck` exchange, the `control` channel,
//! one stream per channel with send priorities, and the [`Router`] that
//! hands each channel's messages to its feature module.
//!
//! Sending: [`Session::send`] keeps at most one open uni stream per channel
//! (§3.1, rule 1) and [`Session::open_files`] one bidi stream per transfer.
//! Every stream gets the §3 priority, so `control` and `input` are sent
//! before bulk channels, and `files` streams share what is left.
//!
//! Receiving: uni streams are taken in stream-number order; a stream's
//! messages reach its module only after every earlier stream's header is in
//! and every earlier stream of the same channel is done (§3.1, rule 3).
//! Each channel is read by its own task, so a slow module or a large
//! transfer holds back only its own channel.
//!
//! §3.1 rule 5 (a receiver MAY refuse a fifth unfinished stream of a
//! channel) is not applied: a receiver can't tell a sender that broke rule
//! 1 from a fast, correct one whose finished streams a slow module hasn't
//! read yet, and refusing the latter loses messages. QUIC's own stream
//! limit (`MAX_STREAMS`) and flow control bound what a peer can open and
//! buffer; a sender over them waits instead of losing data.
//!
//! Not here yet: sending `Capabilities` (changing what this side accepts
//! mid-connection), datagrams (WC-13), keep-alive and resumption (WC-12).

mod framed;
mod router;

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

pub use framed::{write_raw, FramedRecv, RecvError};
pub use router::{capability_name, is_uni, ChannelMessage, FileStream, Router};

use crate::identity::PublicKey;
use crate::protocol::{
    decode_stream_header, encode_frame, stream_header, Capability, Channel, ControlMessage,
    DeviceKind, ErrorCode, ErrorMessage, FrameError, Goodbye, Hello, Message, RawMessage, Version,
    PROTOCOL_MAJOR, PROTOCOL_MINOR,
};
use crate::transport::Connection;

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
        Channel::Media => 4,
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
    Timeout,
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
            SessionError::Timeout => f.write_str("timed out"),
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
        let (mut send, recv) = conn.quinn().open_bi().await?;
        let _ = send.set_priority(send_priority(Channel::Control));
        let frame = encode_frame(&Message::Control(ControlMessage::Hello(hello_for(
            &local, &router,
        ))))
        .map_err(SessionError::Frame)?;
        let mut out = stream_header(Channel::Control).to_vec();
        out.extend_from_slice(&frame);
        send.write_all(&out).await?;

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

        let _ = send.set_priority(send_priority(Channel::Control));
        let ack = encode_frame(&Message::Control(ControlMessage::HelloAck(hello_for(
            &local, &router,
        ))))
        .map_err(SessionError::Frame)?;
        send.write_all(&ack).await?;
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
            Channel::Input,
        ]
        .into_iter()
        .map(|ch| (ch.byte(), tokio::sync::Mutex::new(None)))
        .collect();
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
        });
        tokio::spawn(control_loop(inner.clone(), control_recv));
        tokio::spawn(uni_loop(inner.clone(), router.uni));
        tokio::spawn(bidi_loop(inner.clone(), router.files));
        (Session { inner }, events_rx)
    }

    pub fn peer(&self) -> &PeerInfo {
        &self.inner.peer
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

    /// Opens a `files` stream for one transfer (lowest priority; transfers
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
    /// the close.
    pub async fn goodbye(&self, code: ErrorCode, message: Option<String>) {
        let frame = encode_frame(&Message::Control(ControlMessage::Goodbye(Goodbye {
            code,
            message,
        })));
        if let Ok(frame) = frame {
            let mut control = self.inner.control.lock().await;
            if control.write_all(&frame).await.is_ok() && control.finish().is_ok() {
                let _ = tokio::time::timeout(GOODBYE_FLUSH, control.stopped()).await;
            }
        }
        self.inner.conn.close(code, b"");
    }

    /// Waits until the connection is closed.
    pub async fn closed(&self) -> quinn::ConnectionError {
        self.inner.conn.quinn().closed().await
    }
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
) {
    let mut channels: HashMap<u8, mpsc::UnboundedSender<quinn::RecvStream>> = HashMap::new();
    for (byte, (channel, _, out)) in routes {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(channel_task(channel, rx, out));
        channels.insert(byte, tx);
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
        let Some(route) = channels.get(&channel.byte()) else {
            let _ = stream.stop(varint_code(ErrorCode::CapabilityMissing));
            continue;
        };
        if route.send(stream).is_err() {
            return;
        }
    }
}

/// Reads one channel's streams one after another, in order, and passes
/// their messages to the module.
async fn channel_task(
    channel: Channel,
    mut streams: mpsc::UnboundedReceiver<quinn::RecvStream>,
    out: mpsc::Sender<ChannelMessage>,
) {
    while let Some(stream) = streams.recv().await {
        let mut recv = FramedRecv::new(stream, channel);
        let mut lost = false;
        loop {
            match recv.next_raw().await {
                Ok(Some(message)) => {
                    if out.send(ChannelMessage { channel, message }).await.is_err() {
                        // The module is gone: stop taking this channel.
                        recv.stop(ErrorCode::Cancelled);
                        break;
                    }
                }
                Ok(None) => break,
                Err(RecvError::Frame(e)) => {
                    recv.stop(e.code());
                    break;
                }
                Err(RecvError::Read(quinn::ReadError::Reset(_))) => break,
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

/// Takes the peer's bidi streams after `control`: `files` goes to its
/// module; anything else breaks §3.
async fn bidi_loop(inner: Arc<Inner>, files: Option<(u64, mpsc::Sender<FileStream>)>) {
    let conn = inner.conn.quinn().clone();
    let files = files.map(|(_, tx)| tx);
    loop {
        let (mut send, mut recv) = match conn.accept_bi().await {
            Ok(s) => s,
            Err(_) => return,
        };
        let inner = inner.clone();
        let files = files.clone();
        // Each stream on its own, so a slow header holds back nothing else.
        tokio::spawn(async move {
            let mut header = [0u8; 2];
            if recv.read_exact(&mut header).await.is_err() {
                let _ = recv.stop(varint_code(ErrorCode::MalformedFrame));
                return;
            }
            match decode_stream_header(header) {
                Err(e) => {
                    let _ = recv.stop(varint_code(e.code()));
                    let _ = send.reset(varint_code(e.code()));
                }
                Ok(Channel::Files) => match files {
                    Some(tx) => {
                        let _ = send.set_priority(send_priority(Channel::Files));
                        let stream = FileStream {
                            send,
                            recv: FramedRecv::new(recv, Channel::Files),
                        };
                        if let Err(mpsc::error::SendError(mut s)) = tx.send(stream).await {
                            s.recv.stop(ErrorCode::Cancelled);
                            let _ = s.send.reset(varint_code(ErrorCode::Cancelled));
                        }
                    }
                    None => {
                        let _ = recv.stop(varint_code(ErrorCode::CapabilityMissing));
                        let _ = send.reset(varint_code(ErrorCode::CapabilityMissing));
                    }
                },
                Ok(_) => {
                    close(
                        &inner.conn,
                        ErrorCode::ProtocolViolation,
                        "second control or a uni channel on a bidi stream",
                    );
                }
            }
        });
    }
}
