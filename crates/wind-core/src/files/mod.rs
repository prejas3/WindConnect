//! The file transfer engine (`files/1`, PROTOCOL.md §9.1; WC-38).
//!
//! One file per transfer, in chunks of [`FilesConfig::chunk`] bytes with a
//! SHA-256 each, over up to [`FilesConfig::streams`] `files` streams at
//! once. Transfers to a device wait in a queue, [`FilesConfig::concurrent`]
//! running at a time. A transfer broken by a lost connection waits, and
//! resumes by itself on the next [`FileEngine::attach`]: the receiver
//! answers the repeated `Offer` with the chunks it already holds on disk.
//!
//! CCS/1 FLOW:send() -> queue -> attach(session) -> Offer/Accept -> N streams of Chunk/Ack -> Complete
//! CCS/1 SEC:paths and limits checked on receipt (path.rs); partials per device (store.rs).
//!
//! The application calls [`FileEngine::attach`] whenever a session with a
//! device starts (with the queue from
//! [`Router::route_files`](crate::session::Router::route_files) to receive),
//! and watches [`FileEngine::subscribe`] for progress.

pub mod path;
pub mod store;
pub mod wire;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime};

use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch, Notify};

use crate::protocol::{Channel, ErrorCode, FrameError};
use crate::session::{varint_code, write_raw, FileStream, RecvError, Session, SessionError};

pub use path::PathError;
use store::Partial;
pub use wire::Id as TransferId;
use wire::{Offer, ToReceiver, ToSender};

/// Wait for a `Join`'s `Offer` (§9.1).
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(5);
/// Wait for the first message of an incoming stream.
pub const FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);
/// Most incoming transfers at once, over all devices.
pub const MAX_INCOMING: usize = 16;
/// Most streams waiting for their `Offer`, over all devices.
pub const MAX_PENDING_JOINS: usize = 32;
/// A chunk failing its checksum more often than this ends the transfer.
pub const MAX_BAD: u8 = 3;
/// The receiver records its chunks every this many.
pub const FLUSH_EVERY: u64 = 32;
/// A transfer broken off while its session lives is retried after this.
pub const LOST_RETRY: Duration = Duration::from_secs(1);
/// Progress is published at most this often (state changes at once).
pub const PUBLISH_EVERY: Duration = Duration::from_millis(100);

/// Engine settings.
#[derive(Debug, Clone)]
pub struct FilesConfig {
    /// Where incoming files go (created if missing).
    pub inbox: PathBuf,
    /// Where partial incoming files wait (created if missing): the app's
    /// own state folder, outside `inbox` (refused inside it, so no path
    /// from a peer reaches it). Best on the inbox's volume: then finishing
    /// is a rename, else a copy.
    pub partials: PathBuf,
    /// Streams per transfer, 1–8.
    pub streams: u64,
    /// Outgoing transfers running at once, per device.
    pub concurrent: usize,
    /// Chunk size: a multiple of 64 KiB, 64–512 KiB.
    pub chunk: u32,
    /// Partial incoming files unchanged this long are deleted at start.
    pub partial_max_age: Duration,
    /// Free space kept on top of an incoming file.
    pub space_margin: u64,
}

impl FilesConfig {
    pub fn new(inbox: impl Into<PathBuf>, partials: impl Into<PathBuf>) -> Self {
        FilesConfig {
            inbox: inbox.into(),
            partials: partials.into(),
            streams: 4,
            concurrent: 2,
            chunk: 256 * 1024,
            partial_max_age: Duration::from_secs(7 * 24 * 3600),
            space_margin: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

/// Why a transfer failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailReason {
    /// The receiver has no room for the file (`NO_SPACE`).
    NoSpace,
    /// The source file changed while it was being sent.
    SourceChanged,
    /// The other side refused or stopped it with this code.
    Refused(ErrorCode),
    /// The other side broke §9.1.
    Protocol(String),
    /// A local file error.
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    /// Waiting for its turn (outgoing).
    Queued,
    Running,
    /// Broken off with the connection; resumes on the next session.
    Waiting,
    Done,
    Failed(FailReason),
    Cancelled {
        by_peer: bool,
    },
}

impl TransferState {
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_) | Self::Cancelled { .. })
    }
}

/// One transfer as the application shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferInfo {
    pub id: TransferId,
    /// The other device's key fingerprint.
    pub peer: [u8; 32],
    pub direction: Direction,
    /// The relative path offered (`/`-separated).
    pub path: String,
    pub size: u64,
    /// Bytes verified by the receiver.
    pub done: u64,
    pub state: TransferState,
    /// Outgoing: the file being sent.
    pub source: Option<PathBuf>,
    /// Incoming, once done: where the file went.
    pub saved_as: Option<PathBuf>,
}

/// Why a call failed.
#[derive(Debug)]
pub enum FilesError {
    Io(io::Error),
    /// A name the receiver would refuse (§9.1).
    BadPath(String, PathError),
    /// No such transfer.
    Unknown,
    /// The transfer is not in a state this call applies to.
    WrongState,
}

impl fmt::Display for FilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::BadPath(p, why) => write!(f, "{p:?} cannot be sent: {why:?}"),
            Self::Unknown => f.write_str("no such transfer"),
            Self::WrongState => f.write_str("not possible in this state"),
        }
    }
}

impl std::error::Error for FilesError {}

impl From<io::Error> for FilesError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Hex of a transfer id, for logs and file names.
pub fn id_hex(id: &TransferId) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a source looked like when it was offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<u64>,
}

fn stamp_of(meta: &std::fs::Metadata) -> Stamp {
    Stamp {
        size: meta.len(),
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64),
    }
}

struct Out {
    info: TransferInfo,
    /// Set while running: cancels it.
    cancel: Option<watch::Sender<bool>>,
}

struct In {
    info: TransferInfo,
    run: Option<Arc<InRun>>,
}

#[derive(Default)]
struct Book {
    order: Vec<(Direction, [u8; 32], TransferId)>,
    out: HashMap<TransferId, Out>,
    inc: HashMap<([u8; 32], TransferId), In>,
    /// The live session per device, with a number telling attaches apart.
    sessions: HashMap<[u8; 32], (u64, Session)>,
}

struct Inner {
    config: FilesConfig,
    book: Mutex<Book>,
    infos: watch::Sender<Vec<TransferInfo>>,
    dirty: AtomicBool,
    last_publish: Mutex<Instant>,
    pending_joins: AtomicUsize,
    offers: Notify,
    attaches: AtomicU64,
}

/// The engine: one per application, for every paired device.
#[derive(Clone)]
pub struct FileEngine {
    inner: Arc<Inner>,
}

impl FileEngine {
    /// Requires a Tokio runtime. Creates the inbox and the partials folder
    /// (`InvalidInput` if either lies inside the other) and deletes partial files
    /// older than [`FilesConfig::partial_max_age`].
    pub fn new(mut config: FilesConfig) -> io::Result<Self> {
        config.streams = config.streams.clamp(1, wire::MAX_STREAMS);
        config.concurrent = config.concurrent.max(1);
        let chunk = config.chunk.clamp(wire::MIN_CHUNK, wire::MAX_CHUNK);
        config.chunk = chunk - chunk % wire::MIN_CHUNK;
        std::fs::create_dir_all(&config.inbox)?;
        std::fs::create_dir_all(&config.partials)?;
        let (inbox, partials) = (
            config.inbox.canonicalize()?,
            config.partials.canonicalize()?,
        );
        if partials.starts_with(&inbox) || inbox.starts_with(&partials) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the partials folder must lie outside the inbox",
            ));
        }
        store::clean(&config.partials, config.partial_max_age);
        let inner = Arc::new(Inner {
            config,
            book: Mutex::new(Book::default()),
            infos: watch::channel(Vec::new()).0,
            dirty: AtomicBool::new(false),
            last_publish: Mutex::new(Instant::now()),
            pending_joins: AtomicUsize::new(0),
            offers: Notify::new(),
            attaches: AtomicU64::new(0),
        });
        tokio::spawn(publisher(Arc::downgrade(&inner)));
        Ok(FileEngine { inner })
    }

    pub fn config(&self) -> &FilesConfig {
        &self.inner.config
    }

    /// Every transfer, oldest first; updated on each change of state and at
    /// most every [`PUBLISH_EVERY`] for progress.
    pub fn subscribe(&self) -> watch::Receiver<Vec<TransferInfo>> {
        self.inner.infos.subscribe()
    }

    pub fn transfers(&self) -> Vec<TransferInfo> {
        self.inner.snapshot()
    }

    /// Starts using `session` for its device: queued and waiting transfers
    /// to it start or resume, and incoming streams (from
    /// `Router::route_files`) are received. Call it on every (re)connect.
    pub fn attach(&self, session: Session, incoming: Option<mpsc::Receiver<FileStream>>) {
        let peer = *session.peer().key.fingerprint().as_bytes();
        let number = self.inner.attaches.fetch_add(1, Ordering::Relaxed);
        self.inner
            .lock()
            .sessions
            .insert(peer, (number, session.clone()));
        let weak = Arc::downgrade(&self.inner);
        let watched = session.clone();
        tokio::spawn(async move {
            watched.closed().await;
            if let Some(inner) = weak.upgrade() {
                let mut book = inner.lock();
                if book.sessions.get(&peer).is_some_and(|(n, _)| *n == number) {
                    book.sessions.remove(&peer);
                }
            }
        });
        if let Some(mut incoming) = incoming {
            let weak = Arc::downgrade(&self.inner);
            tokio::spawn(async move {
                while let Some(stream) = incoming.recv().await {
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    tokio::spawn(receive_stream(inner, peer, stream));
                }
            });
        }
        schedule(&self.inner);
    }

    /// Queues `path` for the device `peer` (its key fingerprint). A folder
    /// becomes one transfer per file inside it, keeping relative paths;
    /// links are skipped. Nothing is queued if any name would be refused.
    pub fn send(&self, peer: [u8; 32], path: &Path) -> Result<Vec<TransferId>, FilesError> {
        let meta = std::fs::metadata(path)?;
        let base = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| FilesError::BadPath(path.display().to_string(), PathError::Empty))?
            .to_owned();
        let mut files = Vec::new();
        if meta.is_dir() {
            walk(path, &base, &mut files)?;
        } else {
            files.push((path.to_path_buf(), base, meta.len()));
        }
        for (_, rel, _) in &files {
            path::check(rel).map_err(|e| FilesError::BadPath(rel.clone(), e))?;
        }
        let mut ids = Vec::new();
        {
            let mut book = self.inner.lock();
            for (source, rel, size) in files {
                let mut id = [0u8; 16];
                OsRng.fill_bytes(&mut id);
                book.order.push((Direction::Outgoing, peer, id));
                book.out.insert(
                    id,
                    Out {
                        info: TransferInfo {
                            id,
                            peer,
                            direction: Direction::Outgoing,
                            path: rel,
                            size,
                            done: 0,
                            state: TransferState::Queued,
                            source: Some(source),
                            saved_as: None,
                        },
                        cancel: None,
                    },
                );
                ids.push(id);
            }
        }
        self.inner.publish(true);
        schedule(&self.inner);
        Ok(ids)
    }

    /// Cancels a transfer either way. The other side sees `CANCELLED`, and
    /// a partial incoming file is deleted.
    pub fn cancel(&self, id: &TransferId) -> Result<(), FilesError> {
        let mut book = self.inner.lock();
        if let Some(out) = book.out.get_mut(id) {
            if out.info.state.is_finished() {
                return Err(FilesError::WrongState);
            }
            match &out.cancel {
                Some(c) => {
                    c.send_replace(true);
                }
                None => out.info.state = TransferState::Cancelled { by_peer: false },
            }
        } else {
            let key = book
                .inc
                .keys()
                .find(|(_, i)| i == id)
                .copied()
                .ok_or(FilesError::Unknown)?;
            let inc = book.inc.get_mut(&key).expect("found above");
            if inc.info.state.is_finished() {
                return Err(FilesError::WrongState);
            }
            match &inc.run {
                Some(run) => {
                    run.cancel
                        .send_replace(Some(End::Cancelled { by_peer: false }));
                }
                None => {
                    inc.info.state = TransferState::Cancelled { by_peer: false };
                    store::discard(&self.inner.config.partials, &key.0, &key.1);
                }
            }
        }
        drop(book);
        self.inner.publish(true);
        Ok(())
    }

    /// Queues a failed or cancelled outgoing transfer again.
    pub fn retry(&self, id: &TransferId) -> Result<(), FilesError> {
        let mut book = self.inner.lock();
        let out = book.out.get_mut(id).ok_or(FilesError::Unknown)?;
        if !matches!(
            out.info.state,
            TransferState::Failed(_) | TransferState::Cancelled { .. }
        ) {
            return Err(FilesError::WrongState);
        }
        out.info.state = TransferState::Queued;
        out.info.done = 0;
        drop(book);
        self.inner.publish(true);
        schedule(&self.inner);
        Ok(())
    }

    /// Drops a finished transfer from the list.
    pub fn forget(&self, id: &TransferId) -> Result<(), FilesError> {
        let mut book = self.inner.lock();
        let finished = |s: &TransferState| s.is_finished();
        if book.out.get(id).is_some_and(|o| finished(&o.info.state)) {
            book.out.remove(id);
        } else if let Some(key) = book
            .inc
            .iter()
            .find(|((_, i), t)| i == id && finished(&t.info.state))
            .map(|(k, _)| *k)
        {
            book.inc.remove(&key);
        } else {
            return Err(FilesError::WrongState);
        }
        book.order.retain(|(_, _, i)| i != id);
        drop(book);
        self.inner.publish(true);
        Ok(())
    }
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<(PathBuf, String, u64)>) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let meta = std::fs::symlink_metadata(entry.path())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = format!("{rel}/{name}");
        if meta.file_type().is_symlink() {
            continue;
        } else if meta.is_dir() {
            walk(&entry.path(), &child, out)?;
        } else if meta.is_file() {
            out.push((entry.path(), child, meta.len()));
        }
    }
    Ok(())
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn snapshot(&self) -> Vec<TransferInfo> {
        let book = self.lock();
        book.order
            .iter()
            .filter_map(|(dir, peer, id)| match dir {
                Direction::Outgoing => book.out.get(id).map(|o| o.info.clone()),
                Direction::Incoming => book.inc.get(&(*peer, *id)).map(|i| i.info.clone()),
            })
            .collect()
    }

    /// Publishes now when `now` or the last publish is old enough; else
    /// leaves it to the publisher task.
    fn publish(&self, now: bool) {
        let mut last = self.last_publish.lock().unwrap_or_else(|e| e.into_inner());
        if now || last.elapsed() >= PUBLISH_EVERY {
            *last = Instant::now();
            drop(last);
            self.dirty.store(false, Ordering::Relaxed);
            self.infos.send_replace(self.snapshot());
        } else {
            self.dirty.store(true, Ordering::Relaxed);
        }
    }

    fn set_out(&self, id: &TransferId, f: impl FnOnce(&mut Out)) {
        if let Some(out) = self.lock().out.get_mut(id) {
            f(out);
        }
    }

    fn set_in(&self, key: &([u8; 32], TransferId), f: impl FnOnce(&mut In)) {
        if let Some(inc) = self.lock().inc.get_mut(key) {
            f(inc);
        }
    }
}

async fn publisher(inner: Weak<Inner>) {
    let mut tick = tokio::time::interval(PUBLISH_EVERY);
    loop {
        tick.tick().await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        if inner.dirty.load(Ordering::Relaxed) {
            inner.publish(true);
        }
    }
}

/// Starts queued and waiting outgoing transfers on live sessions, up to
/// [`FilesConfig::concurrent`] per device.
fn schedule(inner: &Arc<Inner>) {
    let mut started = Vec::new();
    {
        let mut book = inner.lock();
        let Book {
            order,
            out,
            sessions,
            ..
        } = &mut *book;
        for (peer, (_, session)) in sessions.iter() {
            if session.connection().quinn().close_reason().is_some() {
                continue;
            }
            let mut running = out
                .values()
                .filter(|o| o.info.peer == *peer && o.info.state == TransferState::Running)
                .count();
            for (dir, p, id) in order.iter() {
                if running >= inner.config.concurrent {
                    break;
                }
                if *dir != Direction::Outgoing || p != peer {
                    continue;
                }
                let Some(o) = out.get_mut(id) else { continue };
                if !matches!(o.info.state, TransferState::Queued | TransferState::Waiting) {
                    continue;
                }
                let (tx, rx) = watch::channel(false);
                o.cancel = Some(tx);
                o.info.state = TransferState::Running;
                running += 1;
                started.push((
                    session.clone(),
                    *id,
                    o.info.source.clone().unwrap_or_default(),
                    o.info.path.clone(),
                    rx,
                ));
            }
        }
    }
    if started.is_empty() {
        return;
    }
    inner.publish(true);
    for (session, id, source, rel, cancel) in started {
        let inner = inner.clone();
        tokio::spawn(async move {
            let end = send_file(&inner, &session, id, &source, &rel, cancel).await;
            let lost = end == End::Lost;
            inner.set_out(&id, |o| {
                o.cancel = None;
                o.info.state = match end {
                    End::Done => TransferState::Done,
                    End::Lost => TransferState::Waiting,
                    End::Cancelled { by_peer } => TransferState::Cancelled { by_peer },
                    End::Failed(why) => TransferState::Failed(why),
                };
            });
            inner.publish(true);
            if lost {
                // Usually the session is gone and the next attach resumes;
                // if it is not, don't spin on a peer that keeps resetting.
                tokio::time::sleep(LOST_RETRY).await;
            }
            schedule(&inner);
        });
    }
}

/// How a transfer (or one of its streams) ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    Done,
    /// The connection went; it resumes later.
    Lost,
    Cancelled {
        by_peer: bool,
    },
    Failed(FailReason),
}

/// What a stop/reset code from the other side means.
fn end_of_code(code: u64) -> End {
    match ErrorCode::from_code(code) {
        ErrorCode::Cancelled => End::Cancelled { by_peer: true },
        ErrorCode::NoSpace => End::Failed(FailReason::NoSpace),
        // The other side is going away: the transfer resumes later.
        ErrorCode::GoingAway | ErrorCode::NoError => End::Lost,
        other => End::Failed(FailReason::Refused(other)),
    }
}

fn end_of_session(e: SessionError) -> End {
    match e {
        SessionError::Connection(_) => End::Lost,
        SessionError::Stopped(code) => end_of_code(code),
        SessionError::Write(quinn::WriteError::ClosedStream)
        | SessionError::Write(quinn::WriteError::ZeroRttRejected) => End::Lost,
        SessionError::CapabilityMissing(_) => {
            End::Failed(FailReason::Refused(ErrorCode::CapabilityMissing))
        }
        SessionError::Recv(r) => end_of_recv(r),
        other => End::Failed(FailReason::Protocol(other.to_string())),
    }
}

fn end_of_recv(e: RecvError) -> End {
    match e {
        RecvError::Read(quinn::ReadError::Reset(code)) => end_of_code(code.into_inner()),
        RecvError::Read(_) => End::Lost,
        RecvError::Frame(f) => End::Failed(FailReason::Protocol(format!("{f:?}"))),
    }
}

fn frame_code(e: &RecvError) -> ErrorCode {
    match e {
        RecvError::Frame(f) => f.code(),
        RecvError::Read(_) => ErrorCode::Cancelled,
    }
}

/// Ends both halves of a stream with `code`.
fn abort(stream: &mut FileStream, code: ErrorCode) {
    stream.recv.stop(code);
    let _ = stream.send.reset(varint_code(code));
}

fn code_of(end: &End) -> ErrorCode {
    match end {
        End::Cancelled { .. } => ErrorCode::Cancelled,
        End::Failed(FailReason::NoSpace) => ErrorCode::NoSpace,
        End::Failed(FailReason::Protocol(_)) => ErrorCode::MalformedFrame,
        End::Failed(FailReason::Refused(c)) => *c,
        End::Done | End::Lost => ErrorCode::Cancelled,
        End::Failed(_) => ErrorCode::Cancelled,
    }
}

/// Waits until a watched option is set; its value (`None` if the sender went).
async fn until_set<T: Clone>(rx: &mut watch::Receiver<Option<T>>) -> Option<T> {
    rx.wait_for(Option::is_some)
        .await
        .ok()
        .and_then(|v| v.clone())
}

/// Waits until a watched flag is set (forever if the sender went).
async fn until_true(rx: &mut watch::Receiver<bool>) {
    if rx.wait_for(|c| *c).await.is_err() {
        std::future::pending::<()>().await;
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f)
        .await
        .expect("blocking file work does not panic")
}

// ---------------------------------------------------------------- sending

/// Chunks still to send and what has come back.
struct Work {
    queue: VecDeque<u64>,
    in_flight: HashSet<u64>,
    bad: HashMap<u64, u8>,
    left: u64,
}

struct SendShared {
    work: Mutex<Work>,
    changed: Notify,
    file: Arc<File>,
    source: PathBuf,
    stamp: Stamp,
    offer: Offer,
    /// The first end of any stream: ends the transfer.
    end: watch::Sender<Option<End>>,
}

impl SendShared {
    fn finish(&self, end: End) {
        self.end.send_if_modified(|e| {
            if e.is_none() {
                *e = Some(end);
                true
            } else {
                false
            }
        });
    }
}

/// The chunk size for `size`: the configured one, larger if the file needs
/// more than [`wire::MAX_CHUNKS`].
fn chunk_for(size: u64, configured: u32) -> Option<u32> {
    let need = size.div_ceil(wire::MAX_CHUNKS);
    let chunk = u64::from(configured)
        .max(need.div_ceil(u64::from(wire::MIN_CHUNK)) * u64::from(wire::MIN_CHUNK));
    (chunk <= u64::from(wire::MAX_CHUNK)).then_some(chunk as u32)
}

async fn send_file(
    inner: &Arc<Inner>,
    session: &Session,
    id: TransferId,
    source: &Path,
    rel: &str,
    mut cancel: watch::Receiver<bool>,
) -> End {
    let opened = {
        let source = source.to_path_buf();
        blocking(move || {
            let file = File::open(&source)?;
            let stamp = stamp_of(&file.metadata()?);
            io::Result::Ok((file, stamp))
        })
        .await
    };
    let (file, stamp) = match opened {
        Ok(x) => x,
        Err(e) => return End::Failed(FailReason::Io(e.to_string())),
    };
    let Some(chunk) = chunk_for(stamp.size, inner.config.chunk) else {
        return End::Failed(FailReason::Io("file too large".into()));
    };
    inner.set_out(&id, |o| o.info.size = stamp.size);
    let offer = Offer {
        id,
        path: rel.to_owned(),
        size: stamp.size,
        chunk,
        streams: inner.config.streams,
        modified: stamp.modified,
    };
    let mut main = match session.open_files().await {
        Ok(s) => s,
        Err(e) => return end_of_session(e),
    };
    let accepted = async {
        write_raw(
            &mut main.send,
            Channel::Files,
            &ToReceiver::Offer(offer.clone()).encode(),
        )
        .await?;
        loop {
            match main.recv.next_raw().await? {
                None => {
                    return Err(SessionError::Protocol(
                        ErrorCode::MalformedFrame,
                        "files stream ended before Accept",
                    ))
                }
                Some(raw) => {
                    match ToSender::decode(raw).map_err(|e| SessionError::Recv(e.into()))? {
                        Some(ToSender::Accept { have }) => return Ok(have),
                        None => continue,
                        Some(_) => {
                            return Err(SessionError::Recv(RecvError::Frame(
                                FrameError::Malformed("expected Accept"),
                            )))
                        }
                    }
                }
            }
        }
    };
    let have = tokio::select! {
        r = accepted => match r {
            Ok(have) => have,
            Err(e) => {
                let end = end_of_session(e);
                abort(&mut main, code_of(&end));
                return end;
            }
        },
        _ = until_true(&mut cancel) => {
            abort(&mut main, ErrorCode::Cancelled);
            return End::Cancelled { by_peer: false };
        }
    };
    let chunks = offer.chunks();
    let missing: VecDeque<u64> = (0..chunks).filter(|i| !wire::has(&have, *i)).collect();
    let have_bytes: u64 = (0..chunks)
        .filter(|i| wire::has(&have, *i))
        .map(|i| offer.chunk_len(i) as u64)
        .sum();
    inner.set_out(&id, |o| o.info.done = have_bytes);
    inner.publish(false);
    let extra = (inner.config.streams - 1).min((missing.len() as u64).saturating_sub(1));
    let shared = Arc::new(SendShared {
        work: Mutex::new(Work {
            left: missing.len() as u64,
            queue: missing,
            in_flight: HashSet::new(),
            bad: HashMap::new(),
        }),
        changed: Notify::new(),
        file: Arc::new(file),
        source: source.to_path_buf(),
        stamp,
        offer,
        end: watch::channel(None).0,
    });
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(send_stream(inner.clone(), shared.clone(), main, true));
    for n in 1..=extra {
        let (inner, shared, session) = (inner.clone(), shared.clone(), session.clone());
        tasks.spawn(async move {
            let mut stream = match session.open_files().await {
                Ok(s) => s,
                Err(e) => {
                    shared.finish(end_of_session(e));
                    return;
                }
            };
            let join = ToReceiver::Join { id, stream: n }.encode();
            if let Err(e) = write_raw(&mut stream.send, Channel::Files, &join).await {
                shared.finish(end_of_session(e));
                abort(&mut stream, ErrorCode::Cancelled);
                return;
            }
            send_stream(inner, shared, stream, false).await;
        });
    }
    let mut ended = shared.end.subscribe();
    let end = tokio::select! {
        e = until_set(&mut ended) => e.unwrap_or(End::Lost),
        _ = until_true(&mut cancel) => {
            shared.finish(End::Cancelled { by_peer: false });
            End::Cancelled { by_peer: false }
        }
    };
    // Every stream task ends on `end` (resetting its stream unless done).
    shared.changed.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    tasks.abort_all();
    end
}

/// One stream of an outgoing transfer: sends chunks from the shared queue
/// and reads what comes back.
async fn send_stream(inner: Arc<Inner>, shared: Arc<SendShared>, stream: FileStream, main: bool) {
    let FileStream { mut send, mut recv } = stream;
    let mut ended = shared.end.subscribe();
    let reader = async {
        loop {
            let raw = match recv.next_raw().await {
                Ok(Some(raw)) => raw,
                Ok(None) => {
                    return if main {
                        End::Failed(FailReason::Protocol("ended before Complete".into()))
                    } else {
                        End::Done
                    }
                }
                Err(e) => {
                    let code = frame_code(&e);
                    let end = end_of_recv(e);
                    if matches!(end, End::Failed(FailReason::Protocol(_))) {
                        recv.stop(code);
                    }
                    return end;
                }
            };
            let msg = match ToSender::decode(raw) {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(e) => {
                    recv.stop(e.code());
                    return End::Failed(FailReason::Protocol(format!("{e:?}")));
                }
            };
            match msg {
                ToSender::Ack { index } => {
                    let fresh = {
                        let mut w = shared.work.lock().unwrap_or_else(|e| e.into_inner());
                        let fresh = w.in_flight.remove(&index);
                        if fresh {
                            w.left -= 1;
                        }
                        fresh
                    };
                    if fresh {
                        let len = shared.offer.chunk_len(index) as u64;
                        inner.set_out(&shared.offer.id, |o| o.info.done += len);
                        inner.publish(false);
                        shared.changed.notify_waiters();
                    }
                }
                ToSender::Bad { index } => {
                    let mut w = shared.work.lock().unwrap_or_else(|e| e.into_inner());
                    if w.in_flight.remove(&index) {
                        let n = w.bad.entry(index).or_insert(0);
                        *n += 1;
                        if *n > MAX_BAD {
                            return End::Failed(FailReason::Protocol(format!(
                                "chunk {index} failed its checksum {MAX_BAD} times"
                            )));
                        }
                        w.queue.push_front(index);
                        drop(w);
                        shared.changed.notify_waiters();
                    }
                }
                ToSender::Complete if main => return End::Done,
                ToSender::Complete | ToSender::Accept { .. } => {
                    recv.stop(ErrorCode::MalformedFrame);
                    return End::Failed(FailReason::Protocol("unexpected reply".into()));
                }
            }
        }
    };
    let writer = async {
        loop {
            let notified = shared.changed.notified();
            let next = {
                let mut w = shared.work.lock().unwrap_or_else(|e| e.into_inner());
                let next = w.queue.pop_front();
                if let Some(i) = next {
                    w.in_flight.insert(i);
                }
                next.map(|i| (i, w.queue.is_empty()))
            };
            let Some((index, last)) = next else {
                // Nothing to send: wait for a Bad, or for the end.
                notified.await;
                continue;
            };
            let shared2 = shared.clone();
            let read = blocking(move || {
                let s = &shared2;
                if last || index.is_multiple_of(64) {
                    let now = stamp_of(&std::fs::metadata(&s.source)?);
                    if now != s.stamp {
                        return Ok(None);
                    }
                }
                let len = s.offer.chunk_len(index);
                let mut data = vec![0u8; len];
                store::read_at(&s.file, &mut data, index * u64::from(s.offer.chunk))?;
                let sha256: [u8; 32] = Sha256::digest(&data).into();
                io::Result::Ok(Some(ToReceiver::Chunk {
                    index,
                    data,
                    sha256,
                }))
            })
            .await;
            let msg = match read {
                Ok(Some(m)) => m,
                Ok(None) => return End::Failed(FailReason::SourceChanged),
                Err(e) => return End::Failed(FailReason::Io(e.to_string())),
            };
            if let Err(e) = write_raw(&mut send, Channel::Files, &msg.encode()).await {
                return end_of_session(e);
            }
        }
    };
    let outcome = tokio::select! {
        e = reader => Some(e),
        e = writer => Some(e),
        _ = until_set(&mut ended) => None,
    };
    if let Some(end) = outcome {
        if end == End::Done && !main {
            // The receiver finished this stream (after Complete).
            let _ = send.finish();
            return;
        }
        shared.finish(end);
    }
    let end = shared.end.borrow().clone();
    match end {
        Some(End::Done) => {
            let _ = send.finish();
        }
        Some(End::Lost) | None => {}
        Some(other) => {
            let code = code_of(&other);
            let _ = send.reset(varint_code(code));
            recv.stop(code);
        }
    }
}

// -------------------------------------------------------------- receiving

/// An incoming transfer while its streams run.
struct InRun {
    peer: [u8; 32],
    offer: Offer,
    partial: Arc<Partial>,
    /// Set to end every stream: cancel, failure, or a newer `Offer`.
    cancel: watch::Sender<Option<End>>,
    /// The final path once complete, or why finishing failed.
    complete: watch::Sender<Option<Result<PathBuf, String>>>,
    finishing: AtomicBool,
    /// Further stream numbers joined so far.
    joined: Mutex<HashSet<u64>>,
    /// Chunks written since the last flush.
    unflushed: AtomicU64,
    /// Streams still running.
    streams: AtomicUsize,
}

impl InRun {
    fn key(&self) -> ([u8; 32], TransferId) {
        (self.peer, self.offer.id)
    }

    fn end(&self, end: End) {
        self.cancel.send_if_modified(|e| {
            if e.is_none() {
                *e = Some(end);
                true
            } else {
                false
            }
        });
    }
}

async fn receive_stream(inner: Arc<Inner>, peer: [u8; 32], mut stream: FileStream) {
    let first = tokio::time::timeout(FIRST_MESSAGE_TIMEOUT, async {
        loop {
            match stream.recv.next_raw().await {
                Ok(Some(raw)) => match ToReceiver::decode(raw) {
                    Ok(Some(m)) => return Ok(m),
                    Ok(None) => continue,
                    Err(e) => return Err(e.code()),
                },
                Ok(None) => return Err(ErrorCode::MalformedFrame),
                Err(e) => return Err(frame_code(&e)),
            }
        }
    })
    .await;
    match first {
        Err(_) => abort(&mut stream, ErrorCode::Cancelled),
        Ok(Err(code)) => abort(&mut stream, code),
        Ok(Ok(ToReceiver::Offer(offer))) => receive_offer(inner, peer, offer, stream).await,
        Ok(Ok(ToReceiver::Join { id, stream: n })) => {
            receive_join(inner, peer, id, n, stream).await
        }
        Ok(Ok(ToReceiver::Chunk { .. })) => abort(&mut stream, ErrorCode::MalformedFrame),
    }
}

async fn receive_offer(inner: Arc<Inner>, peer: [u8; 32], offer: Offer, mut stream: FileStream) {
    let key = (peer, offer.id);
    let Ok(parts) = path::check(&offer.path) else {
        abort(&mut stream, ErrorCode::MalformedFrame);
        return;
    };
    let parts: Vec<String> = parts.into_iter().map(str::to_owned).collect();
    // Cancelled by the user here: stays refused (a sender's own cancel
    // and retry is a new start).
    let (previous, running) = {
        let book = inner.lock();
        let running = book
            .inc
            .values()
            .filter(|i| i.run.is_some() && i.info.state == TransferState::Running)
            .count();
        (
            book.inc
                .get(&key)
                .map(|i| (i.info.state.clone(), i.run.clone())),
            running,
        )
    };
    if let Some((TransferState::Cancelled { by_peer: false }, _)) = previous {
        abort(&mut stream, ErrorCode::Cancelled);
        return;
    }
    if let Some((TransferState::Done, _)) = previous {
        // Already complete: say so again.
        let _ = write_raw(
            &mut stream.send,
            Channel::Files,
            &ToSender::Accept { have: full(&offer) }.encode(),
        )
        .await;
        let _ = write_raw(
            &mut stream.send,
            Channel::Files,
            &ToSender::Complete.encode(),
        )
        .await;
        let _ = stream.send.finish();
        return;
    }
    if let Some((_, Some(old))) = &previous {
        // A newer Offer (the sender resumed): the old streams stop quietly.
        old.end(End::Lost);
    } else if running >= MAX_INCOMING {
        abort(&mut stream, ErrorCode::RateLimited);
        return;
    }
    let opened = {
        let (root, offer) = (inner.config.partials.clone(), offer.clone());
        blocking(move || Partial::open(&root, &peer, &offer)).await
    };
    let (partial, have) = match opened {
        Ok(x) => x,
        Err(e) => {
            abort(&mut stream, ErrorCode::InternalError);
            record_in(
                &inner,
                &offer,
                peer,
                TransferState::Failed(FailReason::Io(e.to_string())),
                0,
                None,
            );
            return;
        }
    };
    let chunks = offer.chunks();
    let have_bytes: u64 = (0..chunks)
        .filter(|i| wire::has(&have, *i))
        .map(|i| offer.chunk_len(i) as u64)
        .sum();
    let need = offer.size - have_bytes + inner.config.space_margin;
    let free = {
        let inbox = inner.config.inbox.clone();
        blocking(move || store::free_space(&inbox)).await
    };
    if free.map(|f| f < need).unwrap_or(false) {
        abort(&mut stream, ErrorCode::NoSpace);
        if have.is_empty() {
            partial.discard();
        }
        record_in(
            &inner,
            &offer,
            peer,
            TransferState::Failed(FailReason::NoSpace),
            have_bytes,
            None,
        );
        return;
    }
    let run = Arc::new(InRun {
        peer,
        offer: offer.clone(),
        partial: Arc::new(partial),
        cancel: watch::channel(None).0,
        complete: watch::channel(None).0,
        finishing: AtomicBool::new(false),
        joined: Mutex::new(HashSet::new()),
        unflushed: AtomicU64::new(0),
        streams: AtomicUsize::new(1),
    });
    record_in(
        &inner,
        &offer,
        peer,
        TransferState::Running,
        have_bytes,
        Some(run.clone()),
    );
    inner.offers.notify_waiters();
    if let Err(e) = write_raw(
        &mut stream.send,
        Channel::Files,
        &ToSender::Accept { have }.encode(),
    )
    .await
    {
        run.end(end_of_session(e));
    } else if run.partial.written() == chunks && !run.finishing.swap(true, Ordering::SeqCst) {
        tokio::spawn(finish_file(inner.clone(), run.clone(), parts.clone()));
    }
    receive_chunks(inner, run, stream, true, parts).await;
}

fn full(offer: &Offer) -> Vec<u8> {
    let mut b = Vec::new();
    for i in 0..offer.chunks() {
        wire::set(&mut b, i);
    }
    b
}

fn record_in(
    inner: &Inner,
    offer: &Offer,
    peer: [u8; 32],
    state: TransferState,
    done: u64,
    run: Option<Arc<InRun>>,
) {
    let key = (peer, offer.id);
    {
        let mut book = inner.lock();
        if !book.inc.contains_key(&key) {
            book.order.push((Direction::Incoming, peer, offer.id));
        }
        book.inc.insert(
            key,
            In {
                info: TransferInfo {
                    id: offer.id,
                    peer,
                    direction: Direction::Incoming,
                    path: offer.path.clone(),
                    size: offer.size,
                    done,
                    state,
                    source: None,
                    saved_as: None,
                },
                run,
            },
        );
    }
    inner.publish(true);
}

async fn receive_join(
    inner: Arc<Inner>,
    peer: [u8; 32],
    id: TransferId,
    n: u64,
    mut stream: FileStream,
) {
    if inner.pending_joins.fetch_add(1, Ordering::SeqCst) >= MAX_PENDING_JOINS {
        inner.pending_joins.fetch_sub(1, Ordering::SeqCst);
        abort(&mut stream, ErrorCode::RateLimited);
        return;
    }
    let find = || {
        inner
            .lock()
            .inc
            .get(&(peer, id))
            .and_then(|i| i.run.clone())
            .filter(|r| r.cancel.borrow().is_none())
    };
    let run = tokio::time::timeout(JOIN_TIMEOUT, async {
        loop {
            let notified = inner.offers.notified();
            if let Some(run) = find() {
                return run;
            }
            notified.await;
        }
    })
    .await;
    inner.pending_joins.fetch_sub(1, Ordering::SeqCst);
    let Ok(run) = run else {
        abort(&mut stream, ErrorCode::Cancelled);
        return;
    };
    if n >= run.offer.streams
        || !run
            .joined
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(n)
    {
        abort(&mut stream, ErrorCode::MalformedFrame);
        return;
    }
    run.streams.fetch_add(1, Ordering::SeqCst);
    let parts = path::check(&run.offer.path)
        .map(|p| p.into_iter().map(str::to_owned).collect())
        .unwrap_or_default();
    receive_chunks(inner, run, stream, false, parts).await;
}

/// Reads chunks on one stream of an incoming transfer until the transfer
/// ends.
async fn receive_chunks(
    inner: Arc<Inner>,
    run: Arc<InRun>,
    mut stream: FileStream,
    main: bool,
    parts: Vec<String>,
) {
    let mut cancel = run.cancel.subscribe();
    let mut complete = run.complete.subscribe();
    let mut told = false;
    let end: Option<End> = loop {
        // Biased: a finished file is told before anything ends the stream.
        tokio::select! {
            biased;
            c = until_set(&mut complete), if !told => {
                let result = c.unwrap_or(Err("closed".into()));
                told = true;
                match result {
                    Ok(_) => {
                        if main {
                            if let Err(e) = write_raw(&mut stream.send, Channel::Files, &ToSender::Complete.encode()).await {
                                break Some(end_of_session(e));
                            }
                        }
                        let _ = stream.send.finish();
                    }
                    Err(why) => break Some(End::Failed(FailReason::Io(why))),
                }
            }
            _ = until_set(&mut cancel) => break None,
            raw = stream.recv.next_raw() => {
                let raw = match raw {
                    Ok(Some(raw)) => raw,
                    // The sender finished this stream (after Complete).
                    Ok(None) => break if told { Some(End::Done) } else { None },
                    Err(e) => {
                        let code = frame_code(&e);
                        let end = end_of_recv(e);
                        if matches!(end, End::Failed(FailReason::Protocol(_))) {
                            stream.recv.stop(code);
                        }
                        break Some(end);
                    }
                };
                let msg = match ToReceiver::decode(raw) {
                    Ok(Some(m)) => m,
                    Ok(None) => continue,
                    Err(e) => break Some(End::Failed(FailReason::Protocol(format!("{e:?}")))),
                };
                let ToReceiver::Chunk { index, data, sha256 } = msg else {
                    break Some(End::Failed(FailReason::Protocol("Offer or Join mid-stream".into())));
                };
                if index >= run.offer.chunks() || data.len() != run.offer.chunk_len(index) {
                    break Some(End::Failed(FailReason::Protocol(format!("chunk {index} out of range"))));
                }
                let partial = run.partial.clone();
                let offset = index * u64::from(run.offer.chunk);
                let len = data.len() as u64;
                let written = blocking(move || {
                    let digest: [u8; 32] = Sha256::digest(&data).into();
                    if digest != sha256 {
                        return Ok(None);
                    }
                    partial.write_chunk(index, offset, &data).map(Some)
                })
                .await;
                let reply = match written {
                    Ok(None) => ToSender::Bad { index },
                    Ok(Some(fresh)) => {
                        if fresh {
                            inner.set_in(&run.key(), |i| i.info.done += len);
                            inner.publish(false);
                            if run.partial.written() == run.offer.chunks() {
                                if !run.finishing.swap(true, Ordering::SeqCst) {
                                    tokio::spawn(finish_file(inner.clone(), run.clone(), parts.clone()));
                                }
                            } else if run.unflushed.fetch_add(1, Ordering::SeqCst) + 1 >= FLUSH_EVERY {
                                run.unflushed.store(0, Ordering::SeqCst);
                                let partial = run.partial.clone();
                                // A failed flush only costs a re-send after a crash.
                                let _ = blocking(move || partial.flush()).await;
                            }
                        }
                        ToSender::Ack { index }
                    }
                    Err(e) => break Some(End::Failed(FailReason::Io(e.to_string()))),
                };
                if let Err(e) = write_raw(&mut stream.send, Channel::Files, &reply.encode()).await {
                    break Some(end_of_session(e));
                }
            }
        }
    };
    // One stream done ends only itself; anything else ends the transfer.
    if let Some(end) = end.filter(|e| *e != End::Done) {
        run.end(end);
    }
    let end = run.cancel.borrow().clone();
    match &end {
        None | Some(End::Done) | Some(End::Lost) => {}
        Some(other) => abort(&mut stream, code_of(other)),
    }
    if run.streams.fetch_sub(1, Ordering::SeqCst) != 1 {
        return;
    }
    // The last stream of this run decides what is left.
    let key = run.key();
    let superseded = inner
        .lock()
        .inc
        .get(&key)
        .and_then(|i| i.run.as_ref())
        .is_some_and(|r| !Arc::ptr_eq(r, &run));
    if superseded {
        return;
    }
    let done = run.complete.borrow().clone();
    let state = match (done, end) {
        (Some(Ok(path)), _) => {
            inner.set_in(&key, |i| i.info.saved_as = Some(path));
            TransferState::Done
        }
        (_, Some(End::Cancelled { by_peer })) => {
            let partial = run.partial.clone();
            blocking(move || partial.discard()).await;
            TransferState::Cancelled { by_peer }
        }
        (_, Some(End::Failed(why))) => {
            let partial = run.partial.clone();
            blocking(move || partial.discard()).await;
            TransferState::Failed(why)
        }
        _ => {
            // Lost: keep what is written for the resume.
            let partial = run.partial.clone();
            let _ = blocking(move || partial.flush()).await;
            TransferState::Waiting
        }
    };
    inner.set_in(&key, |i| {
        i.info.state = state;
        i.run = None;
    });
    inner.publish(true);
}

/// Every chunk is in: make it durable, move it to its name, tell the
/// streams.
async fn finish_file(inner: Arc<Inner>, run: Arc<InRun>, parts: Vec<String>) {
    let partial = run.partial.clone();
    let inbox = inner.config.inbox.clone();
    let result = blocking(move || {
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        partial.finish(&inbox, &parts)
    })
    .await
    .map_err(|e| e.to_string());
    run.complete.send_replace(Some(result));
}
