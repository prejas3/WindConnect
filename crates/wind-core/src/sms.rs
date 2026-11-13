//! CCS/1 M:sms/1, the phone's SMS threads read on the PC (§9.11, WN-55). The PC opens one bidi `sms` stream per
//! request: `Threads` [0,{0:limit}], `Messages` [1,{0:thread,1?:before,2?:since,3:limit}], `Photo` [2,{0:address}],
//! `Watch` [3,{}]. The phone answers in batches (`Threads` [0,{0:[..]}], `Messages` [1,{0:[..]}], `Photo` [2,{0:png}],
//! `Changed` [3,{}] for a watch), then one `End` [4,{0:status}] and FIN.
//! RULE:the phone announces `sms/1` only with READ_SMS granted and the person's setting on; nothing is persisted here
//! and no text or number is ever logged. Incremental sync: `since` (inclusive) on dates, the PC dedups by (kind, id).
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Semaphore};

use crate::{
    protocol::{cbor::Value, Channel, ErrorCode, RawMessage},
    session::{write_raw, Session, SessionError, SmsStream},
};

/// The request must arrive this soon after the stream opens.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// The PC gives up on an answer after this long.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);
/// Most threads in one answer.
pub const MAX_THREADS: u32 = 500;
/// Most messages in one answer.
pub const MAX_MESSAGES: u32 = 2000;
/// Most people in one thread.
pub const MAX_PEOPLE: usize = 20;
/// Bytes of an address (a number or an e-mail gateway address).
pub const MAX_ADDRESS: usize = 64;
/// Bytes of a contact's name.
pub const MAX_NAME: usize = 128;
/// Bytes of a thread's snippet.
pub const MAX_SNIPPET: usize = 512;
/// Bytes of a message body (longer bodies are cut on the phone at a character boundary).
pub const MAX_BODY: usize = 16 * 1024;
/// Bytes of a contact photo (PNG).
pub const MAX_PHOTO: usize = 64 * 1024;
/// A batch is flushed once its items reach about this many bytes (well under the 1 MiB frame).
const BATCH_BYTES: usize = 256 * 1024;
/// Requests served at once per session (a watch doesn't count).
pub const CONCURRENT: usize = 4;
/// Watches at once per session; one more gets `End(Busy)`.
pub const WATCHES: usize = 2;
/// How long the phone gathers changes before one `Changed`.
pub const CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);

/// How a request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// No READ_SMS, or the person turned SMS on the PC off.
    Denied,
    /// No such thread, or no photo for the address.
    NotFound,
    /// Too many requests or watches.
    Busy,
    Failed,
}

impl Status {
    pub fn code(self) -> u64 {
        match self {
            Self::Ok => 0,
            Self::Denied => 1,
            Self::NotFound => 2,
            Self::Busy => 3,
            Self::Failed => 4,
        }
    }
    /// An unknown code is `Failed`.
    pub fn from_code(code: u64) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::Denied,
            2 => Self::NotFound,
            3 => Self::Busy,
            _ => Self::Failed,
        }
    }
}

/// From the PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The newest `limit` threads, newest first.
    Threads { limit: u32 },
    /// A thread's messages, newest first: at most `limit`, dated at most `before` and at least `since` (both
    /// inclusive, ms since the epoch).
    Messages {
        thread: u64,
        before: Option<u64>,
        since: Option<u64>,
        limit: u32,
    },
    /// The contact photo of an address, when it has one.
    Photo { address: String },
    /// `Changed` whenever SMS change on the phone, until either side ends the stream.
    Watch,
}

impl Request {
    pub fn encode(&self) -> RawMessage {
        let (ty, f) = match self {
            Self::Threads { limit } => (0, vec![(0, uint(*limit as u64))]),
            Self::Messages {
                thread,
                before,
                since,
                limit,
            } => {
                let mut f = vec![(0, uint(*thread))];
                if let Some(b) = before {
                    f.push((1, uint(*b)));
                }
                if let Some(s) = since {
                    f.push((2, uint(*s)));
                }
                f.push((3, uint(*limit as u64)));
                (1, f)
            }
            Self::Photo { address } => (2, vec![(0, Value::Text(address.clone()))]),
            Self::Watch => (3, vec![]),
        };
        RawMessage::new(ty, map(f))
    }

    /// `None` for an unknown type or a malformed message (a limit of 0 or over the maximum, an empty or long
    /// address): never clamped.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields(m)?;
        match m.ty {
            0 => Some(Self::Threads {
                limit: limit(&f, 0, MAX_THREADS)?,
            }),
            1 => Some(Self::Messages {
                thread: get_uint(&f, 0)?,
                before: opt_uint(&f, 1)?,
                since: opt_uint(&f, 2)?,
                limit: limit(&f, 3, MAX_MESSAGES)?,
            }),
            2 => match get(&f, 0)? {
                Value::Text(a) if !a.is_empty() && a.len() <= MAX_ADDRESS => {
                    Some(Self::Photo { address: a.clone() })
                }
                _ => None,
            },
            3 => Some(Self::Watch),
            _ => None,
        }
    }
}

/// Someone in a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub address: String,
    /// From the phone's contacts (READ_CONTACTS), else none.
    pub name: Option<String>,
    /// A `Photo` request for the address will find one.
    pub photo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: u64,
    /// The newest message, ms since the epoch.
    pub date_ms: u64,
    pub snippet: String,
    pub people: Vec<Person>,
    pub unread: u32,
    pub count: u32,
}

/// Where a message stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageBox {
    /// Received.
    Inbox,
    Sent,
    /// Still being sent.
    Pending,
    Failed,
}

impl MessageBox {
    pub fn code(self) -> u64 {
        match self {
            Self::Inbox => 0,
            Self::Sent => 1,
            Self::Pending => 2,
            Self::Failed => 3,
        }
    }
    /// An unknown code is `Inbox`.
    pub fn from_code(code: u64) -> Self {
        match code {
            1 => Self::Sent,
            2 => Self::Pending,
            3 => Self::Failed,
            _ => Self::Inbox,
        }
    }
    pub fn outgoing(self) -> bool {
        self != Self::Inbox
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Unique within its kind (SMS and MMS are numbered apart).
    pub id: u64,
    pub mms: bool,
    pub date_ms: u64,
    pub folder: MessageBox,
    /// Possibly empty (an MMS with only a picture).
    pub body: String,
    /// The sender of a received message in a group thread; else none.
    pub address: Option<String>,
    pub read: bool,
    /// MMS parts that aren't text (shown as a note; WN-56 shows them).
    pub attachments: u32,
}

impl Message {
    /// What the PC dedups by.
    pub fn key(&self) -> (bool, u64) {
        (self.mms, self.id)
    }
}

/// From the phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Threads(Vec<Thread>),
    Messages(Vec<Message>),
    Photo(Vec<u8>),
    Changed,
    End(Status),
}

impl Reply {
    pub fn encode(&self) -> RawMessage {
        let (ty, f) = match self {
            Self::Threads(items) => (
                0,
                vec![(0, Value::Array(items.iter().map(thread_value).collect()))],
            ),
            Self::Messages(items) => (
                1,
                vec![(0, Value::Array(items.iter().map(message_value).collect()))],
            ),
            Self::Photo(png) => (2, vec![(0, Value::Bytes(png.clone()))]),
            Self::Changed => (3, vec![]),
            Self::End(status) => (4, vec![(0, uint(status.code()))]),
        };
        RawMessage::new(ty, map(f))
    }

    /// `None` for an unknown type or anything over the limits (never trimmed).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields(m)?;
        match m.ty {
            0 => Some(Self::Threads(items(&f, MAX_THREADS, decode_thread)?)),
            1 => Some(Self::Messages(items(&f, MAX_MESSAGES, decode_message)?)),
            2 => match get(&f, 0)? {
                Value::Bytes(b) if !b.is_empty() && b.len() <= MAX_PHOTO => {
                    Some(Self::Photo(b.clone()))
                }
                _ => None,
            },
            3 => Some(Self::Changed),
            4 => Some(Self::End(Status::from_code(get_uint(&f, 0)?))),
            _ => None,
        }
    }
}

fn uint(n: u64) -> Value {
    Value::Int(n.into())
}

fn map(f: Vec<(u64, Value)>) -> Value {
    Value::Map(f.into_iter().map(|(k, v)| (uint(k), v)).collect())
}

fn text_or_null(s: &Option<String>) -> Value {
    s.as_ref().map_or(Value::Null, |s| Value::Text(s.clone()))
}

fn thread_value(t: &Thread) -> Value {
    map(vec![
        (0, uint(t.id)),
        (1, uint(t.date_ms)),
        (2, Value::Text(t.snippet.clone())),
        (
            3,
            Value::Array(
                t.people
                    .iter()
                    .map(|p| {
                        Value::Array(vec![
                            Value::Text(p.address.clone()),
                            text_or_null(&p.name),
                            Value::Bool(p.photo),
                        ])
                    })
                    .collect(),
            ),
        ),
        (4, uint(t.unread as u64)),
        (5, uint(t.count as u64)),
    ])
}

fn message_value(m: &Message) -> Value {
    let mut f = vec![
        (0, uint(m.id)),
        (1, Value::Bool(m.mms)),
        (2, uint(m.date_ms)),
        (3, uint(m.folder.code())),
        (4, Value::Text(m.body.clone())),
    ];
    if let Some(a) = &m.address {
        f.push((5, Value::Text(a.clone())));
    }
    f.push((6, Value::Bool(m.read)));
    if m.attachments > 0 {
        f.push((7, uint(m.attachments as u64)));
    }
    map(f)
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

fn fields(m: &RawMessage) -> Option<Fields<'_>> {
    fields_of(&m.body)
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

/// `Some(None)` when absent, `None` when not a natural number.
fn opt_uint(f: &Fields<'_>, key: u64) -> Option<Option<u64>> {
    match get(f, key) {
        None => Some(None),
        Some(v) => as_uint(v).map(Some),
    }
}

fn limit(f: &Fields<'_>, key: u64, max: u32) -> Option<u32> {
    let n = get_uint(f, key)?;
    (1..=max as u64).contains(&n).then_some(n as u32)
}

fn bounded_text(v: &Value, max: usize) -> Option<String> {
    match v {
        Value::Text(t) if t.len() <= max => Some(t.clone()),
        _ => None,
    }
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn items<T>(f: &Fields<'_>, max: u32, one: fn(&Value) -> Option<T>) -> Option<Vec<T>> {
    let Value::Array(list) = get(f, 0)? else {
        return None;
    };
    if list.len() > max as usize {
        return None;
    }
    list.iter().map(one).collect()
}

fn decode_person(v: &Value) -> Option<Person> {
    let Value::Array(parts) = v else { return None };
    let [address, name, photo] = parts.as_slice() else {
        return None;
    };
    let address = bounded_text(address, MAX_ADDRESS).filter(|a| !a.is_empty())?;
    let name = match name {
        Value::Null => None,
        v => Some(bounded_text(v, MAX_NAME)?),
    };
    Some(Person {
        address,
        name,
        photo: as_bool(photo)?,
    })
}

fn decode_thread(v: &Value) -> Option<Thread> {
    let f = fields_of(v)?;
    let Value::Array(people) = get(&f, 3)? else {
        return None;
    };
    if people.len() > MAX_PEOPLE {
        return None;
    }
    Some(Thread {
        id: get_uint(&f, 0)?,
        date_ms: get_uint(&f, 1)?,
        snippet: bounded_text(get(&f, 2)?, MAX_SNIPPET)?,
        people: people.iter().map(decode_person).collect::<Option<_>>()?,
        unread: u32::try_from(get_uint(&f, 4)?).ok()?,
        count: u32::try_from(get_uint(&f, 5)?).ok()?,
    })
}

fn decode_message(v: &Value) -> Option<Message> {
    let f = fields_of(v)?;
    Some(Message {
        id: get_uint(&f, 0)?,
        mms: as_bool(get(&f, 1)?)?,
        date_ms: get_uint(&f, 2)?,
        folder: MessageBox::from_code(get_uint(&f, 3)?),
        body: bounded_text(get(&f, 4)?, MAX_BODY)?,
        address: match get(&f, 5) {
            None => None,
            Some(v) => Some(bounded_text(v, MAX_ADDRESS).filter(|a| !a.is_empty())?),
        },
        read: as_bool(get(&f, 6)?)?,
        attachments: match opt_uint(&f, 7)? {
            None => 0,
            Some(n) => u32::try_from(n).ok()?,
        },
    })
}

/// The longest prefix of `s` within `max` bytes, cut at a character boundary.
pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// What the phone's source gave, fitted to the limits: long texts cut, extra people dropped, empty addresses
/// removed. The source is trusted code, but one long contact name must not fail a whole answer.
fn fit_thread(mut t: Thread) -> Thread {
    t.snippet = clip(&t.snippet, MAX_SNIPPET);
    t.people
        .retain(|p| !p.address.is_empty() && p.address.len() <= MAX_ADDRESS);
    t.people.truncate(MAX_PEOPLE);
    for p in &mut t.people {
        p.name = p.name.take().map(|n| clip(&n, MAX_NAME));
    }
    t
}

fn fit_message(mut m: Message) -> Message {
    m.body = clip(&m.body, MAX_BODY);
    m.address = m
        .address
        .filter(|a| !a.is_empty() && a.len() <= MAX_ADDRESS);
    m
}

/// The phone's SMS store. Called off the async runtime; may block on the content provider.
pub trait Source: Send + Sync {
    fn threads(&self, limit: u32) -> Result<Vec<Thread>, Status>;
    fn messages(
        &self,
        thread: u64,
        before: Option<u64>,
        since: Option<u64>,
        limit: u32,
    ) -> Result<Vec<Message>, Status>;
    /// A PNG of at most [`MAX_PHOTO`] bytes.
    fn photo(&self, address: &str) -> Result<Vec<u8>, Status>;
}

/// Serves `sms` streams on the phone.
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

    /// SMS changed on the phone (a content observer): every watch sends `Changed` after [`CHANGE_DEBOUNCE`].
    pub fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Serves the streams of one session until it ends.
    pub async fn serve(self: Arc<Self>, mut streams: mpsc::Receiver<SmsStream>) {
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
        mut stream: SmsStream,
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
        let answer = tokio::task::spawn_blocking(move || match request {
            Request::Threads { limit } => source.threads(limit).map(|t| {
                let mut t: Vec<Reply> = batches(
                    t.into_iter().map(fit_thread).take(limit as usize),
                    thread_value,
                )
                .into_iter()
                .map(Reply::Threads)
                .collect();
                if t.is_empty() {
                    t.push(Reply::Threads(vec![]));
                }
                t
            }),
            Request::Messages {
                thread,
                before,
                since,
                limit,
            } => source.messages(thread, before, since, limit).map(|m| {
                let mut m: Vec<Reply> = batches(
                    m.into_iter().map(fit_message).take(limit as usize),
                    message_value,
                )
                .into_iter()
                .map(Reply::Messages)
                .collect();
                if m.is_empty() {
                    m.push(Reply::Messages(vec![]));
                }
                m
            }),
            Request::Photo { address } => source.photo(&address).and_then(|png| {
                if png.is_empty() || png.len() > MAX_PHOTO || !crate::notify::is_png(&png) {
                    Err(Status::NotFound)
                } else {
                    Ok(vec![Reply::Photo(png)])
                }
            }),
            Request::Watch => unreachable!("handled above"),
        })
        .await
        .unwrap_or(Err(Status::Failed));
        let status = match answer {
            Ok(replies) => {
                for r in &replies {
                    if write_raw(&mut stream.send, Channel::Sms, &r.encode())
                        .await
                        .is_err()
                    {
                        return; // the PC left
                    }
                }
                Status::Ok
            }
            Err(s) => s,
        };
        end(&mut stream, status).await;
    }

    async fn watch(&self, mut stream: SmsStream) {
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
                    if write_raw(&mut stream.send, Channel::Sms, &Reply::Changed.encode()).await.is_err() {
                        return;
                    }
                }
            }
        }
        end(&mut stream, Status::Ok).await;
    }
}

/// Splits items into batches of about [`BATCH_BYTES`].
fn batches<T>(items: impl Iterator<Item = T>, value: fn(&T) -> Value) -> Vec<Vec<T>> {
    let mut out = vec![];
    let mut batch = vec![];
    let mut bytes = 0;
    for item in items {
        let mut buf = Vec::new();
        crate::protocol::cbor::encode(&value(&item), &mut buf);
        if !batch.is_empty() && bytes + buf.len() > BATCH_BYTES {
            out.push(std::mem::take(&mut batch));
            bytes = 0;
        }
        bytes += buf.len();
        batch.push(item);
    }
    if !batch.is_empty() {
        out.push(batch);
    }
    out
}

async fn end(stream: &mut SmsStream, status: Status) {
    let _ = write_raw(&mut stream.send, Channel::Sms, &Reply::End(status).encode()).await;
    let _ = stream.send.finish();
}

fn abort(stream: &mut SmsStream, code: ErrorCode) {
    stream.recv.stop(code);
    let _ = stream.send.reset(crate::session::varint_code(code));
}

/// Why a request of the PC gave nothing.
#[derive(Debug)]
pub enum AskError {
    /// The phone doesn't take `sms/1`, or the session broke.
    Session(SessionError),
    /// The phone's `End` said so.
    Status(Status),
    /// A wrong or malformed answer, an answer over the asked limit, or no answer in [`ANSWER_TIMEOUT`].
    Protocol,
}

impl From<SessionError> for AskError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

/// Sends one request and gathers its batches until `End`.
async fn ask(session: &Session, request: Request) -> Result<Vec<Reply>, AskError> {
    let mut stream = session.open_sms().await?;
    write_raw(&mut stream.send, Channel::Sms, &request.encode()).await?;
    let _ = stream.send.finish();
    let gather = async {
        let mut replies = vec![];
        loop {
            match stream.recv.next_raw().await {
                Ok(Some(m)) => match Reply::decode(&m) {
                    Some(Reply::End(Status::Ok)) => return Ok(replies),
                    Some(Reply::End(s)) => return Err(AskError::Status(s)),
                    Some(r) => replies.push(r),
                    None => return Err(AskError::Protocol),
                },
                _ => return Err(AskError::Protocol),
            }
        }
    };
    match tokio::time::timeout(ANSWER_TIMEOUT, gather).await {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) => {
            stream.recv.stop(ErrorCode::Cancelled);
            Err(e)
        }
        Err(_) => {
            stream.recv.stop(ErrorCode::Cancelled);
            Err(AskError::Protocol)
        }
    }
}

/// The newest `limit` threads (§9.11).
pub async fn threads(session: &Session, limit: u32) -> Result<Vec<Thread>, AskError> {
    let mut out = vec![];
    for r in ask(session, Request::Threads { limit }).await? {
        let Reply::Threads(t) = r else {
            return Err(AskError::Protocol);
        };
        out.extend(t);
    }
    if out.len() > limit as usize {
        return Err(AskError::Protocol);
    }
    Ok(out)
}

/// A thread's messages, newest first (§9.11).
pub async fn messages(
    session: &Session,
    thread: u64,
    before: Option<u64>,
    since: Option<u64>,
    limit: u32,
) -> Result<Vec<Message>, AskError> {
    let mut out = vec![];
    let request = Request::Messages {
        thread,
        before,
        since,
        limit,
    };
    for r in ask(session, request).await? {
        let Reply::Messages(m) = r else {
            return Err(AskError::Protocol);
        };
        out.extend(m);
    }
    if out.len() > limit as usize {
        return Err(AskError::Protocol);
    }
    Ok(out)
}

/// The contact photo of `address`: a PNG, or `None` when it has none.
pub async fn photo(session: &Session, address: &str) -> Result<Option<Vec<u8>>, AskError> {
    let request = Request::Photo {
        address: address.to_owned(),
    };
    match ask(session, request).await {
        Ok(replies) => match replies.as_slice() {
            [Reply::Photo(png)] if crate::notify::is_png(png) => Ok(Some(png.clone())),
            _ => Err(AskError::Protocol),
        },
        Err(AskError::Status(Status::NotFound)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Watches the phone's SMS: the receiver gets `()` per `Changed` and closes when the watch ends (the session, the
/// phone withdrawing `sms/1`, or dropping the receiver).
pub async fn watch(session: &Session) -> Result<mpsc::Receiver<()>, AskError> {
    let mut stream = session.open_sms().await?;
    write_raw(&mut stream.send, Channel::Sms, &Request::Watch.encode()).await?;
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

    fn message(id: u64) -> Message {
        Message {
            id,
            mms: false,
            date_ms: 1_700_000_000_000 + id,
            folder: MessageBox::Inbox,
            body: "Hej".into(),
            address: None,
            read: true,
            attachments: 0,
        }
    }

    #[test]
    fn requests_round_trip_and_bytes() {
        let all = [
            Request::Threads { limit: 200 },
            Request::Messages {
                thread: 7,
                before: Some(5),
                since: None,
                limit: 1000,
            },
            Request::Messages {
                thread: 7,
                before: None,
                since: Some(9),
                limit: 1,
            },
            Request::Photo {
                address: "+48600100200".into(),
            },
            Request::Watch,
        ];
        for r in all {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
        assert_eq!(
            encode_raw_frame(Channel::Sms, &Request::Threads { limit: 200 }.encode()).unwrap(),
            [0x06, 0x82, 0x00, 0xa1, 0x00, 0x18, 0xc8]
        );
        let since = Request::Messages {
            thread: 7,
            before: None,
            since: Some(1000),
            limit: 1000,
        };
        assert_eq!(
            encode_raw_frame(Channel::Sms, &since.encode()).unwrap(),
            [0x0d, 0x82, 0x01, 0xa3, 0x00, 0x07, 0x02, 0x19, 0x03, 0xe8, 0x03, 0x19, 0x03, 0xe8]
        );
        assert_eq!(
            encode_raw_frame(Channel::Sms, &Request::Watch.encode()).unwrap(),
            [0x03, 0x82, 0x03, 0xa0]
        );
        assert_eq!(
            encode_raw_frame(Channel::Sms, &Reply::End(Status::Ok).encode()).unwrap(),
            [0x05, 0x82, 0x04, 0xa1, 0x00, 0x00]
        );
        assert_eq!(
            encode_raw_frame(Channel::Sms, &Reply::Changed.encode()).unwrap(),
            [0x03, 0x82, 0x03, 0xa0]
        );
    }

    #[test]
    fn limits_are_refused_not_clamped() {
        let threads = |n: i128| RawMessage::new(0, map(vec![(0, Value::Int(n))]));
        assert_eq!(Request::decode(&threads(0)), None);
        assert_eq!(Request::decode(&threads(501)), None);
        assert_eq!(Request::decode(&threads(-1)), None);
        let photo = |a: &str| RawMessage::new(2, map(vec![(0, Value::Text(a.into()))]));
        assert_eq!(Request::decode(&photo("")), None);
        assert_eq!(Request::decode(&photo(&"1".repeat(65))), None);
        assert_eq!(Request::decode(&RawMessage::new(9, map(vec![]))), None);
        let long = Message {
            body: "x".repeat(MAX_BODY + 1),
            ..message(1)
        };
        assert_eq!(Reply::decode(&Reply::Messages(vec![long]).encode()), None);
        let too_many = Reply::Messages((0..=MAX_MESSAGES as u64).map(message).collect());
        assert_eq!(Reply::decode(&too_many.encode()), None);
    }

    #[test]
    fn replies_round_trip() {
        let thread = Thread {
            id: 3,
            date_ms: 1_700_000_000_000,
            snippet: "Będziesz o 19?".into(),
            people: vec![
                Person {
                    address: "+48600100200".into(),
                    name: Some("Ola".into()),
                    photo: true,
                },
                Person {
                    address: "+48600100300".into(),
                    name: None,
                    photo: false,
                },
            ],
            unread: 2,
            count: 1000,
        };
        let mms = Message {
            mms: true,
            folder: MessageBox::Sent,
            body: String::new(),
            address: Some("+48600100200".into()),
            read: false,
            attachments: 2,
            ..message(9)
        };
        for r in [
            Reply::Threads(vec![thread]),
            Reply::Messages(vec![message(1), mms]),
            Reply::Photo(vec![0x89, 0x50, 0x4e, 0x47]),
            Reply::Changed,
            Reply::End(Status::Denied),
        ] {
            assert_eq!(Reply::decode(&r.encode()), Some(r));
        }
        let unknown = RawMessage::new(4, map(vec![(0, uint(77))]));
        assert_eq!(Reply::decode(&unknown), Some(Reply::End(Status::Failed)));
    }

    #[test]
    fn fitting_cuts_at_character_boundaries() {
        assert_eq!(clip("ąę", 3), "ą");
        assert_eq!(clip("abc", 3), "abc");
        let t = fit_thread(Thread {
            id: 1,
            date_ms: 0,
            snippet: "ż".repeat(MAX_SNIPPET),
            people: (0..30)
                .map(|i| Person {
                    address: if i == 0 {
                        String::new()
                    } else {
                        format!("{i}")
                    },
                    name: Some("ł".repeat(100)),
                    photo: false,
                })
                .collect(),
            unread: 0,
            count: 0,
        });
        assert!(t.snippet.len() <= MAX_SNIPPET);
        assert_eq!(t.people.len(), MAX_PEOPLE);
        assert!(t
            .people
            .iter()
            .all(|p| p.name.as_ref().unwrap().len() <= MAX_NAME));
        assert!(Reply::decode(&Reply::Threads(vec![t]).encode()).is_some());
    }

    #[test]
    fn batches_stay_under_a_frame() {
        let big = |id| Message {
            body: "x".repeat(MAX_BODY),
            ..message(id)
        };
        let b = batches((0..100).map(big), message_value);
        assert!(b.len() > 1);
        for batch in b {
            let frame = encode_raw_frame(Channel::Sms, &Reply::Messages(batch).encode()).unwrap();
            assert!(frame.len() < crate::protocol::MAX_STREAM_FRAME);
        }
    }
}
