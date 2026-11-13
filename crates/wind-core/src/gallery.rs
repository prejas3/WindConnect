//! CCS/1 M:gallery/1, the phone's newest photos and screenshots in the PC's panel (§9.13, WN-58). The PC opens one
//! bidi `gallery` stream per request: `Recent` [0,{0:limit}], `Thumb` [1,{0:id,1:size}], `Original` [2,{0:id}],
//! `Watch` [3,{}]. The phone answers `Items` [0,{0:[..]}], `Thumb` [1,{0:jpeg}], `File` [2,{0:name,1:mime,2:size}]
//! then `Chunk` [3,{0:bytes}].., `Changed` [4,{}] for a watch, then one `End` [5,{0:status}] and FIN.
//! RULE:the phone announces `gallery/1` only with its photo access granted and the person's setting on; the PC keeps
//! thumbnails in memory and an original only as the file a drag hands to Windows. Nothing here logs names.
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Semaphore};

use crate::{
    protocol::{cbor::Value, Channel, ErrorCode, RawMessage},
    session::{write_raw, GalleryStream, Session, SessionError},
};

pub use crate::sms::Status;

/// The request must come this soon after the stream opens.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// `Recent` and `Thumb` answer within this; an `Original` gets it between frames.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);
/// Most items one `Recent` asks for.
pub const MAX_RECENT: u32 = 60;
/// Thumbnail sizes (the longer side, px) a `Thumb` may ask for.
pub const THUMB_SIZES: std::ops::RangeInclusive<u32> = 64..=512;
/// Largest thumbnail, a JPEG.
pub const MAX_THUMB: usize = 64 * 1024;
/// Largest original the phone hands over.
pub const MAX_ORIGINAL: u64 = 256 * 1024 * 1024;
/// Largest `Chunk`.
pub const CHUNK: usize = 256 * 1024;
/// Longest file name and type, UTF-8 bytes.
pub const MAX_NAME: usize = 255;
pub const MAX_MIME: usize = 64;
/// Requests served at once per session (a watch doesn't count).
pub const CONCURRENT: usize = 4;
/// Watches per session; another gets `End(Busy)`.
pub const WATCHES: usize = 2;
/// Changes within this window make one `Changed` (a camera writes a photo in a few steps).
pub const CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);

/// A photo or screenshot on the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The phone's id for it (Android: the MediaStore id); stable while the photo exists.
    pub id: u64,
    /// When it was added, ms since the epoch: the order of [`recent`].
    pub date_ms: u64,
    pub name: String,
    pub mime: String,
    /// Pixels; 0 when unknown.
    pub width: u32,
    pub height: u32,
    /// Bytes of the original.
    pub size: u64,
    /// A screenshot rather than a photo.
    pub screenshot: bool,
}

/// What an original is, before its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// A single file name (`files::path::check`), safe to create on the PC.
    pub name: String,
    pub mime: String,
    /// 1 to [`MAX_ORIGINAL`] bytes, exactly the `Chunk`s that follow.
    pub size: u64,
}

/// From the PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The newest `limit` photos and screenshots, newest first.
    Recent { limit: u32 },
    /// A JPEG of photo `id`, its longer side at most `size` px.
    Thumb { id: u64, size: u32 },
    /// Photo `id` as it is on the phone: `File`, then its bytes in `Chunk`s.
    Original { id: u64 },
    /// `Changed` whenever the phone's photos change, until either side ends the stream.
    Watch,
}

/// From the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Items(Vec<Item>),
    Thumb(Vec<u8>),
    File(FileInfo),
    Chunk(Vec<u8>),
    Changed,
    End(Status),
}

/// A JPEG by its first bytes.
pub fn is_jpeg(b: &[u8]) -> bool {
    b.starts_with(&[0xff, 0xd8, 0xff])
}

/// A name the PC can create as it is (one part, no reserved or odd characters).
pub fn name_ok(name: &str) -> bool {
    name.len() <= MAX_NAME && matches!(crate::files::path::check(name), Ok(p) if p.len() == 1)
}

fn uint(n: u64) -> Value {
    Value::Int(n.into())
}

fn map(f: Vec<(u64, Value)>) -> Value {
    Value::Map(f.into_iter().map(|(k, v)| (uint(k), v)).collect())
}

type Fields<'a> = Vec<(u64, &'a Value)>;

fn fields_of(v: &Value) -> Option<Fields<'_>> {
    let Value::Map(pairs) = v else { return None };
    let mut out: Fields = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let Value::Int(k) = k else { return None };
        let k = u64::try_from(*k).ok()?;
        if out.iter().any(|(seen, _)| *seen == k) {
            return None;
        }
        out.push((k, v));
    }
    Some(out)
}

fn get<'a>(f: &Fields<'a>, key: u64) -> Option<&'a Value> {
    f.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn as_uint(v: &Value) -> Option<u64> {
    match v {
        Value::Int(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

fn get_uint(f: &Fields<'_>, key: u64) -> Option<u64> {
    as_uint(get(f, key)?)
}

fn get_u32(f: &Fields<'_>, key: u64) -> Option<u32> {
    u32::try_from(get_uint(f, key)?).ok()
}

fn get_text(f: &Fields<'_>, key: u64, max: usize) -> Option<String> {
    match get(f, key)? {
        Value::Text(t) if !t.is_empty() && t.len() <= max => Some(t.clone()),
        _ => None,
    }
}

fn get_bytes(f: &Fields<'_>, key: u64, max: usize) -> Option<Vec<u8>> {
    match get(f, key)? {
        Value::Bytes(b) if !b.is_empty() && b.len() <= max => Some(b.clone()),
        _ => None,
    }
}

fn item_value(i: &Item) -> Value {
    map(vec![
        (0, uint(i.id)),
        (1, uint(i.date_ms)),
        (2, Value::Text(i.name.clone())),
        (3, Value::Text(i.mime.clone())),
        (4, uint(i.width.into())),
        (5, uint(i.height.into())),
        (6, uint(i.size)),
        (7, Value::Bool(i.screenshot)),
    ])
}

fn decode_item(v: &Value) -> Option<Item> {
    let f = fields_of(v)?;
    Some(Item {
        id: get_uint(&f, 0)?,
        date_ms: get_uint(&f, 1)?,
        name: get_text(&f, 2, MAX_NAME)?,
        mime: get_text(&f, 3, MAX_MIME)?,
        width: get_u32(&f, 4)?,
        height: get_u32(&f, 5)?,
        size: get_uint(&f, 6)?,
        screenshot: match get(&f, 7)? {
            Value::Bool(b) => *b,
            _ => return None,
        },
    })
}

impl Request {
    pub fn encode(&self) -> RawMessage {
        let (ty, f) = match self {
            Self::Recent { limit } => (0, vec![(0, uint((*limit).into()))]),
            Self::Thumb { id, size } => (1, vec![(0, uint(*id)), (1, uint((*size).into()))]),
            Self::Original { id } => (2, vec![(0, uint(*id))]),
            Self::Watch => (3, vec![]),
        };
        RawMessage::new(ty, map(f))
    }

    /// `None` for an unknown type or a malformed message (a limit or size out of range): never clamped.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields_of(&m.body)?;
        match m.ty {
            0 => Some(Self::Recent {
                limit: get_u32(&f, 0).filter(|l| (1..=MAX_RECENT).contains(l))?,
            }),
            1 => Some(Self::Thumb {
                id: get_uint(&f, 0)?,
                size: get_u32(&f, 1).filter(|s| THUMB_SIZES.contains(s))?,
            }),
            2 => Some(Self::Original {
                id: get_uint(&f, 0)?,
            }),
            3 => Some(Self::Watch),
            _ => None,
        }
    }
}

impl Reply {
    pub fn encode(&self) -> RawMessage {
        let (ty, f) = match self {
            Self::Items(items) => (
                0,
                vec![(0, Value::Array(items.iter().map(item_value).collect()))],
            ),
            Self::Thumb(jpeg) => (1, vec![(0, Value::Bytes(jpeg.clone()))]),
            Self::File(info) => (
                2,
                vec![
                    (0, Value::Text(info.name.clone())),
                    (1, Value::Text(info.mime.clone())),
                    (2, uint(info.size)),
                ],
            ),
            Self::Chunk(bytes) => (3, vec![(0, Value::Bytes(bytes.clone()))]),
            Self::Changed => (4, vec![]),
            Self::End(status) => (5, vec![(0, uint(status.code()))]),
        };
        RawMessage::new(ty, map(f))
    }

    /// `None` for an unknown type or anything over the limits (never trimmed): more than [`MAX_RECENT`] items, a
    /// thumbnail that isn't a JPEG, a file name the PC couldn't create as it is.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields_of(&m.body)?;
        match m.ty {
            0 => {
                let Value::Array(list) = get(&f, 0)? else {
                    return None;
                };
                if list.len() > MAX_RECENT as usize {
                    return None;
                }
                Some(Self::Items(
                    list.iter().map(decode_item).collect::<Option<_>>()?,
                ))
            }
            1 => Some(Self::Thumb(
                get_bytes(&f, 0, MAX_THUMB).filter(|b| is_jpeg(b))?,
            )),
            2 => {
                let name = get_text(&f, 0, MAX_NAME).filter(|n| name_ok(n))?;
                let mime = get_text(&f, 1, MAX_MIME)?;
                let size = get_uint(&f, 2).filter(|s| (1..=MAX_ORIGINAL).contains(s))?;
                Some(Self::File(FileInfo { name, mime, size }))
            }
            3 => Some(Self::Chunk(get_bytes(&f, 0, CHUNK)?)),
            4 => Some(Self::Changed),
            5 => Some(Self::End(Status::from_code(get_uint(&f, 0)?))),
            _ => None,
        }
    }
}

/// An original opened on the phone: what it is and its bytes.
pub struct Opened {
    pub info: FileInfo,
    pub bytes: Box<dyn Read + Send>,
}

/// The phone's photos. Called off the async runtime; may block on the content provider.
pub trait Source: Send + Sync {
    /// The newest `limit` photos and screenshots, newest first.
    fn recent(&self, limit: u32) -> Result<Vec<Item>, Status>;
    /// A JPEG of at most [`MAX_THUMB`] bytes, its longer side at most `size` px.
    fn thumb(&self, id: u64, size: u32) -> Result<Vec<u8>, Status>;
    /// Opens photo `id`; `info.size` must be what `bytes` gives. The name is fitted by the server.
    fn original(&self, id: u64) -> Result<Opened, Status>;
}

/// `name` as the PC will create it, else `IMG_<id>` with an extension from the type.
fn fit_name(name: &str, id: u64, mime: &str) -> String {
    let name = crate::sms::clip(name.trim(), MAX_NAME);
    if name_ok(&name) {
        return name;
    }
    let ext = match mime {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/heic" | "image/heif" => "heic",
        "image/gif" => "gif",
        _ => "jpg",
    };
    format!("IMG_{id}.{ext}")
}

fn fit_item(mut i: Item) -> Item {
    i.name = crate::sms::clip(&i.name, MAX_NAME);
    if i.name.is_empty() {
        i.name = format!("IMG_{}", i.id);
    }
    i.mime = crate::sms::clip(&i.mime, MAX_MIME);
    if i.mime.is_empty() {
        i.mime = "image/jpeg".into();
    }
    i
}

/// Serves `gallery` streams on the phone.
pub struct Server {
    source: Arc<dyn Source>,
    changes: watch::Sender<u64>,
}

impl Server {
    pub fn new(source: Arc<dyn Source>) -> Arc<Self> {
        Arc::new(Self {
            source,
            changes: watch::channel(0).0,
        })
    }

    /// The phone's photos changed (a content observer): every watch sends `Changed` after [`CHANGE_DEBOUNCE`].
    pub fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Serves the streams of one session until it ends.
    pub async fn serve(self: Arc<Self>, mut streams: mpsc::Receiver<GalleryStream>) {
        let requests = Arc::new(Semaphore::new(CONCURRENT));
        let watches = Arc::new(Semaphore::new(WATCHES));
        while let Some(stream) = streams.recv().await {
            tokio::spawn(
                self.clone()
                    .handle(stream, requests.clone(), watches.clone()),
            );
        }
    }

    async fn handle(
        self: Arc<Self>,
        mut stream: GalleryStream,
        requests: Arc<Semaphore>,
        watches: Arc<Semaphore>,
    ) {
        let request = match tokio::time::timeout(REQUEST_TIMEOUT, stream.recv.next_raw()).await {
            Ok(Ok(Some(m))) => match Request::decode(&m) {
                Some(r) => r,
                None => {
                    let code = if m.ty <= 3 {
                        ErrorCode::MalformedFrame
                    } else {
                        ErrorCode::UnknownMessage
                    };
                    return abort(&mut stream, code);
                }
            },
            Ok(Err(_)) => return abort(&mut stream, ErrorCode::MalformedFrame),
            Ok(Ok(None)) => return,
            Err(_) => return abort(&mut stream, ErrorCode::Cancelled),
        };
        if request == Request::Watch {
            let Ok(_permit) = watches.try_acquire_owned() else {
                return end(&mut stream, Status::Busy).await;
            };
            return self.watch(stream).await;
        }
        let Ok(_permit) = requests.acquire_owned().await else {
            return;
        };
        let source = self.source.clone();
        let status = match request {
            Request::Recent { limit } => {
                let answer = tokio::task::spawn_blocking(move || source.recent(limit))
                    .await
                    .unwrap_or(Err(Status::Failed));
                match answer {
                    Ok(items) => {
                        let items = items
                            .into_iter()
                            .take(limit as usize)
                            .map(fit_item)
                            .collect();
                        if write_raw(
                            &mut stream.send,
                            Channel::Gallery,
                            &Reply::Items(items).encode(),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                        Status::Ok
                    }
                    Err(s) => s,
                }
            }
            Request::Thumb { id, size } => {
                let answer = tokio::task::spawn_blocking(move || source.thumb(id, size))
                    .await
                    .unwrap_or(Err(Status::Failed))
                    .and_then(|jpeg| {
                        if jpeg.is_empty() || jpeg.len() > MAX_THUMB || !is_jpeg(&jpeg) {
                            Err(Status::Failed)
                        } else {
                            Ok(jpeg)
                        }
                    });
                match answer {
                    Ok(jpeg) => {
                        if write_raw(
                            &mut stream.send,
                            Channel::Gallery,
                            &Reply::Thumb(jpeg).encode(),
                        )
                        .await
                        .is_err()
                        {
                            return;
                        }
                        Status::Ok
                    }
                    Err(s) => s,
                }
            }
            Request::Original { id } => match self.original(&mut stream, id).await {
                Some(s) => s,
                None => return,
            },
            Request::Watch => unreachable!("handled above"),
        };
        end(&mut stream, status).await;
    }

    /// Streams an original; `None` when the stream is gone or was reset (the PC sees no `End`).
    async fn original(&self, stream: &mut GalleryStream, id: u64) -> Option<Status> {
        let source = self.source.clone();
        let opened = tokio::task::spawn_blocking(move || source.original(id))
            .await
            .unwrap_or(Err(Status::Failed))
            .and_then(|o| {
                if o.info.size == 0 || o.info.size > MAX_ORIGINAL {
                    Err(Status::Failed)
                } else {
                    Ok(o)
                }
            });
        let Opened { mut info, bytes } = match opened {
            Ok(o) => o,
            Err(s) => return Some(s),
        };
        info.name = fit_name(&info.name, id, &info.mime);
        info.mime = crate::sms::clip(&info.mime, MAX_MIME);
        if info.mime.is_empty() {
            info.mime = "application/octet-stream".into();
        }
        let size = info.size;
        write_raw(
            &mut stream.send,
            Channel::Gallery,
            &Reply::File(info).encode(),
        )
        .await
        .ok()?;
        // The reader runs on a blocking thread; a small queue keeps reading just ahead of the network.
        let (tx, mut rx) = mpsc::channel::<std::io::Result<Vec<u8>>>(4);
        let reader = tokio::task::spawn_blocking(move || {
            let mut bytes = bytes;
            let mut left = size;
            while left > 0 {
                let mut buf = vec![0u8; (left as usize).min(CHUNK)];
                let n = match bytes.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.blocking_send(Err(std::io::ErrorKind::UnexpectedEof.into()));
                        return;
                    }
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        return;
                    }
                };
                buf.truncate(n);
                left -= n as u64;
                if tx.blocking_send(Ok(buf)).is_err() {
                    return;
                }
            }
        });
        let mut sent = 0u64;
        while sent < size {
            match rx.recv().await {
                Some(Ok(buf)) => {
                    sent += buf.len() as u64;
                    if write_raw(
                        &mut stream.send,
                        Channel::Gallery,
                        &Reply::Chunk(buf).encode(),
                    )
                    .await
                    .is_err()
                    {
                        return None;
                    }
                }
                // The file changed or failed under us: the PC must not keep a short file.
                _ => {
                    abort(stream, ErrorCode::Cancelled);
                    return None;
                }
            }
        }
        drop(rx);
        let _ = reader.await;
        Some(Status::Ok)
    }

    async fn watch(&self, mut stream: GalleryStream) {
        let mut changes = self.changes.subscribe();
        changes.mark_unchanged();
        loop {
            tokio::select! {
                m = stream.recv.next_raw() => match m {
                    // Nothing more is expected; FIN or a broken stream ends the watch.
                    Ok(Some(_)) => {}
                    _ => break,
                },
                r = changes.changed() => {
                    if r.is_err() { break; }
                    tokio::time::sleep(CHANGE_DEBOUNCE).await;
                    changes.mark_unchanged();
                    if write_raw(&mut stream.send, Channel::Gallery, &Reply::Changed.encode()).await.is_err() {
                        return;
                    }
                }
            }
        }
        end(&mut stream, Status::Ok).await;
    }
}

async fn end(stream: &mut GalleryStream, status: Status) {
    let _ = write_raw(
        &mut stream.send,
        Channel::Gallery,
        &Reply::End(status).encode(),
    )
    .await;
    let _ = stream.send.finish();
}

fn abort(stream: &mut GalleryStream, code: ErrorCode) {
    stream.recv.stop(code);
    let _ = stream.send.reset(crate::session::varint_code(code));
}

/// Why a request of the PC gave nothing.
#[derive(Debug)]
pub enum AskError {
    /// The phone doesn't take `gallery/1`, or the session broke.
    Session(SessionError),
    /// The phone's `End` said so.
    Status(Status),
    /// A wrong or malformed answer, or none in [`ANSWER_TIMEOUT`].
    Protocol,
}

impl From<SessionError> for AskError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

async fn open(session: &Session, request: &Request) -> Result<GalleryStream, AskError> {
    let mut stream = session.open_gallery().await?;
    write_raw(&mut stream.send, Channel::Gallery, &request.encode()).await?;
    Ok(stream)
}

/// Sends one request and reads its single reply and `End`.
async fn ask_one(session: &Session, request: Request) -> Result<Reply, AskError> {
    let mut stream = open(session, &request).await?;
    let _ = stream.send.finish();
    let read = async {
        let mut reply = None;
        loop {
            match stream.recv.next_raw().await {
                Ok(Some(m)) => match Reply::decode(&m) {
                    Some(Reply::End(Status::Ok)) => return reply.ok_or(AskError::Protocol),
                    Some(Reply::End(s)) => return Err(AskError::Status(s)),
                    Some(r @ (Reply::Items(_) | Reply::Thumb(_))) if reply.is_none() => {
                        reply = Some(r)
                    }
                    _ => return Err(AskError::Protocol),
                },
                _ => return Err(AskError::Protocol),
            }
        }
    };
    let result = tokio::time::timeout(ANSWER_TIMEOUT, read)
        .await
        .unwrap_or(Err(AskError::Protocol));
    if result.is_err() {
        stream.recv.stop(ErrorCode::Cancelled);
    }
    result
}

/// The newest `limit` photos and screenshots, newest first (§9.13).
pub async fn recent(session: &Session, limit: u32) -> Result<Vec<Item>, AskError> {
    match ask_one(session, Request::Recent { limit }).await? {
        Reply::Items(items) if items.len() <= limit as usize => Ok(items),
        _ => Err(AskError::Protocol),
    }
}

/// A JPEG thumbnail of photo `id`, its longer side at most `size` px; `Status(NotFound)` when the photo is gone.
pub async fn thumb(session: &Session, id: u64, size: u32) -> Result<Vec<u8>, AskError> {
    match ask_one(session, Request::Thumb { id, size }).await? {
        Reply::Thumb(jpeg) => Ok(jpeg),
        _ => Err(AskError::Protocol),
    }
}

/// An original on its way from the phone: [`Download::info`], then its bytes from [`Download::next`].
pub struct Download {
    pub info: FileInfo,
    stream: GalleryStream,
    received: u64,
    done: bool,
}

impl Download {
    /// The next bytes, in order; `Ok(None)` once all `info.size` bytes came and the phone ended with `End(Ok)`.
    /// More or fewer bytes than announced, or [`ANSWER_TIMEOUT`] without a frame, is `Protocol`.
    pub async fn next(&mut self) -> Result<Option<Vec<u8>>, AskError> {
        if self.done {
            return Ok(None);
        }
        let frame = tokio::time::timeout(ANSWER_TIMEOUT, self.stream.recv.next_raw()).await;
        let result = match frame {
            Ok(Ok(Some(m))) => match Reply::decode(&m) {
                Some(Reply::Chunk(b)) if self.received + b.len() as u64 <= self.info.size => {
                    self.received += b.len() as u64;
                    Ok(Some(b))
                }
                Some(Reply::End(Status::Ok)) if self.received == self.info.size => {
                    self.done = true;
                    Ok(None)
                }
                Some(Reply::End(s)) if s != Status::Ok => Err(AskError::Status(s)),
                _ => Err(AskError::Protocol),
            },
            _ => Err(AskError::Protocol),
        };
        if result.is_err() {
            self.done = true;
            self.stream.recv.stop(ErrorCode::Cancelled);
        }
        result
    }

    /// Bytes received so far.
    pub fn received(&self) -> u64 {
        self.received
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        if !self.done {
            self.stream.recv.stop(ErrorCode::Cancelled);
        }
    }
}

/// Starts fetching photo `id` as it is on the phone (§9.13).
pub async fn original(session: &Session, id: u64) -> Result<Download, AskError> {
    let mut stream = open(session, &Request::Original { id }).await?;
    let _ = stream.send.finish();
    let first = tokio::time::timeout(ANSWER_TIMEOUT, stream.recv.next_raw()).await;
    let info = match first {
        Ok(Ok(Some(m))) => match Reply::decode(&m) {
            Some(Reply::File(info)) => Ok(info),
            Some(Reply::End(s)) if s != Status::Ok => Err(AskError::Status(s)),
            _ => Err(AskError::Protocol),
        },
        _ => Err(AskError::Protocol),
    };
    match info {
        Ok(info) => Ok(Download {
            info,
            stream,
            received: 0,
            done: false,
        }),
        Err(e) => {
            stream.recv.stop(ErrorCode::Cancelled);
            Err(e)
        }
    }
}

/// Watches the phone's photos: the receiver gets `()` per `Changed` and closes when the watch ends (the session,
/// the phone withdrawing `gallery/1`, or dropping the receiver).
pub async fn watch(session: &Session) -> Result<mpsc::Receiver<()>, AskError> {
    let mut stream = open(session, &Request::Watch).await?;
    let (tx, rx) = mpsc::channel(4);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                m = stream.recv.next_raw() => match m {
                    Ok(Some(m)) if Reply::decode(&m) == Some(Reply::Changed) => {
                        // A full queue already holds a change: one is enough.
                        let _ = tx.try_send(());
                    }
                    _ => break,
                },
                _ = tx.closed() => break,
            }
        }
        stream.recv.stop(ErrorCode::Cancelled);
        let _ = stream.send.finish();
    });
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    fn item(id: u64) -> Item {
        Item {
            id,
            date_ms: 1_700_000_000_000 + id,
            name: format!("IMG_{id}.jpg"),
            mime: "image/jpeg".into(),
            width: 4000,
            height: 3000,
            size: 3_000_000,
            screenshot: id.is_multiple_of(2),
        }
    }

    #[test]
    fn requests_and_replies_round_trip() {
        for r in [
            Request::Recent { limit: 20 },
            Request::Thumb { id: 7, size: 256 },
            Request::Original { id: 7 },
            Request::Watch,
        ] {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
        for r in [
            Reply::Items(vec![item(1), item(2)]),
            Reply::Items(vec![]),
            Reply::Thumb(vec![0xff, 0xd8, 0xff, 0xe0]),
            Reply::File(FileInfo {
                name: "PXL_20261113.jpg".into(),
                mime: "image/jpeg".into(),
                size: 5,
            }),
            Reply::Chunk(vec![1, 2, 3]),
            Reply::Changed,
            Reply::End(Status::NotFound),
        ] {
            assert_eq!(Reply::decode(&r.encode()), Some(r));
        }
    }

    #[test]
    fn out_of_range_is_refused_not_clamped() {
        for r in [
            Request::Recent { limit: 0 },
            Request::Recent { limit: 61 },
            Request::Thumb { id: 1, size: 63 },
            Request::Thumb { id: 1, size: 513 },
        ] {
            assert_eq!(Request::decode(&r.encode()), None, "{r:?}");
        }
        assert_eq!(Request::decode(&RawMessage::new(4, map(vec![]))), None);
        // Not a JPEG, too many items, a name Windows can't create, an empty or oversized file.
        assert_eq!(
            Reply::decode(&Reply::Thumb(b"\x89PNG".to_vec()).encode()),
            None
        );
        assert_eq!(
            Reply::decode(&Reply::Items((0..61).map(item).collect()).encode()),
            None
        );
        for (name, size) in [
            ("..", 5),
            ("a/b.jpg", 5),
            ("CON.jpg", 5),
            ("x:y.jpg", 5),
            ("ok.jpg", 0),
            ("ok.jpg", MAX_ORIGINAL + 1),
        ] {
            let r = Reply::File(FileInfo {
                name: name.into(),
                mime: "image/jpeg".into(),
                size,
            });
            assert_eq!(Reply::decode(&r.encode()), None, "{name} {size}");
        }
        assert_eq!(
            Reply::decode(&Reply::Chunk(vec![0; CHUNK + 1]).encode()),
            None
        );
    }

    #[test]
    fn names_are_fitted_for_the_pc() {
        assert_eq!(fit_name("PXL_1.jpg", 1, "image/jpeg"), "PXL_1.jpg");
        assert_eq!(fit_name("a:b.png", 9, "image/png"), "IMG_9.png");
        assert_eq!(fit_name("", 3, "image/heic"), "IMG_3.heic");
        assert_eq!(fit_name("nul", 4, "x"), "IMG_4.jpg");
    }

    #[test]
    fn spec_examples() {
        // PROTOCOL.md §9.13, with the frame length.
        let hex = |m: &RawMessage| {
            encode_raw_frame(Channel::Gallery, m)
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            hex(&Request::Recent { limit: 20 }.encode()),
            "05 82 00 a1 00 14"
        );
        assert_eq!(
            hex(&Request::Thumb { id: 7, size: 256 }.encode()),
            "09 82 01 a2 00 07 01 19 01 00"
        );
        assert_eq!(
            hex(&Request::Original { id: 7 }.encode()),
            "05 82 02 a1 00 07"
        );
        assert_eq!(hex(&Request::Watch.encode()), "03 82 03 a0");
        assert_eq!(hex(&Reply::Changed.encode()), "03 82 04 a0");
        assert_eq!(hex(&Reply::End(Status::Ok).encode()), "05 82 05 a1 00 00");
        assert_eq!(
            hex(&Reply::Chunk(vec![1, 2, 3]).encode()),
            "08 82 03 a1 00 43 01 02 03"
        );
    }
}
