//! CCS/1 M:clipboard/1 | RULE:bounded plain text, explicit OS-applied ack, no content logging/persistence.
use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session},
};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch, Semaphore};

pub const MAX_TEXT: usize = 256 * 1024;
pub type Id = [u8; 16];
type Key = (u64, Id);
type Pending = Arc<Mutex<HashMap<Id, oneshot::Sender<Status>>>>;

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
        }
    }
}

/// CCS/1 SEC:platform checks sensitivity before publish; received text always opts out of history/cloud.
pub trait Sink: Send + Sync {
    fn apply(&self, text: &str) -> bool;
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
}
impl Default for Clipboard {
    fn default() -> Self {
        Self {
            epoch: watch::channel(0).0,
            state: Mutex::new(State::default()),
            enabled: AtomicBool::new(true),
            sink: Mutex::new(None),
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
    pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
        router.route(Channel::Clipboard, 1, 4)
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
        let alive = Arc::new(AtomicBool::new(true));
        let (shared, p, live, s) = (
            self.clone(),
            pending.clone(),
            alive.clone(),
            session.clone(),
        );
        let worker = tokio::spawn(async move {
            loop {
                let incoming = tokio::select! { _=changed.changed()=>break,_=s.closed()=>break, incoming=input.recv()=>match incoming { Some(m)=>m,None=>break } };
                match Message::decode(&incoming.message) {
                    Some(Message::Ack { id, status }) => {
                        if let Some(waiter) = p.lock().unwrap().remove(&id) {
                            let _ = waiter.send(status);
                        }
                    }
                    Some(Message::Set { key, text }) => {
                        let (hub, live, session) = (shared.clone(), live.clone(), s.clone());
                        let status = tokio::task::spawn_blocking(move || {
                            // CCS/1 RULE:one serialized clock/OS commit; callbacks must never synchronously re-enter this mutex.
                            let mut state = hub.state.lock().unwrap();
                            if !live.load(Ordering::SeqCst)
                                || *hub.epoch.borrow() != epoch
                                || session.connection().quinn().close_reason().is_some()
                                || !session
                                    .accepted_capabilities()
                                    .iter()
                                    .any(|c| c.name() == "clipboard" && c.version() == 1)
                            {
                                return Status::Refused;
                            }
                            state.clock = state.clock.max(key.0);
                            if !hub.enabled() || state.key.is_some_and(|last| key <= last) {
                                return Status::Refused;
                            }
                            let sink = hub.sink.lock().unwrap().clone();
                            let Some(sink) = sink else {
                                return Status::Refused;
                            };
                            // A failed write never produces Applied. The clock still advances past the attempted remote update.
                            state.key = Some(key);
                            if !sink.apply(&text) {
                                return Status::Failed;
                            }
                            state.hash = Some(hash(&text));
                            Status::Applied
                        })
                        .await
                        .unwrap_or(Status::Failed);
                        if s.send(
                            Channel::Clipboard,
                            &Message::Ack { id: key.1, status }.encode(),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    None => { /* CCS/1 SEC:malformed clipboard data is discarded, never written to the OS. */
                    }
                }
            }
            live.store(false, Ordering::SeqCst);
            p.lock().unwrap().clear();
        });
        Arc::new(Connection {
            epoch,
            shared: self.clone(),
            session,
            pending,
            alive,
            worker,
            limit: Semaphore::new(4),
        })
    }
}

pub struct Connection {
    epoch: u64,
    shared: Arc<Clipboard>,
    session: Session,
    pending: Pending,
    alive: Arc<AtomicBool>,
    worker: tokio::task::JoinHandle<()>,
    limit: Semaphore,
}
struct PendingGuard {
    pending: Pending,
    id: Id,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
impl Connection {
    pub async fn send(&self, text: String, sensitive: bool, manual: bool) -> Status {
        if text.len() > MAX_TEXT || text.contains('\0') {
            return Status::Invalid;
        }
        let Ok(_permit) = self.limit.try_acquire() else {
            return Status::Busy;
        };
        let digest = hash(&text);
        let key = {
            let mut state = self.shared.state.lock().unwrap();
            if *self.shared.epoch.borrow() != self.epoch {
                return Status::Unavailable;
            }
            if sensitive {
                state.hash = Some(digest);
                return Status::Sensitive;
            }
            if !manual && !self.shared.enabled() {
                return Status::Disabled;
            }
            if !manual && state.hash == Some(digest) {
                return Status::Unchanged;
            }
            if !self.alive.load(Ordering::SeqCst)
                || self.session.connection().quinn().close_reason().is_some()
                || !self
                    .session
                    .peer_capabilities()
                    .iter()
                    .any(|c| c.name() == "clipboard" && c.version() == 1)
            {
                return Status::Unavailable;
            }
            state.clock = match state.clock.checked_add(1) {
                Some(n) => n,
                None => return Status::Failed,
            };
            let mut id = [0; 16];
            OsRng.fill_bytes(&mut id);
            let key = (state.clock, id);
            state.key = Some(key);
            state.hash = Some(digest);
            key
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(key.1, tx);
        let _cleanup = PendingGuard {
            pending: self.pending.clone(),
            id: key.1,
        };
        if self
            .session
            .send(Channel::Clipboard, &Message::Set { key, text }.encode())
            .await
            .is_err()
        {
            return Status::Unavailable;
        }
        tokio::select! {
            _=self.session.closed()=>Status::Unavailable,
            result=tokio::time::timeout(Duration::from_secs(3),rx)=>result.ok().and_then(Result::ok).unwrap_or(Status::Unavailable),
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.worker.abort();
        self.pending.lock().unwrap().clear();
    }
}
fn hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

// CCS/1 IO:clipboard/1 [0,{0:id16B,1:clock,2:text}] | [1,{0:id16B,1:status(0/1/2)}].
#[derive(PartialEq, Eq)]
pub enum Message {
    Set { key: Key, text: String },
    Ack { id: Id, status: Status },
}
impl Message {
    pub fn encode(&self) -> RawMessage {
        let uint = |v: u64| Value::Int(v.into());
        let (ty, fields) = match self {
            Self::Set { key, text } => (
                0,
                vec![
                    (0, Value::Bytes(key.1.to_vec())),
                    (1, uint(key.0)),
                    (2, Value::Text(text.clone())),
                ],
            ),
            Self::Ack { id, status } => (
                1,
                vec![
                    (0, Value::Bytes(id.to_vec())),
                    (
                        1,
                        uint(match status {
                            Status::Applied => 0,
                            Status::Refused => 1,
                            _ => 2,
                        }),
                    ),
                ],
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
        let Value::Bytes(id) = *values.get(&0)? else {
            return None;
        };
        let id: Id = id.as_slice().try_into().ok()?;
        let Value::Int(number) = *values.get(&1)? else {
            return None;
        };
        let number = u64::try_from(*number).ok()?;
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
            1 => Some(Self::Ack {
                id,
                status: match number {
                    0 => Status::Applied,
                    1 => Status::Refused,
                    2 => Status::Failed,
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
                    status: Status::Applied
                }
                .encode()
            )
            .unwrap(),
            ack
        );
    }
    #[test]
    fn wire_bounds_and_shape() {
        for text in [
            "".to_string(),
            "Zażółć 👩‍💻\nhttps://example.com/ż".into(),
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
        assert!(Message::decode(&RawMessage::new(2, Value::Map(vec![]))).is_none());
    }
}
