//! CCS/1 M:clipboard/1 text, clipboard/2 PNG images (WN-36) | RULE:bounded, explicit OS-applied ack, no content logging/persistence.
use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session},
};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

pub const MAX_TEXT: usize = 256 * 1024;
/// Hard cap of an image's PNG (§9.3); the user's limit is lower.
pub const MAX_IMAGE: u64 = 64 * 1024 * 1024;
/// Largest thumbnail PNG carried in `ImageSet` (long side ≤ 512 px).
pub const MAX_THUMBNAIL: usize = 128 * 1024;
/// Largest `ImageChunk` payload.
pub const CHUNK: usize = 256 * 1024;
/// Largest image side in pixels.
pub const MAX_SIDE: u64 = 16_384;
/// Default image limit: 16 MB (decimal, as the setting shows it).
pub const DEFAULT_IMAGE_LIMIT: u64 = 16_000_000;
/// A reader of a received image gives up after this long without new bytes.
pub const STALL: Duration = Duration::from_secs(10);
/// How long the sender waits for the receiver's `ImageDone` after the last chunk.
pub const DONE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a sender waits for `Applied`.
const ACK_TIMEOUT: Duration = Duration::from_secs(3);
pub type Id = [u8; 16];
type Key = (u64, Id);
type Pending = Arc<Mutex<HashMap<Id, oneshot::Sender<(Status, Option<u64>)>>>>;
type DonePending = Arc<Mutex<HashMap<Id, oneshot::Sender<bool>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Applied,
    Refused,
    Failed,
    Sensitive,
    Unchanged,
    Disabled,
    Unavailable,
    Invalid,
    Busy,
    /// Over this side's own image limit; nothing was sent.
    TooLarge,
    /// The peer refused an image over its limit (bytes).
    PeerLimit(u64),
    /// The peer has no `clipboard/2` (an older app): images can't go.
    Unsupported,
    /// A newer clipboard item replaced this one while it was going.
    Replaced,
}
impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Sensitive => "sensitive",
            Self::Unchanged => "unchanged",
            Self::Disabled => "disabled",
            Self::Unavailable => "unavailable",
            Self::Invalid => "invalid",
            Self::Busy => "busy",
            Self::TooLarge => "too-large",
            Self::PeerLimit(_) => "peer-limit",
            Self::Unsupported => "unsupported",
            Self::Replaced => "replaced",
        }
    }
}

/// CCS/1 SEC:platform checks sensitivity before publish; received text always opts out of history/cloud.
pub trait Sink: Send + Sync {
    fn apply(&self, text: &str) -> bool;
    /// Puts a received image on the OS clipboard at once (pasteable), its
    /// bytes coming later through `image` (WN-36). True once the OS took it.
    /// A sink without images keeps the default: not applied.
    fn apply_image(&self, image: Arc<IncomingImage>) -> bool {
        let _ = image;
        false
    }
}

/// Why a received image's bytes can't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// A newer clipboard item replaced it, the session ended before it was
    /// complete, or its bytes were malformed.
    Failed,
    /// No new bytes for the stall time.
    Stalled,
}

struct Received {
    data: Vec<u8>,
    failed: bool,
    progress: Instant,
}

/// A received image (§9.3, WN-36): its thumbnail at once, its PNG as it
/// arrives. Memory only. A newer item fails an unfinished one, so a reader
/// never gets a mix of two images; a complete one stays readable.
pub struct IncomingImage {
    id: Id,
    size: u64,
    width: u64,
    height: u64,
    thumbnail: Vec<u8>,
    received: Mutex<Received>,
    changed: Condvar,
}
impl IncomingImage {
    pub fn new(id: Id, size: u64, width: u64, height: u64, thumbnail: Vec<u8>) -> Self {
        Self {
            id,
            size,
            width,
            height,
            thumbnail,
            received: Mutex::new(Received {
                data: Vec::new(),
                failed: false,
                progress: Instant::now(),
            }),
            changed: Condvar::new(),
        }
    }
    pub fn id(&self) -> Id {
        self.id
    }
    /// Size of the full PNG in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn width(&self) -> u64 {
        self.width
    }
    pub fn height(&self) -> u64 {
        self.height
    }
    /// The thumbnail PNG: for previews only, never pasted as the image.
    pub fn thumbnail(&self) -> &[u8] {
        &self.thumbnail
    }
    pub fn is_complete(&self) -> bool {
        let r = self.received.lock().unwrap();
        !r.failed && r.data.len() as u64 == self.size
    }
    pub fn is_failed(&self) -> bool {
        self.received.lock().unwrap().failed
    }
    /// Appends the bytes at `offset`; true when the image is complete. A gap,
    /// an overlap or bytes past `size` fail it.
    pub fn push(&self, offset: u64, bytes: &[u8]) -> Result<bool, ImageError> {
        let mut r = self.received.lock().unwrap();
        if r.failed {
            return Err(ImageError::Failed);
        }
        if offset != r.data.len() as u64 || offset + bytes.len() as u64 > self.size {
            r.failed = true;
            self.changed.notify_all();
            return Err(ImageError::Failed);
        }
        r.data.extend_from_slice(bytes);
        r.progress = Instant::now();
        self.changed.notify_all();
        Ok(r.data.len() as u64 == self.size)
    }
    /// Ends an unfinished image for every reader (a complete one stays).
    pub fn fail(&self) {
        let mut r = self.received.lock().unwrap();
        if r.data.len() as u64 != self.size {
            r.failed = true;
        }
        self.changed.notify_all();
    }
    /// Blocks until bytes past `offset` are there (up to `max`), the image
    /// is complete (empty: the end), or it failed / had no new bytes for
    /// [`STALL`].
    pub fn read_at(&self, offset: u64, max: usize) -> Result<Vec<u8>, ImageError> {
        self.read_at_with(offset, max, STALL)
    }
    pub fn read_at_with(
        &self,
        offset: u64,
        max: usize,
        stall: Duration,
    ) -> Result<Vec<u8>, ImageError> {
        let mut r = self.received.lock().unwrap();
        loop {
            if r.failed {
                return Err(ImageError::Failed);
            }
            let have = r.data.len() as u64;
            if offset < have {
                let start = offset as usize;
                let end = start.saturating_add(max).min(r.data.len());
                return Ok(r.data[start..end].to_vec());
            }
            if have == self.size {
                return Ok(Vec::new());
            }
            let since = r.progress.elapsed();
            if since >= stall {
                return Err(ImageError::Stalled);
            }
            r = self.changed.wait_timeout(r, stall - since).unwrap().0;
        }
    }
    /// The whole PNG once complete (the waits of [`read_at`](Self::read_at)).
    pub fn wait_complete(&self, stall: Duration) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        loop {
            let part = self.read_at_with(out.len() as u64, usize::MAX, stall)?;
            if part.is_empty() {
                return Ok(out);
            }
            out.extend_from_slice(&part);
        }
    }
}

/// An image to send: its PNG, a thumbnail PNG and the pixel size.
pub struct OutgoingImage {
    pub png: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub width: u64,
    pub height: u64,
}

#[derive(Default)]
struct State {
    clock: u64,
    key: Option<Key>,
    hash: Option<[u8; 32]>,
}
pub struct Clipboard {
    epoch: watch::Sender<u64>,
    state: Mutex<State>,
    enabled: AtomicBool,
    sink: Mutex<Option<Arc<dyn Sink>>>,
    image_limit: AtomicU64,
    /// The image being received now; a newer item fails it.
    current: Mutex<Option<Arc<IncomingImage>>>,
}
impl Default for Clipboard {
    fn default() -> Self {
        Self {
            epoch: watch::channel(0).0,
            state: Mutex::new(State::default()),
            enabled: AtomicBool::new(true),
            sink: Mutex::new(None),
            image_limit: AtomicU64::new(DEFAULT_IMAGE_LIMIT),
            current: Mutex::new(None),
        }
    }
}
impl Clipboard {
    pub fn set_sink(&self, sink: Arc<dyn Sink>) {
        *self.sink.lock().unwrap() = Some(sink);
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    /// The largest image PNG this side sends or takes (the user's setting).
    pub fn set_image_limit(&self, bytes: u64) {
        self.image_limit
            .store(bytes.clamp(1, MAX_IMAGE), Ordering::SeqCst);
    }
    pub fn image_limit(&self) -> u64 {
        self.image_limit.load(Ordering::SeqCst)
    }
    /// Takes clipboard/1 (text) and clipboard/2 (images).
    pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
        router.route(Channel::Clipboard, 2, 4)
    }
    /// The image being received or last received, if any.
    pub fn current_image(&self) -> Option<Arc<IncomingImage>> {
        self.current.lock().unwrap().clone()
    }
    /// A newer clipboard item (either side, text or image) ends the image
    /// still arriving; `next` becomes the current one.
    fn replace_current(&self, next: Option<Arc<IncomingImage>>) {
        let old = std::mem::replace(&mut *self.current.lock().unwrap(), next);
        if let Some(old) = old {
            old.fail();
        }
    }
    pub fn attach(
        self: &Arc<Self>,
        session: Session,
        mut input: mpsc::Receiver<ChannelMessage>,
    ) -> Arc<Connection> {
        // CCS/1 RULE:ordering belongs to an adopted session, hash memo survives reconnect; stale sessions cannot write.
        let (epoch, mut changed) = {
            let mut state = self.state.lock().unwrap();
            state.clock = 0;
            state.key = None;
            self.epoch
                .send_modify(|value| *value = value.wrapping_add(1));
            (*self.epoch.borrow(), self.epoch.subscribe())
        };
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let done: DonePending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (shared, p, d, live, s) = (
            self.clone(),
            pending.clone(),
            done.clone(),
            alive.clone(),
            session.clone(),
        );
        let worker = tokio::spawn(async move {
            loop {
                let incoming = tokio::select! { _=changed.changed()=>break,_=s.closed()=>break, incoming=input.recv()=>match incoming { Some(m)=>m,None=>break } };
                let reply = match Message::decode(&incoming.message) {
                    Some(Message::Ack { id, status, limit }) => {
                        if let Some(waiter) = p.lock().unwrap().remove(&id) {
                            let _ = waiter.send((status, limit));
                        }
                        None
                    }
                    Some(Message::Done { id, complete }) => {
                        if let Some(waiter) = d.lock().unwrap().remove(&id) {
                            let _ = waiter.send(complete);
                        }
                        None
                    }
                    Some(Message::Set { key, text }) => {
                        let (hub, live, session) = (shared.clone(), live.clone(), s.clone());
                        let status = tokio::task::spawn_blocking(move || {
                            hub.receive(epoch, &live, &session, key, Item::Text(&text))
                        })
                        .await
                        .unwrap_or((Status::Failed, None));
                        Some(Message::Ack {
                            id: key.1,
                            status: status.0,
                            limit: status.1,
                        })
                    }
                    Some(Message::ImageSet {
                        key,
                        size,
                        width,
                        height,
                        thumbnail,
                    }) => {
                        let (hub, live, session) = (shared.clone(), live.clone(), s.clone());
                        let image =
                            Arc::new(IncomingImage::new(key.1, size, width, height, thumbnail));
                        let status = tokio::task::spawn_blocking(move || {
                            hub.receive(epoch, &live, &session, key, Item::Image(image))
                        })
                        .await
                        .unwrap_or((Status::Failed, None));
                        Some(Message::Ack {
                            id: key.1,
                            status: status.0,
                            limit: status.1,
                        })
                    }
                    Some(Message::Chunk { id, offset, bytes }) => {
                        // Chunks of a replaced or refused image are dropped: its sender stops.
                        match shared.current_image().filter(|i| i.id() == id) {
                            Some(image) if !image.is_complete() => match image.push(offset, &bytes)
                            {
                                Ok(true) => Some(Message::Done { id, complete: true }),
                                Ok(false) => None,
                                Err(_) => Some(Message::Done {
                                    id,
                                    complete: false,
                                }),
                            },
                            _ => None,
                        }
                    }
                    None => None, /* CCS/1 SEC:malformed clipboard data is discarded, never written to the OS. */
                };
                if let Some(reply) = reply {
                    if s.send(Channel::Clipboard, &reply.encode()).await.is_err() {
                        break;
                    }
                }
            }
            live.store(false, Ordering::SeqCst);
            p.lock().unwrap().clear();
            d.lock().unwrap().clear();
            // An image still arriving from this session never completes.
            if let Some(image) = shared.current_image() {
                if !image.is_complete() {
                    image.fail();
                }
            }
        });
        Arc::new(Connection {
            epoch,
            shared: self.clone(),
            session,
            pending,
            done,
            alive,
            worker,
            limit: Semaphore::new(4),
        })
    }

    /// One received item through the clock and the OS (blocking).
    fn receive(
        &self,
        epoch: u64,
        live: &AtomicBool,
        session: &Session,
        key: Key,
        item: Item<'_>,
    ) -> (Status, Option<u64>) {
        // CCS/1 RULE:one serialized clock/OS commit; callbacks must never synchronously re-enter this mutex.
        let mut state = self.state.lock().unwrap();
        let version = if matches!(item, Item::Image(_)) { 2 } else { 1 };
        if !live.load(Ordering::SeqCst)
            || *self.epoch.borrow() != epoch
            || session.connection().quinn().close_reason().is_some()
            || !session
                .accepted_capabilities()
                .iter()
                .any(|c| c.name() == "clipboard" && c.version() == version)
        {
            return (Status::Refused, None);
        }
        state.clock = state.clock.max(key.0);
        if !self.enabled() || state.key.is_some_and(|last| key <= last) {
            return (Status::Refused, None);
        }
        if let Item::Image(image) = &item {
            let limit = self.image_limit();
            if image.size() > limit {
                // Not applied, so the clock's key stays: a later smaller item still wins.
                return (Status::Refused, Some(limit));
            }
        }
        let sink = self.sink.lock().unwrap().clone();
        let Some(sink) = sink else {
            return (Status::Refused, None);
        };
        // A failed write never produces Applied. The clock still advances past the attempted remote update.
        state.key = Some(key);
        match item {
            Item::Text(text) => {
                self.replace_current(None);
                if !sink.apply(text) {
                    return (Status::Failed, None);
                }
                state.hash = Some(hash(text.as_bytes()));
            }
            Item::Image(image) => {
                self.replace_current(Some(image.clone()));
                if !sink.apply_image(image.clone()) {
                    self.replace_current(None);
                    return (Status::Failed, None);
                }
                // The hash of the PNG is known only once it is complete; echo is
                // stopped by origin on each platform (its own write is skipped).
            }
        }
        (Status::Applied, None)
    }
}

enum Item<'a> {
    Text(&'a str),
    Image(Arc<IncomingImage>),
}

pub struct Connection {
    epoch: u64,
    shared: Arc<Clipboard>,
    session: Session,
    pending: Pending,
    done: DonePending,
    alive: Arc<AtomicBool>,
    worker: tokio::task::JoinHandle<()>,
    limit: Semaphore,
}
struct PendingGuard<T> {
    pending: Arc<Mutex<HashMap<Id, T>>>,
    id: Id,
}
impl<T> Drop for PendingGuard<T> {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
impl Connection {
    fn peer_takes(&self, version: u64) -> bool {
        self.session
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "clipboard" && c.version() == version)
    }
    /// The checks every send shares, then the next key (with the content's
    /// hash recorded). `Err` is the final status.
    fn next_key(
        &self,
        digest: [u8; 32],
        sensitive: bool,
        manual: bool,
        version: u64,
    ) -> Result<Key, Status> {
        let mut state = self.shared.state.lock().unwrap();
        if *self.shared.epoch.borrow() != self.epoch {
            return Err(Status::Unavailable);
        }
        if sensitive {
            state.hash = Some(digest);
            return Err(Status::Sensitive);
        }
        if !manual && !self.shared.enabled() {
            return Err(Status::Disabled);
        }
        if !manual && state.hash == Some(digest) {
            return Err(Status::Unchanged);
        }
        if !self.alive.load(Ordering::SeqCst)
            || self.session.connection().quinn().close_reason().is_some()
            || !self.peer_takes(1)
        {
            return Err(Status::Unavailable);
        }
        if !self.peer_takes(version) {
            return Err(Status::Unsupported);
        }
        state.clock = state.clock.checked_add(1).ok_or(Status::Failed)?;
        let mut id = [0; 16];
        OsRng.fill_bytes(&mut id);
        let key = (state.clock, id);
        state.key = Some(key);
        state.hash = Some(digest);
        drop(state);
        // Our own newer item ends an image still arriving from the peer.
        self.shared.replace_current(None);
        Ok(key)
    }
    /// Sends `message` (an item with `id`) and waits up to [`ACK_TIMEOUT`] for its `Applied`.
    async fn send_and_ack(&self, id: Id, message: Message) -> (Status, Option<u64>) {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let _cleanup = PendingGuard {
            pending: self.pending.clone(),
            id,
        };
        if self
            .session
            .send(Channel::Clipboard, &message.encode())
            .await
            .is_err()
        {
            return (Status::Unavailable, None);
        }
        tokio::select! {
            _=self.session.closed()=>(Status::Unavailable, None),
            result=tokio::time::timeout(ACK_TIMEOUT,rx)=>result.ok().and_then(Result::ok).unwrap_or((Status::Unavailable, None)),
        }
    }
    pub async fn send(&self, text: String, sensitive: bool, manual: bool) -> Status {
        if text.len() > MAX_TEXT || text.contains('\0') {
            return Status::Invalid;
        }
        let Ok(_permit) = self.limit.try_acquire() else {
            return Status::Busy;
        };
        let key = match self.next_key(hash(text.as_bytes()), sensitive, manual, 1) {
            Ok(key) => key,
            Err(status) => return status,
        };
        self.send_and_ack(key.1, Message::Set { key, text }).await.0
    }
    /// Sends an image (clipboard/2): `ImageSet` with the thumbnail, then the
    /// PNG in chunks. `on_applied` runs once the peer has it on its clipboard
    /// (pasteable); the result is [`Status::Applied`] only after the peer
    /// got the whole PNG (`ImageDone`).
    pub async fn send_image(
        &self,
        image: OutgoingImage,
        sensitive: bool,
        manual: bool,
        on_applied: impl FnOnce() + Send,
    ) -> Status {
        let size = image.png.len() as u64;
        if size == 0
            || size > MAX_IMAGE
            || image.thumbnail.is_empty()
            || image.thumbnail.len() > MAX_THUMBNAIL
            || !(1..=MAX_SIDE).contains(&image.width)
            || !(1..=MAX_SIDE).contains(&image.height)
        {
            return Status::Invalid;
        }
        if size > self.shared.image_limit() && !sensitive {
            return Status::TooLarge;
        }
        let Ok(_permit) = self.limit.try_acquire() else {
            return Status::Busy;
        };
        let key = match self.next_key(hash(&image.png), sensitive, manual, 2) {
            Ok(key) => key,
            Err(status) => return status,
        };
        let id = key.1;
        let (done_tx, done_rx) = oneshot::channel();
        self.done.lock().unwrap().insert(id, done_tx);
        let _done_cleanup = PendingGuard {
            pending: self.done.clone(),
            id,
        };
        let set = Message::ImageSet {
            key,
            size,
            width: image.width,
            height: image.height,
            thumbnail: image.thumbnail,
        };
        match self.send_and_ack(id, set).await {
            (Status::Applied, _) => {}
            (Status::Refused, Some(limit)) => return Status::PeerLimit(limit),
            (status, _) => return status,
        }
        on_applied();
        for (n, bytes) in image.png.chunks(CHUNK).enumerate() {
            if self.replaced(key) {
                return Status::Replaced;
            }
            let chunk = Message::Chunk {
                id,
                offset: (n * CHUNK) as u64,
                bytes: bytes.to_vec(),
            };
            if self
                .session
                .send(Channel::Clipboard, &chunk.encode())
                .await
                .is_err()
            {
                return Status::Unavailable;
            }
        }
        tokio::select! {
            _=self.session.closed()=>Status::Unavailable,
            result=tokio::time::timeout(DONE_TIMEOUT, done_rx)=>match result {
                Ok(Ok(true)) => Status::Applied,
                Ok(Ok(false)) => Status::Failed,
                _ if self.replaced(key) => Status::Replaced,
                _ => Status::Unavailable,
            },
        }
    }
    /// Whether a newer item (either side) took the clock past `key`, or the session moved on.
    fn replaced(&self, key: Key) -> bool {
        let state = self.shared.state.lock().unwrap();
        *self.shared.epoch.borrow() != self.epoch || state.key != Some(key)
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.worker.abort();
        self.pending.lock().unwrap().clear();
        self.done.lock().unwrap().clear();
    }
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

// CCS/1 IO:clipboard/1 [0,{0:id16B,1:clock,2:text}] | [1,{0:id16B,1:status(0/1/2),2?:limit}];
// clipboard/2 adds [2,{0:id,1:clock,2:"image/png",3:size,4:w,5:h,6:thumb}] | [3,{0:id,1:offset,2:bytes}] | [4,{0:id,1:0 complete/2 failed}].
#[derive(PartialEq, Eq, Debug)]
pub enum Message {
    Set {
        key: Key,
        text: String,
    },
    Ack {
        id: Id,
        status: Status,
        /// With `Refused` of an image over the receiver's limit: that limit.
        limit: Option<u64>,
    },
    ImageSet {
        key: Key,
        size: u64,
        width: u64,
        height: u64,
        thumbnail: Vec<u8>,
    },
    Chunk {
        id: Id,
        offset: u64,
        bytes: Vec<u8>,
    },
    Done {
        id: Id,
        complete: bool,
    },
}
pub const PNG: &str = "image/png";
impl Message {
    pub fn encode(&self) -> RawMessage {
        let uint = |v: u64| Value::Int(v.into());
        let bytes = |b: &[u8]| Value::Bytes(b.to_vec());
        let (ty, fields) = match self {
            Self::Set { key, text } => (
                0,
                vec![
                    (0, bytes(&key.1)),
                    (1, uint(key.0)),
                    (2, Value::Text(text.clone())),
                ],
            ),
            Self::Ack { id, status, limit } => {
                let mut fields = vec![
                    (0, bytes(id)),
                    (
                        1,
                        uint(match status {
                            Status::Applied => 0,
                            Status::Refused => 1,
                            _ => 2,
                        }),
                    ),
                ];
                if let Some(limit) = limit {
                    fields.push((2, uint(*limit)));
                }
                (1, fields)
            }
            Self::ImageSet {
                key,
                size,
                width,
                height,
                thumbnail,
            } => (
                2,
                vec![
                    (0, bytes(&key.1)),
                    (1, uint(key.0)),
                    (2, Value::Text(PNG.into())),
                    (3, uint(*size)),
                    (4, uint(*width)),
                    (5, uint(*height)),
                    (6, bytes(thumbnail)),
                ],
            ),
            Self::Chunk {
                id,
                offset,
                bytes: b,
            } => (3, vec![(0, bytes(id)), (1, uint(*offset)), (2, bytes(b))]),
            Self::Done { id, complete } => (
                4,
                vec![(0, bytes(id)), (1, uint(if *complete { 0 } else { 2 }))],
            ),
        };
        RawMessage::new(
            ty,
            Value::Map(fields.into_iter().map(|(k, v)| (uint(k), v)).collect()),
        )
    }
    pub fn decode(message: &RawMessage) -> Option<Self> {
        let Value::Map(fields) = &message.body else {
            return None;
        };
        let mut values = HashMap::new();
        for (key, value) in fields {
            let Value::Int(key) = key else { return None };
            let key = u64::try_from(*key).ok()?;
            if values.insert(key, value).is_some() {
                return None;
            }
        }
        let uint = |k: u64| match values.get(&k) {
            Some(Value::Int(n)) => u64::try_from(*n).ok(),
            _ => None,
        };
        let bytes = |k: u64| match values.get(&k) {
            Some(Value::Bytes(b)) => Some(b),
            _ => None,
        };
        let id: Id = bytes(0)?.as_slice().try_into().ok()?;
        let number = uint(1)?;
        match message.ty {
            0 => {
                let Value::Text(text) = *values.get(&2)? else {
                    return None;
                };
                if number == 0 || text.len() > MAX_TEXT || text.contains('\0') {
                    return None;
                }
                Some(Self::Set {
                    key: (number, id),
                    text: text.clone(),
                })
            }
            1 => {
                let limit = match values.get(&2) {
                    None => None,
                    Some(_) => Some(uint(2)?),
                };
                Some(Self::Ack {
                    id,
                    status: match number {
                        0 => Status::Applied,
                        1 => Status::Refused,
                        2 => Status::Failed,
                        _ => return None,
                    },
                    limit,
                })
            }
            2 => {
                let Value::Text(mime) = *values.get(&2)? else {
                    return None;
                };
                let (size, width, height) = (uint(3)?, uint(4)?, uint(5)?);
                let thumbnail = bytes(6)?;
                if number == 0
                    || mime != PNG
                    || size == 0
                    || size > MAX_IMAGE
                    || !(1..=MAX_SIDE).contains(&width)
                    || !(1..=MAX_SIDE).contains(&height)
                    || thumbnail.is_empty()
                    || thumbnail.len() > MAX_THUMBNAIL
                {
                    return None;
                }
                Some(Self::ImageSet {
                    key: (number, id),
                    size,
                    width,
                    height,
                    thumbnail: thumbnail.clone(),
                })
            }
            3 => {
                let b = bytes(2)?;
                if b.is_empty() || b.len() > CHUNK || number >= MAX_IMAGE {
                    return None;
                }
                Some(Self::Chunk {
                    id,
                    offset: number,
                    bytes: b.clone(),
                })
            }
            4 => Some(Self::Done {
                id,
                complete: match number {
                    0 => true,
                    2 => false,
                    _ => return None,
                },
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn specification_binary_vectors() {
        let mut set = vec![27, 0x82, 0, 0xa3, 0, 0x50];
        set.extend([0; 16]);
        set.extend([1, 1, 2, 0x62, 0x68, 0x69]);
        let raw = Message::Set {
            key: (1, [0; 16]),
            text: "hi".into(),
        }
        .encode();
        assert_eq!(
            crate::protocol::encode_raw_frame(Channel::Clipboard, &raw).unwrap(),
            set
        );
        let mut ack = vec![23, 0x82, 1, 0xa2, 0, 0x50];
        ack.extend([0; 16]);
        ack.extend([1, 0]);
        assert_eq!(
            crate::protocol::encode_raw_frame(
                Channel::Clipboard,
                &Message::Ack {
                    id: [0; 16],
                    status: Status::Applied,
                    limit: None,
                }
                .encode()
            )
            .unwrap(),
            ack
        );
    }
    /// The clipboard/2 examples of §9.3 (WN-36).
    #[test]
    fn image_binary_vectors() {
        let frame = |m: Message| {
            crate::protocol::encode_raw_frame(Channel::Clipboard, &m.encode()).unwrap()
        };
        // ImageSet id 0×16, clock 1, 2×1 px, size 3, thumbnail [1 2].
        let mut set = vec![0x2c, 0x82, 2, 0xa7, 0, 0x50];
        set.extend([0; 16]);
        set.extend([1, 1, 2, 0x69]);
        set.extend(b"image/png");
        set.extend([3, 3, 4, 2, 5, 1, 6, 0x42, 1, 2]);
        assert_eq!(
            frame(Message::ImageSet {
                key: (1, [0; 16]),
                size: 3,
                width: 2,
                height: 1,
                thumbnail: vec![1, 2],
            }),
            set
        );
        let mut chunk = vec![0x1c, 0x82, 3, 0xa3, 0, 0x50];
        chunk.extend([0; 16]);
        chunk.extend([1, 0, 2, 0x43, 7, 8, 9]);
        assert_eq!(
            frame(Message::Chunk {
                id: [0; 16],
                offset: 0,
                bytes: vec![7, 8, 9],
            }),
            chunk
        );
        let mut done = vec![0x17, 0x82, 4, 0xa2, 0, 0x50];
        done.extend([0; 16]);
        done.extend([1, 0]);
        assert_eq!(
            frame(Message::Done {
                id: [0; 16],
                complete: true
            }),
            done
        );
        // Refused over the receiver's limit carries it: 16 000 000.
        let mut refused = vec![0x1d, 0x82, 1, 0xa3, 0, 0x50];
        refused.extend([0; 16]);
        refused.extend([1, 1, 2, 0x1a, 0x00, 0xf4, 0x24, 0x00]);
        assert_eq!(
            frame(Message::Ack {
                id: [0; 16],
                status: Status::Refused,
                limit: Some(16_000_000),
            }),
            refused
        );
    }
    #[test]
    fn wire_bounds_and_shape() {
        for text in [
            "".to_string(),
            "Zażółć 👩‍💻\nhttps://example.com/ż".into(), // i18n: data
            "a".repeat(MAX_TEXT),
        ] {
            let m = Message::Set {
                key: (1, [3; 16]),
                text,
            };
            assert!(Message::decode(&m.encode()) == Some(m));
        }
        for text in ["a".repeat(MAX_TEXT + 1), "a\0b".into()] {
            assert!(Message::decode(
                &Message::Set {
                    key: (1, [3; 16]),
                    text
                }
                .encode()
            )
            .is_none());
        }
        assert!(Message::decode(
            &Message::Set {
                key: (0, [0; 16]),
                text: "x".into()
            }
            .encode()
        )
        .is_none());
        let mut bad = Message::Set {
            key: (1, [0; 16]),
            text: "x".into(),
        }
        .encode();
        if let Value::Map(fields) = &mut bad.body {
            fields.push(fields[0].clone());
        }
        assert!(Message::decode(&bad).is_none());
        assert!(Message::decode(&RawMessage::new(5, Value::Map(vec![]))).is_none());
    }
    #[test]
    fn image_wire_bounds() {
        let set = |size: u64, w: u64, h: u64, thumb: usize| Message::ImageSet {
            key: (1, [5; 16]),
            size,
            width: w,
            height: h,
            thumbnail: vec![1; thumb],
        };
        let ok = set(MAX_IMAGE, MAX_SIDE, 1, MAX_THUMBNAIL);
        assert_eq!(Message::decode(&ok.encode()), Some(ok));
        for bad in [
            set(0, 1, 1, 1),
            set(MAX_IMAGE + 1, 1, 1, 1),
            set(1, 0, 1, 1),
            set(1, 1, MAX_SIDE + 1, 1),
            set(1, 1, 1, 0),
            set(1, 1, 1, MAX_THUMBNAIL + 1),
        ] {
            assert_eq!(Message::decode(&bad.encode()), None, "{bad:?}");
        }
        // Only PNG in v1.
        let mut jpeg = set(1, 1, 1, 1).encode();
        if let Value::Map(fields) = &mut jpeg.body {
            fields[2].1 = Value::Text("image/jpeg".into());
        }
        assert_eq!(Message::decode(&jpeg), None);
        let chunk = |bytes: usize| Message::Chunk {
            id: [5; 16],
            offset: 0,
            bytes: vec![1; bytes],
        };
        assert!(Message::decode(&chunk(CHUNK).encode()).is_some());
        assert_eq!(Message::decode(&chunk(CHUNK + 1).encode()), None);
        assert_eq!(Message::decode(&chunk(0).encode()), None);
        let done = Message::Done {
            id: [5; 16],
            complete: false,
        };
        assert_eq!(Message::decode(&done.encode()), Some(done));
    }
    #[test]
    fn incoming_image_reads_in_order_and_fails_cleanly() {
        let image = Arc::new(IncomingImage::new([1; 16], 6, 1, 1, vec![9]));
        let reader = {
            let image = image.clone();
            std::thread::spawn(move || image.wait_complete(Duration::from_secs(5)))
        };
        assert_eq!(image.push(0, &[1, 2, 3]), Ok(false));
        assert_eq!(image.read_at(1, 10), Ok(vec![2, 3]));
        assert_eq!(image.push(3, &[4, 5, 6]), Ok(true));
        assert_eq!(reader.join().unwrap(), Ok(vec![1, 2, 3, 4, 5, 6]));
        assert_eq!(image.read_at(6, 10), Ok(vec![]), "the end");
        image.fail();
        assert!(image.is_complete(), "a complete image stays readable");

        // A gap or bytes past the size fail it; readers get an error, not a mix.
        let gap = IncomingImage::new([2; 16], 6, 1, 1, vec![9]);
        assert_eq!(gap.push(1, &[1]), Err(ImageError::Failed));
        assert_eq!(gap.read_at(0, 1), Err(ImageError::Failed));
        let over = IncomingImage::new([3; 16], 2, 1, 1, vec![9]);
        assert_eq!(over.push(0, &[1, 2, 3]), Err(ImageError::Failed));
        // Replaced while a reader waits: the reader ends with an error.
        let replaced = Arc::new(IncomingImage::new([4; 16], 4, 1, 1, vec![9]));
        replaced.push(0, &[1]).unwrap();
        let waiting = {
            let image = replaced.clone();
            std::thread::spawn(move || image.read_at(1, 4))
        };
        std::thread::sleep(Duration::from_millis(50));
        replaced.fail();
        assert_eq!(waiting.join().unwrap(), Err(ImageError::Failed));
        // No bytes for the stall time: Stalled.
        let stalled = IncomingImage::new([5; 16], 4, 1, 1, vec![9]);
        assert_eq!(
            stalled.read_at_with(0, 4, Duration::from_millis(30)),
            Err(ImageError::Stalled)
        );
    }
}
