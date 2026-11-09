//! Pairing (PROTOCOL.md §12, WC-19): two devices learn and pin each
//! other's key, either from a QR code with a one-time secret or by people
//! comparing a 6-digit code (SAS).
//!
//! The device showing the QR code (or waiting for the code comparison) runs
//! a [`PairingListener`]: its own QUIC endpoint with ALPN
//! `windconnect-pair/1`, apart from the main endpoint, which keeps refusing
//! every unpinned key in the TLS handshake (§2). The listener answers only
//! while a pairing *window* is open ([`PairingListener::open_qr`],
//! [`PairingListener::open_sas`]); a window ends on success, after
//! [`PairingConfig::ttl`], or after [`MAX_PAIR_ATTEMPTS`] failed attempts,
//! and its secret is wiped then and there.
//!
//! The other device calls [`pair_with_offer`] (with the scanned link) or
//! [`pair_with_sas`]. On success both sides record the other's key in their
//! [`PairedStore`] and pin it in their [`PinnedKeys`]; from then on the two
//! connect over the main endpoint like any paired devices.
//!
//! Every device key used here (in MACs, the commitment, the code, the
//! record) is the one from the certificate verified in the pairing
//! connection's TLS handshake; no message carries a key.

pub mod crypto;
mod store;
pub mod wire;

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand_core::{OsRng, RngCore};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use zeroize::Zeroizing;

pub use store::{
    parse_paired, PairedDevice, PairedDevicesFile, PairedList, PairedStore, PAIRED_FILE,
};
pub use wire::{clean_name, LinkError, Mode, PairingOffer, MAX_NAME, MAX_PAIR_FRAME};

use crate::identity::{DeviceIdentity, PublicKey, CERT_SERVER_NAME};
use crate::protocol::ErrorCode;
use crate::transport::{
    handshake_peer_key, pairing_client_config, pairing_server_config, EndpointError, PinnedKeys,
};
use crypto::{exporter, EXPORTER_LEN};
use wire::{read_message, write_message, PairMessage, ReadFail, NONCE_LEN, SECRET_LEN};

/// Failed attempts that close a pairing window.
pub const MAX_PAIR_ATTEMPTS: u32 = 3;

/// Time limits of pairing.
#[derive(Debug, Clone, Copy)]
pub struct PairingConfig {
    /// How long a window stays open.
    pub ttl: Duration,
    /// Longest wait for each step of the protocol (handshake, a message).
    pub step_timeout: Duration,
    /// Longest wait for people to accept or reject the code (SAS).
    pub decision_timeout: Duration,
}

impl Default for PairingConfig {
    fn default() -> Self {
        PairingConfig {
            ttl: Duration::from_secs(5 * 60),
            step_timeout: Duration::from_secs(10),
            decision_timeout: Duration::from_secs(60),
        }
    }
}

/// Why an attempt failed (for the UI; the peer only ever sees a §10 code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// The QR proof (MAC) was wrong.
    BadProof,
    /// A step took too long.
    Timeout,
    /// The peer went away before the result.
    Disconnected,
    /// A frame broke §12.3.
    Malformed,
    /// A message out of order, the wrong mode, or a commitment that didn't
    /// match its reveal.
    Violation,
    /// A person rejected the code, or nobody decided in time.
    Rejected,
}

/// How a pairing window ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEnd {
    Paired,
    Expired,
    TooManyAttempts,
    /// A new window was opened in its place.
    Replaced,
    /// [`PairingListener::close_window`].
    Closed,
}

/// What a [`PairingListener`] reports.
#[derive(Debug)]
pub enum PairingEvent {
    /// Show this code and ask whether it matches the other screen.
    Sas(SasPrompt),
    /// A device is paired: recorded and pinned.
    Paired(PairedDevice),
    /// An attempt failed; `failures` so far in this window.
    AttemptFailed { reason: FailReason, failures: u32 },
    /// The window is closed; nothing more is accepted until a new one.
    WindowClosed(WindowEnd),
}

/// A code for a person to compare with the other screen. Dropping it
/// without an answer rejects.
pub struct SasPrompt {
    code: u32,
    peer_key: PublicKey,
    peer_name: Option<String>,
    respond: oneshot::Sender<bool>,
}

impl fmt::Debug for SasPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SasPrompt")
            .field("code", &self.code_text())
            .field("peer_key", &self.peer_key)
            .field("peer_name", &self.peer_name)
            .finish()
    }
}

impl SasPrompt {
    /// The code, 0–999 999.
    pub fn code(&self) -> u32 {
        self.code
    }

    /// The code as shown: `042 917`.
    pub fn code_text(&self) -> String {
        crypto::format_sas(self.code)
    }

    /// The other device's key (from the TLS handshake).
    pub fn peer_key(&self) -> PublicKey {
        self.peer_key
    }

    /// The name the other device gave, cleaned; for display only.
    pub fn peer_name(&self) -> Option<&str> {
        self.peer_name.as_deref()
    }

    pub fn accept(self) {
        let _ = self.respond.send(true);
    }

    pub fn reject(self) {
        let _ = self.respond.send(false);
    }
}

/// Why pairing failed on the dialing side.
#[derive(Debug)]
pub enum PairError {
    /// Our endpoint or TLS configuration couldn't be made.
    Endpoint(EndpointError),
    /// The connection failed (including a server key other than the link's,
    /// refused in the TLS handshake).
    Connection(quinn::ConnectionError),
    /// The other side closed the attempt with this code (§12.9), e.g.
    /// `GOING_AWAY` (no window), `RATE_LIMITED`, `UNTRUSTED_PEER`.
    Refused(ErrorCode),
    /// The server's proof (MAC) was wrong: it doesn't hold the secret.
    BadProof,
    /// A step took too long.
    Timeout,
    /// The other side broke §12.
    Protocol(&'static str),
    /// A person rejected the code (here or there), or nobody decided.
    Rejected,
    /// The paired device couldn't be recorded.
    Store(io::Error),
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairError::Endpoint(e) => write!(f, "pairing endpoint: {e}"),
            PairError::Connection(e) => write!(f, "pairing connection: {e}"),
            PairError::Refused(code) => write!(f, "pairing refused: {code:?}"),
            PairError::BadProof => f.write_str("the other device doesn't hold the pairing secret"),
            PairError::Timeout => f.write_str("pairing timed out"),
            PairError::Protocol(why) => write!(f, "pairing protocol: {why}"),
            PairError::Rejected => f.write_str("the code was rejected"),
            PairError::Store(e) => write!(f, "recording the paired device: {e}"),
        }
    }
}

impl std::error::Error for PairError {}

impl PairError {
    /// The connection was refused because the server's key isn't the one
    /// in the pairing link (TLS `access_denied`), on our side.
    pub fn is_wrong_server_key(&self) -> bool {
        matches!(self, PairError::Connection(e) if crate::transport::is_untrusted_close(e))
    }
}

/// The client side's QUIC configuration for a pairing connection: for
/// tools and tests that speak §12 themselves. `expected`: the server key
/// from the link, checked in TLS.
pub fn client_config(
    identity: &DeviceIdentity,
    expected: Option<PublicKey>,
) -> Result<quinn::ClientConfig, EndpointError> {
    pairing_client_config(identity, expected)
}

fn code_of(code: ErrorCode) -> quinn::VarInt {
    quinn::VarInt::from_u32(code.code() as u32)
}

fn random<const N: usize>() -> Zeroizing<[u8; N]> {
    let mut out = Zeroizing::new([0u8; N]);
    OsRng.fill_bytes(&mut out[..]);
    out
}

// ---------------------------------------------------------------------------
// The listening side.

/// A failed attempt as the listener handles it: what to report and the
/// code to close with (§12.9).
struct Fail {
    reason: FailReason,
    code: ErrorCode,
}

impl Fail {
    fn new(reason: FailReason, code: ErrorCode) -> Self {
        Fail { reason, code }
    }

    fn timeout() -> Self {
        Fail::new(FailReason::Timeout, ErrorCode::HelloTimeout)
    }

    fn violation() -> Self {
        Fail::new(FailReason::Violation, ErrorCode::ProtocolViolation)
    }

    fn read(e: ReadFail) -> Self {
        match e {
            ReadFail::Frame(f) => Fail::new(FailReason::Malformed, f.code()),
            ReadFail::Finished | ReadFail::Read(_) => {
                Fail::new(FailReason::Disconnected, ErrorCode::Cancelled)
            }
        }
    }

    fn disconnected() -> Self {
        Fail::new(FailReason::Disconnected, ErrorCode::Cancelled)
    }
}

enum Window {
    Closed,
    /// Too many failed attempts; refused with `RATE_LIMITED` until a new
    /// window.
    Locked,
    Open(OpenWindow),
}

struct OpenWindow {
    id: u64,
    /// The QR secret; `None` for a SAS window.
    secret: Option<Zeroizing<[u8; SECRET_LEN]>>,
    failures: u32,
    busy: bool,
}

/// One attempt's view of its window.
struct Attempt {
    id: u64,
    secret: Option<Zeroizing<[u8; SECRET_LEN]>>,
}

/// Why a successful attempt couldn't be recorded.
enum CompleteError {
    /// The window ended meanwhile (expired, replaced, closed).
    WindowGone,
    Store,
}

struct Shared {
    window: Mutex<Window>,
    next_id: AtomicU64,
    events: mpsc::UnboundedSender<PairingEvent>,
    key: PublicKey,
    name: Option<String>,
    store: Arc<dyn PairedStore>,
    pins: Arc<PinnedKeys>,
    config: PairingConfig,
}

impl Shared {
    fn emit(&self, event: PairingEvent) {
        let _ = self.events.send(event);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Window> {
        self.window.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn open(&self, secret: Option<Zeroizing<[u8; SECRET_LEN]>>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut w = self.lock();
        let replaced = matches!(*w, Window::Open(_));
        *w = Window::Open(OpenWindow {
            id,
            secret,
            failures: 0,
            busy: false,
        });
        drop(w);
        if replaced {
            self.emit(PairingEvent::WindowClosed(WindowEnd::Replaced));
        }
        id
    }

    /// Takes the window for one attempt, or the code to refuse with. Runs
    /// before any frame is read (§12.9).
    fn begin(&self) -> Result<Attempt, ErrorCode> {
        match &mut *self.lock() {
            Window::Closed => Err(ErrorCode::GoingAway),
            Window::Locked => Err(ErrorCode::RateLimited),
            Window::Open(o) if o.busy => Err(ErrorCode::RateLimited),
            Window::Open(o) => {
                o.busy = true;
                Ok(Attempt {
                    id: o.id,
                    secret: o.secret.clone(),
                })
            }
        }
    }

    /// Counts a failed attempt; the third closes the window.
    fn fail(&self, id: u64, reason: FailReason) {
        let mut w = self.lock();
        let Window::Open(o) = &mut *w else {
            return;
        };
        if o.id != id {
            return;
        }
        o.busy = false;
        o.failures += 1;
        let failures = o.failures;
        if failures >= MAX_PAIR_ATTEMPTS {
            // Wipes the secret now.
            *w = Window::Locked;
        }
        drop(w);
        self.emit(PairingEvent::AttemptFailed { reason, failures });
        if failures >= MAX_PAIR_ATTEMPTS {
            self.emit(PairingEvent::WindowClosed(WindowEnd::TooManyAttempts));
        }
    }

    /// Frees the window without counting (our own failure).
    fn release(&self, id: u64) {
        if let Window::Open(o) = &mut *self.lock() {
            if o.id == id {
                o.busy = false;
            }
        }
    }

    /// Records and pins `device` and closes the window (the secret is
    /// spent), if the attempt's window is still the open one.
    fn complete(&self, id: u64, device: &PairedDevice) -> Result<(), CompleteError> {
        let mut w = self.lock();
        match &*w {
            Window::Open(o) if o.id == id => {}
            _ => return Err(CompleteError::WindowGone),
        }
        if self.store.add(device).is_err() {
            if let Window::Open(o) = &mut *w {
                o.busy = false;
            }
            return Err(CompleteError::Store);
        }
        *w = Window::Closed;
        self.pins.pin(device.key);
        drop(w);
        self.emit(PairingEvent::Paired(device.clone()));
        self.emit(PairingEvent::WindowClosed(WindowEnd::Paired));
        Ok(())
    }

    fn expire(&self, id: u64) {
        let mut w = self.lock();
        if matches!(&*w, Window::Open(o) if o.id == id) {
            *w = Window::Closed;
            drop(w);
            self.emit(PairingEvent::WindowClosed(WindowEnd::Expired));
        }
    }

    fn close(&self) {
        let mut w = self.lock();
        let was_open = matches!(*w, Window::Open(_));
        *w = Window::Closed;
        drop(w);
        if was_open {
            self.emit(PairingEvent::WindowClosed(WindowEnd::Closed));
        }
    }
}

/// The side that shows the QR code or waits for the code comparison.
pub struct PairingListener {
    endpoint: quinn::Endpoint,
    shared: Arc<Shared>,
    events: mpsc::UnboundedReceiver<PairingEvent>,
    runtime: tokio::runtime::Handle,
}

impl fmt::Debug for PairingListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingListener")
            .field("local_addr", &self.endpoint.local_addr().ok())
            .finish_non_exhaustive()
    }
}

impl PairingListener {
    /// A listener on `addr` (port 0 picks one). `name` is what the other
    /// device shows for this one. Must be called inside a Tokio runtime.
    pub fn bind(
        addr: SocketAddr,
        identity: &DeviceIdentity,
        name: Option<&str>,
        store: Arc<dyn PairedStore>,
        pins: Arc<PinnedKeys>,
        config: PairingConfig,
    ) -> Result<Self, EndpointError> {
        let server = pairing_server_config(identity)?;
        let endpoint = quinn::Endpoint::server(server, addr).map_err(EndpointError::Io)?;
        let (tx, events) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            window: Mutex::new(Window::Closed),
            next_id: AtomicU64::new(1),
            events: tx,
            key: identity.public_key(),
            name: name.and_then(clean_name),
            store,
            pins,
            config,
        });
        let runtime = tokio::runtime::Handle::current();
        runtime.spawn(accept_loop(endpoint.clone(), shared.clone()));
        Ok(PairingListener {
            endpoint,
            shared,
            events,
            runtime,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    /// Opens a QR window with a fresh one-time secret (replacing any open
    /// window) and returns what the QR code shows. `addrs`: where the other
    /// device can reach this listener.
    pub fn open_qr(&self, addrs: Vec<SocketAddr>) -> PairingOffer {
        let secret = random::<SECRET_LEN>();
        let offer = PairingOffer::new(self.shared.key, *secret, addrs, self.shared.name.clone());
        let id = self.shared.open(Some(secret));
        self.expire_after(id);
        offer
    }

    /// Opens a window for pairing by code comparison (replacing any open
    /// window).
    pub fn open_sas(&self) {
        let id = self.shared.open(None);
        self.expire_after(id);
    }

    /// Closes the window, if any; its secret is wiped.
    pub fn close_window(&self) {
        self.shared.close();
    }

    /// The next event; `None` never happens while the listener lives.
    pub async fn next_event(&mut self) -> Option<PairingEvent> {
        self.events.recv().await
    }

    fn expire_after(&self, id: u64) {
        let shared = Arc::downgrade(&self.shared);
        let ttl = self.shared.config.ttl;
        self.runtime.spawn(async move {
            tokio::time::sleep(ttl).await;
            if let Some(shared) = shared.upgrade() {
                shared.expire(id);
            }
        });
    }
}

impl Drop for PairingListener {
    fn drop(&mut self) {
        self.shared.close();
        self.endpoint.close(code_of(ErrorCode::GoingAway), b"");
    }
}

async fn accept_loop(endpoint: quinn::Endpoint, shared: Arc<Shared>) {
    while let Some(incoming) = endpoint.accept().await {
        let shared = shared.clone();
        tokio::spawn(async move {
            // A failed TLS handshake (no client certificate, wrong ALPN)
            // never reaches the window.
            if let Ok(conn) = incoming.await {
                serve(shared, conn).await;
            }
        });
    }
}

async fn serve(shared: Arc<Shared>, conn: quinn::Connection) {
    // The window decides first, before any frame is read (§12.9).
    let attempt = match shared.begin() {
        Ok(a) => a,
        Err(code) => {
            conn.close(code_of(code), b"");
            return;
        }
    };
    let Some(peer) = handshake_peer_key(&conn) else {
        shared.release(attempt.id);
        conn.close(code_of(ErrorCode::InternalError), b"");
        return;
    };
    let result = run_server(&shared, &conn, peer, &attempt).await;
    drop(attempt);
    match result {
        Ok(()) => {
            // The client closes once it has the result.
            let _ = timeout(shared.config.step_timeout, conn.closed()).await;
        }
        Err(Outcome::Failed(fail, id)) => {
            shared.fail(id, fail.reason);
            conn.close(code_of(fail.code), b"");
        }
        Err(Outcome::NotCounted(code, id)) => {
            shared.release(id);
            conn.close(code_of(code), b"");
        }
    }
}

enum Outcome {
    /// Counts against the window.
    Failed(Fail, u64),
    /// The window ended meanwhile, or our store failed.
    NotCounted(ErrorCode, u64),
}

async fn run_server(
    shared: &Shared,
    conn: &quinn::Connection,
    peer: PublicKey,
    attempt: &Attempt,
) -> Result<(), Outcome> {
    let id = attempt.id;
    let failed = |f: Fail| Outcome::Failed(f, id);
    let step = shared.config.step_timeout;
    let (mut send, mut recv) = match timeout(step, conn.accept_bi()).await {
        Err(_) => return Err(failed(Fail::timeout())),
        Ok(Err(_)) => return Err(failed(Fail::disconnected())),
        Ok(Ok(s)) => s,
    };
    let e = exporter(conn).ok_or(Outcome::NotCounted(ErrorCode::InternalError, id))?;
    let hello = match timeout(step, read_message(&mut recv)).await {
        Err(_) => return Err(failed(Fail::timeout())),
        Ok(Err(err)) => return Err(failed(Fail::read(err))),
        Ok(Ok(m)) => m,
    };
    let PairMessage::Hello { mode, name, proof } = &hello else {
        return Err(failed(Fail::violation()));
    };
    let device = PairedDevice::now(peer, name.clone());
    match (&attempt.secret, mode) {
        (Some(secret), Mode::Qr) => {
            if !crypto::verify_client_mac(secret, &shared.key, &peer, &e, proof) {
                return Err(failed(Fail::new(
                    FailReason::BadProof,
                    ErrorCode::UntrustedPeer,
                )));
            }
            complete(shared, id, &device)?;
            let result = PairMessage::Result {
                mac: crypto::server_mac(secret, &shared.key, &peer, &e),
                name: shared.name.clone(),
            };
            // Already paired here; a client that doesn't get this simply
            // isn't paired on its side (§12.8).
            let _ = write_message(&mut send, &result).await;
            let _ = send.finish();
            Ok(())
        }
        (None, Mode::Sas) => {
            let commitment = Zeroizing::new(*proof);
            let nonce_s = random::<NONCE_LEN>();
            let nonce = PairMessage::Nonce {
                nonce: *nonce_s,
                name: shared.name.clone(),
            };
            write_message(&mut send, &nonce)
                .await
                .map_err(|_| failed(Fail::disconnected()))?;
            drop(nonce);
            let reveal = match timeout(step, read_message(&mut recv)).await {
                Err(_) => return Err(failed(Fail::timeout())),
                Ok(Err(err)) => return Err(failed(Fail::read(err))),
                Ok(Ok(m)) => m,
            };
            let PairMessage::Reveal { nonce } = &reveal else {
                return Err(failed(Fail::violation()));
            };
            let nonce_c = Zeroizing::new(*nonce);
            drop(reveal);
            if crypto::commit(&peer, &shared.key, &nonce_c) != *commitment {
                return Err(failed(Fail::violation()));
            }
            let code = crypto::sas(&e, &peer, &shared.key, &nonce_c, &nonce_s);
            let (respond, decision) = oneshot::channel();
            shared.emit(PairingEvent::Sas(SasPrompt {
                code,
                peer_key: peer,
                peer_name: name.clone(),
                respond,
            }));
            decide(
                &mut send,
                &mut recv,
                decision,
                shared.config.decision_timeout,
            )
            .await
            .map_err(|d| failed(d.into_fail()))?;
            complete(shared, id, &device)?;
            let _ = send.finish();
            Ok(())
        }
        _ => Err(failed(Fail::violation())),
    }
}

fn complete(shared: &Shared, id: u64, device: &PairedDevice) -> Result<(), Outcome> {
    shared.complete(id, device).map_err(|e| match e {
        CompleteError::WindowGone => Outcome::NotCounted(ErrorCode::GoingAway, id),
        CompleteError::Store => Outcome::NotCounted(ErrorCode::InternalError, id),
    })
}

/// Why the code comparison didn't end with both sides accepting.
enum DecideError {
    /// A person here or there rejected, or nobody here answered.
    Rejected,
    Timeout,
    /// A message other than `Confirm` arrived.
    Unexpected,
    Read(ReadFail),
    Write,
}

impl DecideError {
    fn into_fail(self) -> Fail {
        match self {
            DecideError::Rejected => Fail::new(FailReason::Rejected, ErrorCode::Cancelled),
            DecideError::Timeout => Fail::new(FailReason::Rejected, ErrorCode::Cancelled),
            DecideError::Unexpected => Fail::violation(),
            DecideError::Read(e) => Fail::read(e),
            DecideError::Write => Fail::disconnected(),
        }
    }
}

/// Whether a read failed because the peer closed the connection with `code`.
fn closed_with(e: &ReadFail, code: ErrorCode) -> bool {
    matches!(
        e,
        ReadFail::Read(quinn::ReadExactError::ReadError(quinn::ReadError::ConnectionLost(c)))
            if refused(c) == Some(code)
    )
}

/// Sends this side's decision when the person makes it and reads the other
/// side's; both must accept (§12.6, step 5). Either rejection ends it at
/// once.
async fn decide(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    mut decision: oneshot::Receiver<bool>,
    limit: Duration,
) -> Result<(), DecideError> {
    let deadline = tokio::time::sleep(limit);
    tokio::pin!(deadline);
    let theirs = read_message(recv);
    tokio::pin!(theirs);
    let (mut mine_ok, mut theirs_ok) = (false, false);
    loop {
        tokio::select! {
            d = &mut decision, if !mine_ok => {
                // A dropped prompt is a rejection.
                let accepted = d.unwrap_or(false);
                write_message(send, &PairMessage::Confirm { accepted })
                    .await
                    .map_err(|_| DecideError::Write)?;
                if !accepted {
                    return Err(DecideError::Rejected);
                }
                mine_ok = true;
            }
            m = &mut theirs, if !theirs_ok => {
                let msg = m.map_err(|e| {
                    // A side that rejects sends `Confirm(false)` and closes
                    // with CANCELLED; the close may overtake the message.
                    if closed_with(&e, ErrorCode::Cancelled) {
                        DecideError::Rejected
                    } else {
                        DecideError::Read(e)
                    }
                })?;
                match &msg {
                    PairMessage::Confirm { accepted: true } => theirs_ok = true,
                    PairMessage::Confirm { accepted: false } => return Err(DecideError::Rejected),
                    _ => return Err(DecideError::Unexpected),
                }
            }
            _ = &mut deadline => return Err(DecideError::Timeout),
        }
        if mine_ok && theirs_ok {
            return Ok(());
        }
    }
}

// ---------------------------------------------------------------------------
// The dialing side.

/// The code a connection or stream error carries, if the peer closed with
/// one.
fn refused(e: &quinn::ConnectionError) -> Option<ErrorCode> {
    match e {
        quinn::ConnectionError::ApplicationClosed(c) => {
            Some(ErrorCode::from_code(c.error_code.into_inner()))
        }
        _ => None,
    }
}

fn connection_error(e: quinn::ConnectionError) -> PairError {
    match refused(&e) {
        Some(code) => PairError::Refused(code),
        None => PairError::Connection(e),
    }
}

fn read_error(e: ReadFail) -> PairError {
    match e {
        ReadFail::Frame(_) => PairError::Protocol("malformed frame"),
        ReadFail::Finished => PairError::Protocol("stream ended early"),
        ReadFail::Read(quinn::ReadExactError::ReadError(r)) => match r {
            quinn::ReadError::ConnectionLost(c) => connection_error(c),
            quinn::ReadError::Reset(code) => {
                PairError::Refused(ErrorCode::from_code(code.into_inner()))
            }
            _ => PairError::Protocol("stream failed"),
        },
        ReadFail::Read(quinn::ReadExactError::FinishedEarly(_)) => {
            PairError::Protocol("stream ended early")
        }
    }
}

fn write_error(e: quinn::WriteError) -> PairError {
    match e {
        quinn::WriteError::ConnectionLost(c) => connection_error(c),
        quinn::WriteError::Stopped(code) => {
            PairError::Refused(ErrorCode::from_code(code.into_inner()))
        }
        _ => PairError::Protocol("stream failed"),
    }
}

/// A connection to the first of `addrs` that answers.
async fn connect(
    identity: &DeviceIdentity,
    expected: Option<PublicKey>,
    addrs: &[SocketAddr],
    step: Duration,
) -> Result<(quinn::Endpoint, quinn::Connection), PairError> {
    let config = pairing_client_config(identity, expected).map_err(PairError::Endpoint)?;
    let mut last = PairError::Protocol("no address to reach");
    for addr in addrs {
        let bind: SocketAddr = if addr.is_ipv4() {
            (Ipv4Addr::UNSPECIFIED, 0).into()
        } else {
            (Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let mut endpoint =
            quinn::Endpoint::client(bind).map_err(|e| PairError::Endpoint(EndpointError::Io(e)))?;
        endpoint.set_default_client_config(config.clone());
        let connecting = match endpoint.connect(*addr, CERT_SERVER_NAME) {
            Ok(c) => c,
            Err(_) => continue,
        };
        match timeout(step, connecting).await {
            Ok(Ok(conn)) => return Ok((endpoint, conn)),
            Ok(Err(e)) => last = connection_error(e),
            Err(_) => last = PairError::Timeout,
        }
    }
    Err(last)
}

/// Closes our side: `NO_ERROR` after success, else the code fitting the
/// failure; then waits briefly for the close to go out.
async fn hang_up(
    endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    result: &Result<PairedDevice, PairError>,
) {
    let code = match result {
        Ok(_) => ErrorCode::NoError,
        Err(PairError::BadProof) => ErrorCode::UntrustedPeer,
        Err(PairError::Rejected) | Err(PairError::Timeout) => ErrorCode::Cancelled,
        Err(PairError::Protocol(_)) => ErrorCode::ProtocolViolation,
        Err(_) => ErrorCode::Cancelled,
    };
    conn.close(code_of(code), b"");
    let _ = timeout(Duration::from_secs(1), endpoint.wait_idle()).await;
}

/// Pairs with the device that showed `offer` (its QR code): proves holding
/// the secret, checks the other side's proof, records and pins its key.
pub async fn pair_with_offer(
    identity: &DeviceIdentity,
    offer: &PairingOffer,
    name: Option<&str>,
    store: &dyn PairedStore,
    pins: &PinnedKeys,
    config: &PairingConfig,
) -> Result<PairedDevice, PairError> {
    let (endpoint, conn) =
        connect(identity, Some(offer.key), &offer.addrs, config.step_timeout).await?;
    let result = qr_client(identity, offer, name, store, pins, config, &conn).await;
    hang_up(endpoint, conn, &result).await;
    result
}

async fn qr_client(
    identity: &DeviceIdentity,
    offer: &PairingOffer,
    name: Option<&str>,
    store: &dyn PairedStore,
    pins: &PinnedKeys,
    config: &PairingConfig,
    conn: &quinn::Connection,
) -> Result<PairedDevice, PairError> {
    let me = identity.public_key();
    let server = handshake_peer_key(conn).ok_or(PairError::Protocol("no server key"))?;
    if server != offer.key {
        // The TLS verifier already refused any other key.
        return Err(PairError::BadProof);
    }
    let e = exporter(conn).ok_or(PairError::Protocol("no TLS exporter"))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(connection_error)?;
    let hello = PairMessage::Hello {
        mode: Mode::Qr,
        name: name.map(String::from),
        proof: crypto::client_mac(offer.secret(), &server, &me, &e),
    };
    write_message(&mut send, &hello)
        .await
        .map_err(write_error)?;
    drop(hello);
    let msg = timeout(config.step_timeout, read_message(&mut recv))
        .await
        .map_err(|_| PairError::Timeout)?
        .map_err(read_error)?;
    let PairMessage::Result {
        mac,
        name: their_name,
    } = &msg
    else {
        return Err(PairError::Protocol("expected the pairing result"));
    };
    if !crypto::verify_server_mac(offer.secret(), &server, &me, &e, mac) {
        return Err(PairError::BadProof);
    }
    let device = PairedDevice::now(server, their_name.clone().or_else(|| offer.name.clone()));
    store.add(&device).map_err(PairError::Store)?;
    pins.pin(server);
    let _ = send.finish();
    Ok(device)
}

/// Pairs with the device listening at `addr` by code comparison: both
/// screens show the code, and `on_prompt` gets ours; both people must
/// accept.
pub async fn pair_with_sas(
    identity: &DeviceIdentity,
    addr: SocketAddr,
    name: Option<&str>,
    store: &dyn PairedStore,
    pins: &PinnedKeys,
    config: &PairingConfig,
    on_prompt: impl FnOnce(SasPrompt) + Send,
) -> Result<PairedDevice, PairError> {
    let (endpoint, conn) = connect(identity, None, &[addr], config.step_timeout).await?;
    let result = sas_client(identity, name, store, pins, config, &conn, on_prompt).await;
    hang_up(endpoint, conn, &result).await;
    result
}

async fn sas_client(
    identity: &DeviceIdentity,
    name: Option<&str>,
    store: &dyn PairedStore,
    pins: &PinnedKeys,
    config: &PairingConfig,
    conn: &quinn::Connection,
    on_prompt: impl FnOnce(SasPrompt),
) -> Result<PairedDevice, PairError> {
    let me = identity.public_key();
    let server = handshake_peer_key(conn).ok_or(PairError::Protocol("no server key"))?;
    let e: Zeroizing<[u8; EXPORTER_LEN]> =
        exporter(conn).ok_or(PairError::Protocol("no TLS exporter"))?;
    let nonce_c = random::<NONCE_LEN>();
    let (mut send, mut recv) = conn.open_bi().await.map_err(connection_error)?;
    let hello = PairMessage::Hello {
        mode: Mode::Sas,
        name: name.map(String::from),
        proof: crypto::commit(&me, &server, &nonce_c),
    };
    write_message(&mut send, &hello)
        .await
        .map_err(write_error)?;
    drop(hello);
    // N_c goes out only after N_s is in (§12.6).
    let msg = timeout(config.step_timeout, read_message(&mut recv))
        .await
        .map_err(|_| PairError::Timeout)?
        .map_err(read_error)?;
    let PairMessage::Nonce {
        nonce,
        name: their_name,
    } = &msg
    else {
        return Err(PairError::Protocol("expected the server nonce"));
    };
    let nonce_s = Zeroizing::new(*nonce);
    let their_name = their_name.clone();
    drop(msg);
    write_message(&mut send, &PairMessage::Reveal { nonce: *nonce_c })
        .await
        .map_err(write_error)?;
    let code = crypto::sas(&e, &me, &server, &nonce_c, &nonce_s);
    let (respond, decision) = oneshot::channel();
    on_prompt(SasPrompt {
        code,
        peer_key: server,
        peer_name: their_name.clone(),
        respond,
    });
    decide(&mut send, &mut recv, decision, config.decision_timeout)
        .await
        .map_err(|d| match d {
            DecideError::Rejected => PairError::Rejected,
            DecideError::Timeout => PairError::Timeout,
            DecideError::Unexpected => PairError::Protocol("unexpected message"),
            DecideError::Read(e) => read_error(e),
            DecideError::Write => PairError::Protocol("stream failed"),
        })?;
    let device = PairedDevice::now(server, their_name);
    store.add(&device).map_err(PairError::Store)?;
    pins.pin(server);
    let _ = send.finish();
    Ok(device)
}

#[cfg(test)]
mod tests;
