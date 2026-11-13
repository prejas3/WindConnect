//! CCS/1 M:notify/1, the phone's notifications shown on the PC (§9.7, WN-42).
//! IO:channel 02 `notify` (accepted with notify/1): Posted [0,{0:id16,1:app,2:package,3?:title,4?:text,5:time_ms,6?:icon8}],
//! Icon [1,{0:key8,1:png}], Removed [2,{0:id16}].
//! RULE:Icon once per key per session, before the first Posted naming it; per id at most one Posted a second (the latest wins,
//! sent when the second is up), a progress notification only at its start and end, nothing identical to what was sent;
//! Removed only for ids sent in this session. Texts and icons come from other apps: bounded here, shown as text only.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Mutex};

use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session, SessionError},
};

/// The app's label, UTF-8 bytes at most.
pub const MAX_APP: usize = 64;
/// The app's package name.
pub const MAX_PACKAGE: usize = 128;
pub const MAX_TITLE: usize = 256;
pub const MAX_TEXT: usize = 1024;
/// The app icon, PNG only, at most this many bytes (64 px a side, by the sender).
pub const MAX_ICON: usize = 16 * 1024;
/// The reply action's label (WN-44).
pub const MAX_REPLY_LABEL: usize = 64;
/// A reply's text, UTF-8 bytes at most (1 at least).
pub const MAX_REPLY: usize = 1024;
/// One `Posted` per id at most this often.
pub const WINDOW: Duration = Duration::from_secs(1);
/// Ids remembered per session (sent ones, for `Removed` and the window); the oldest go first.
pub const REMEMBERED: usize = 512;

/// One notification as the phone shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Posted {
    /// Stable per notification on the sender, meaningless elsewhere (a salted hash on Android).
    pub id: [u8; 16],
    pub app: String,
    pub package: String,
    pub title: Option<String>,
    pub text: Option<String>,
    /// When it was posted, ms since the Unix epoch.
    pub time_ms: u64,
    /// Key of the [`Message::Icon`] that goes with it.
    pub icon: Option<[u8; 8]>,
    /// The label of its reply action, when it can be replied to (WN-44).
    pub reply: Option<String>,
}

impl Posted {
    /// What the person sees: all but the time.
    fn same_content(&self, other: &Posted) -> bool {
        Posted {
            time_ms: 0,
            ..self.clone()
        } == Posted {
            time_ms: 0,
            ..other.clone()
        }
    }
}

/// On channel `notify`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Posted(Posted),
    Icon {
        key: [u8; 8],
        png: Vec<u8>,
    },
    Removed {
        id: [u8; 16],
    },
    /// Whether a [`Action::Reply`] went out to the app (WN-44).
    Replied {
        id: [u8; 16],
        ok: bool,
    },
}

/// On channel `notify-action`, from the side showing the notifications (§9.8, WN-44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Dismiss { id: [u8; 16] },
    Reply { id: [u8; 16], text: String },
}

impl Action {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::Dismiss { id } => RawMessage::new(
                0,
                Value::Map(vec![(Value::Int(0), Value::Bytes(id.to_vec()))]),
            ),
            Self::Reply { id, text } => RawMessage::new(
                1,
                Value::Map(vec![
                    (Value::Int(0), Value::Bytes(id.to_vec())),
                    (Value::Int(1), Value::Text(text.clone())),
                ]),
            ),
        }
    }

    /// `None` for an unknown type or a malformed one (an empty or too long reply: never trimmed).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields(m)?;
        let id = fixed(get(&f, 0)?)?;
        match m.ty {
            0 => Some(Self::Dismiss { id }),
            1 => match get(&f, 1)? {
                Value::Text(t) if !t.is_empty() && t.len() <= MAX_REPLY => Some(Self::Reply {
                    id,
                    text: t.clone(),
                }),
                _ => None,
            },
            _ => None,
        }
    }
}

type Fields<'a> = Vec<(u64, &'a Value)>;

/// The map's natural-number keys; `None` for a repeated or other key.
fn fields(m: &RawMessage) -> Option<Fields<'_>> {
    let Value::Map(pairs) = &m.body else {
        return None;
    };
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

/// A text field: `Some(None)` when absent, `None` when malformed or longer than `max`.
fn text(f: &Fields<'_>, key: u64, max: usize) -> Option<Option<String>> {
    match get(f, key) {
        None => Some(None),
        Some(Value::Text(t)) if t.len() <= max => Some(Some(t.clone())),
        Some(_) => None,
    }
}

fn fixed<const N: usize>(v: &Value) -> Option<[u8; N]> {
    match v {
        Value::Bytes(b) => b.as_slice().try_into().ok(),
        _ => None,
    }
}

/// PNG by its first bytes (`89 50 4E 47`), never by the sender's word.
pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47])
}

/// The key an [`Message::Icon`] goes by: the first 8 bytes of its SHA-256.
pub fn icon_key(png: &[u8]) -> [u8; 8] {
    Sha256::digest(png)[..8].try_into().expect("8 of 32 bytes")
}

impl Message {
    pub fn encode(&self) -> RawMessage {
        let t = |s: &str| Value::Text(s.to_owned());
        let map = |f: Vec<(i64, Value)>| {
            Value::Map(
                f.into_iter()
                    .map(|(k, v)| (Value::Int(k.into()), v))
                    .collect(),
            )
        };
        match self {
            Self::Posted(n) => {
                let mut f = vec![
                    (0, Value::Bytes(n.id.to_vec())),
                    (1, t(&n.app)),
                    (2, t(&n.package)),
                ];
                if let Some(s) = &n.title {
                    f.push((3, t(s)));
                }
                if let Some(s) = &n.text {
                    f.push((4, t(s)));
                }
                f.push((5, Value::Int(n.time_ms.into())));
                if let Some(k) = n.icon {
                    f.push((6, Value::Bytes(k.to_vec())));
                }
                if let Some(r) = &n.reply {
                    f.push((7, t(r)));
                }
                RawMessage::new(0, map(f))
            }
            Self::Icon { key, png } => RawMessage::new(
                1,
                map(vec![
                    (0, Value::Bytes(key.to_vec())),
                    (1, Value::Bytes(png.clone())),
                ]),
            ),
            Self::Removed { id } => RawMessage::new(2, map(vec![(0, Value::Bytes(id.to_vec()))])),
            Self::Replied { id, ok } => RawMessage::new(
                3,
                map(vec![(0, Value::Bytes(id.to_vec())), (1, Value::Bool(*ok))]),
            ),
        }
    }

    /// `None` for an unknown type or a malformed message (too long, wrong type, missing field).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields(m)?;
        match m.ty {
            0 => Some(Self::Posted(Posted {
                id: fixed(get(&f, 0)?)?,
                app: text(&f, 1, MAX_APP)??,
                package: text(&f, 2, MAX_PACKAGE)??,
                title: text(&f, 3, MAX_TITLE)?,
                text: text(&f, 4, MAX_TEXT)?,
                time_ms: match get(&f, 5)? {
                    Value::Int(n) => u64::try_from(*n).ok()?,
                    _ => return None,
                },
                icon: match get(&f, 6) {
                    None => None,
                    Some(v) => Some(fixed(v)?),
                },
                reply: text(&f, 7, MAX_REPLY_LABEL)?,
            })),
            1 => Some(Self::Icon {
                key: fixed(get(&f, 0)?)?,
                png: match get(&f, 1)? {
                    Value::Bytes(b) if b.len() <= MAX_ICON && is_png(b) => b.clone(),
                    _ => return None,
                },
            }),
            2 => Some(Self::Removed {
                id: fixed(get(&f, 0)?)?,
            }),
            3 => Some(Self::Replied {
                id: fixed(get(&f, 0)?)?,
                ok: match get(&f, 1)? {
                    Value::Bool(b) => *b,
                    _ => return None,
                },
            }),
            _ => None,
        }
    }
}

/// Routes `notify/1`: the queue gets the peer's notifications.
pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::Notify, 1, 32)
}

/// Routes `notify-action/1` (WN-44): the queue gets the peer's dismissals and replies for notifications sent here.
pub fn route_actions(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::NotifyAction, 1, 16)
}

/// Asks the peer to dismiss or reply to one of its notifications. `Ok(false)`: it doesn't take `notify-action/1`.
pub async fn act(session: &Session, action: &Action) -> Result<bool, SessionError> {
    match session.send(Channel::NotifyAction, &action.encode()).await {
        Ok(()) => Ok(true),
        Err(SessionError::CapabilityMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// What [`Throttle::posted`] says to do with a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Send,
    /// Kept as the id's latest; it goes out at this time unless replaced or removed.
    Hold(Instant),
    /// A newer one already waits: it replaced that one, whose send is already due at its time.
    Replaced,
    Drop,
}

struct Entry {
    last: Posted,
    at: Instant,
    progress: bool,
    pending: Option<(Posted, bool)>,
}

/// The per-id rules, with the clock given (testable without waiting).
#[derive(Default)]
pub struct Throttle {
    sent: HashMap<[u8; 16], Entry>,
}

impl Throttle {
    /// `progress`: the notification shows a progress bar (only its start and its end go out).
    pub fn posted(&mut self, n: &Posted, progress: bool, now: Instant) -> Decision {
        let Some(e) = self.sent.get_mut(&n.id) else {
            return Decision::Send;
        };
        if progress && e.progress {
            return Decision::Drop;
        }
        if e.last.same_content(n) && e.progress == progress {
            e.pending = None;
            return Decision::Drop;
        }
        let due = e.at + WINDOW;
        if now >= due {
            e.pending = None;
            return Decision::Send;
        }
        let had = e.pending.replace((n.clone(), progress)).is_some();
        if had {
            Decision::Replaced
        } else {
            Decision::Hold(due)
        }
    }

    /// After a send: remembered for the window, `Removed` and progress.
    pub fn sent(&mut self, n: &Posted, progress: bool, now: Instant) {
        if !self.sent.contains_key(&n.id) && self.sent.len() >= REMEMBERED {
            if let Some(oldest) = self.sent.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| *k) {
                self.sent.remove(&oldest);
            }
        }
        let pending = self.sent.remove(&n.id).and_then(|e| e.pending);
        self.sent.insert(
            n.id,
            Entry {
                last: n.clone(),
                at: now,
                progress,
                pending,
            },
        );
    }

    /// The id's held notification, when its time has come.
    pub fn due(&mut self, id: &[u8; 16], now: Instant) -> Option<(Posted, bool)> {
        let e = self.sent.get_mut(id)?;
        if now < e.at + WINDOW {
            return None;
        }
        e.pending.take()
    }

    /// Gone on the phone: true when it was sent in this session (then `Removed` goes out).
    pub fn removed(&mut self, id: &[u8; 16]) -> bool {
        self.sent.remove(id).is_some()
    }
}

/// What happened to a notification handed to [`Sender`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Written to the stream.
    Sent,
    /// Kept for the end of its window.
    Held,
    /// Nothing to send (unchanged, a progress step, not sent before).
    Dropped,
    /// The peer doesn't take `notify/1`.
    NotAccepted,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Held => "held",
            Self::Dropped => "dropped",
            Self::NotAccepted => "not-accepted",
        }
    }
}

#[derive(Default)]
struct State {
    throttle: Throttle,
    icons: HashSet<[u8; 8]>,
    /// Icon bytes of held notifications, by key.
    held_icons: HashMap<[u8; 8], Vec<u8>>,
}

/// Sends this side's notifications to the peer in one session, by the rules above.
pub struct Sender {
    session: Session,
    state: Mutex<State>,
}

impl Sender {
    pub fn new(session: Session) -> Arc<Self> {
        Arc::new(Self {
            session,
            state: Mutex::new(State::default()),
        })
    }

    /// A notification, with its app icon (PNG ≤ [`MAX_ICON`]; anything else is left out).
    pub async fn posted(
        self: &Arc<Self>,
        mut n: Posted,
        icon: Option<Vec<u8>>,
        progress: bool,
    ) -> Result<Outcome, SessionError> {
        let icon = icon.filter(|p| p.len() <= MAX_ICON && is_png(p));
        n.icon = icon.as_deref().map(icon_key);
        let mut s = self.state.lock().await;
        match s.throttle.posted(&n, progress, Instant::now()) {
            Decision::Drop => Ok(Outcome::Dropped),
            Decision::Replaced => {
                if let (Some(k), Some(p)) = (n.icon, icon) {
                    s.held_icons.insert(k, p);
                }
                Ok(Outcome::Held)
            }
            Decision::Hold(due) => {
                if let (Some(k), Some(p)) = (n.icon, icon) {
                    s.held_icons.insert(k, p);
                }
                let me = Arc::clone(self);
                let id = n.id;
                tokio::spawn(async move {
                    tokio::time::sleep_until(due.into()).await;
                    let _ = me.flush(id).await;
                });
                Ok(Outcome::Held)
            }
            Decision::Send => self.send_locked(&mut s, n, icon, progress).await,
        }
    }

    async fn flush(&self, id: [u8; 16]) -> Result<Outcome, SessionError> {
        let mut s = self.state.lock().await;
        let Some((n, progress)) = s.throttle.due(&id, Instant::now()) else {
            return Ok(Outcome::Dropped);
        };
        let icon = n.icon.and_then(|k| s.held_icons.remove(&k));
        self.send_locked(&mut s, n, icon, progress).await
    }

    async fn send_locked(
        &self,
        s: &mut State,
        n: Posted,
        icon: Option<Vec<u8>>,
        progress: bool,
    ) -> Result<Outcome, SessionError> {
        let result = async {
            if let (Some(key), Some(png)) = (n.icon, icon) {
                if !s.icons.contains(&key) {
                    self.session
                        .send(Channel::Notify, &Message::Icon { key, png }.encode())
                        .await?;
                    s.icons.insert(key);
                }
            }
            self.session
                .send(Channel::Notify, &Message::Posted(n.clone()).encode())
                .await
        }
        .await;
        match result {
            Ok(()) => {
                s.throttle.sent(&n, progress, Instant::now());
                Ok(Outcome::Sent)
            }
            Err(SessionError::CapabilityMissing(_)) => {
                // Withdrawn: what was in flight may be lost (§6 rule 5), so icons go again once it is back.
                s.icons.clear();
                Ok(Outcome::NotAccepted)
            }
            Err(e) => Err(e),
        }
    }

    /// The outcome of a reply the peer asked for (WN-44).
    pub async fn replied(&self, id: [u8; 16], ok: bool) -> Result<Outcome, SessionError> {
        let _order = self.state.lock().await;
        match self
            .session
            .send(Channel::Notify, &Message::Replied { id, ok }.encode())
            .await
        {
            Ok(()) => Ok(Outcome::Sent),
            Err(SessionError::CapabilityMissing(_)) => Ok(Outcome::NotAccepted),
            Err(e) => Err(e),
        }
    }

    /// Gone on the phone: `Removed`, only when the id went out in this session.
    pub async fn removed(&self, id: [u8; 16]) -> Result<Outcome, SessionError> {
        let mut s = self.state.lock().await;
        if !s.throttle.removed(&id) {
            return Ok(Outcome::Dropped);
        }
        match self
            .session
            .send(Channel::Notify, &Message::Removed { id }.encode())
            .await
        {
            Ok(()) => Ok(Outcome::Sent),
            Err(SessionError::CapabilityMissing(_)) => {
                s.icons.clear();
                Ok(Outcome::NotAccepted)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    fn note(text: &str) -> Posted {
        Posted {
            id: [7; 16],
            app: "Signal".into(),
            package: "org.thoughtcrime.securesms".into(),
            title: Some("Ola".into()),
            text: Some(text.into()),
            time_ms: 1_794_500_000_000,
            icon: Some([3; 8]),
            reply: None,
        }
    }

    #[test]
    fn replies_and_dismissals_round_trip() {
        let can = Posted {
            reply: Some("Odpowiedz".into()),
            ..note("x")
        };
        assert_eq!(
            Message::decode(&Message::Posted(can.clone()).encode()),
            Some(Message::Posted(can))
        );
        let long_label = Posted {
            reply: Some("x".repeat(MAX_REPLY_LABEL + 1)),
            ..note("x")
        };
        assert_eq!(Message::decode(&Message::Posted(long_label).encode()), None);
        for m in [
            Message::Replied {
                id: [7; 16],
                ok: true,
            },
            Message::Replied {
                id: [7; 16],
                ok: false,
            },
        ] {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        for a in [
            Action::Dismiss { id: [7; 16] },
            Action::Reply {
                id: [7; 16],
                text: "Będę o 19 \u{1F44D}".into(),
            },
            Action::Reply {
                id: [7; 16],
                text: "x".repeat(MAX_REPLY),
            },
        ] {
            assert_eq!(Action::decode(&a.encode()), Some(a));
        }
        let too_long = Action::Reply {
            id: [7; 16],
            text: "x".repeat(MAX_REPLY + 1),
        };
        assert_eq!(Action::decode(&too_long.encode()), None, "never trimmed");
        let empty = Action::Reply {
            id: [7; 16],
            text: String::new(),
        };
        assert_eq!(Action::decode(&empty.encode()), None);
        let mut dismiss = vec![0x15, 0x82, 0x00, 0xa1, 0x00, 0x50];
        dismiss.extend([7; 16]);
        assert_eq!(
            encode_raw_frame(
                Channel::NotifyAction,
                &Action::Dismiss { id: [7; 16] }.encode()
            )
            .unwrap(),
            dismiss
        );
        // PROTOCOL.md §9.7 / §9.8 examples.
        let mut reply = vec![0x19, 0x82, 0x01, 0xa2, 0x00, 0x50];
        reply.extend([7; 16]);
        reply.extend([0x01, 0x62, 0x6f, 0x6b]);
        let ok = Action::Reply {
            id: [7; 16],
            text: "ok".into(),
        };
        assert_eq!(
            encode_raw_frame(Channel::NotifyAction, &ok.encode()).unwrap(),
            reply
        );
        let mut replied = vec![0x17, 0x82, 0x03, 0xa2, 0x00, 0x50];
        replied.extend([7; 16]);
        replied.extend([0x01, 0xf5]);
        assert_eq!(
            encode_raw_frame(
                Channel::Notify,
                &Message::Replied {
                    id: [7; 16],
                    ok: true
                }
                .encode()
            )
            .unwrap(),
            replied
        );
    }

    #[test]
    fn round_trips() {
        let png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        for m in [
            Message::Posted(note("Będziesz o 19?")),
            Message::Posted(Posted {
                title: None,
                text: None,
                icon: None,
                ..note("")
            }),
            Message::Icon {
                key: icon_key(&png),
                png: png.clone(),
            },
            Message::Removed { id: [7; 16] },
        ] {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        let mut removed = vec![0x15, 0x82, 0x02, 0xa1, 0x00, 0x50];
        removed.extend([7; 16]);
        assert_eq!(
            encode_raw_frame(Channel::Notify, &Message::Removed { id: [7; 16] }.encode()).unwrap(),
            removed
        );
    }

    #[test]
    fn untrusted_input_is_bounded() {
        let long = |f: fn(&mut Posted)| {
            let mut n = note("x");
            f(&mut n);
            Message::decode(&Message::Posted(n).encode())
        };
        assert!(long(|n| n.text = Some("x".repeat(MAX_TEXT + 1))).is_none());
        assert!(long(|n| n.title = Some("x".repeat(MAX_TITLE + 1))).is_none());
        assert!(long(|n| n.app = "x".repeat(MAX_APP + 1)).is_none());
        assert!(long(|n| n.package = "x".repeat(MAX_PACKAGE + 1)).is_none());
        assert!(long(|n| n.text = Some("x".repeat(MAX_TEXT))).is_some());
        let jpeg = Message::Icon {
            key: [0; 8],
            png: vec![0xff, 0xd8, 0xff, 0xe0],
        };
        assert_eq!(
            Message::decode(&jpeg.encode()),
            None,
            "PNG only, by magic bytes"
        );
        let big = Message::Icon {
            key: [0; 8],
            png: [&[0x89, 0x50, 0x4e, 0x47][..], &vec![0; MAX_ICON]].concat(),
        };
        assert_eq!(Message::decode(&big.encode()), None);
        let short_id = RawMessage::new(
            2,
            Value::Map(vec![(Value::Int(0), Value::Bytes(vec![1; 15]))]),
        );
        assert_eq!(Message::decode(&short_id), None);
        let no_time = RawMessage::new(
            0,
            Value::Map(vec![
                (Value::Int(0), Value::Bytes(vec![1; 16])),
                (Value::Int(1), Value::Text("a".into())),
                (Value::Int(2), Value::Text("b".into())),
            ]),
        );
        assert_eq!(Message::decode(&no_time), None);
        assert_eq!(
            Message::decode(&RawMessage::new(9, Value::Map(vec![]))),
            None
        );
    }

    #[test]
    fn one_a_second_per_id_the_latest_wins() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut t = Throttle::default();
        assert_eq!(t.posted(&note("1"), false, t0), Decision::Send);
        t.sent(&note("1"), false, t0);
        assert_eq!(
            t.posted(&note("1"), false, ms(100)),
            Decision::Drop,
            "unchanged"
        );
        assert_eq!(
            t.posted(
                &Posted {
                    time_ms: 5,
                    ..note("1")
                },
                false,
                ms(100)
            ),
            Decision::Drop,
            "only the time changed"
        );
        assert_eq!(
            t.posted(&note("2"), false, ms(200)),
            Decision::Hold(ms(1000))
        );
        assert_eq!(t.posted(&note("3"), false, ms(300)), Decision::Replaced);
        assert_eq!(t.due(&[7; 16], ms(999)), None, "not before the window ends");
        assert_eq!(
            t.due(&[7; 16], ms(1000)),
            Some((note("3"), false)),
            "the latest"
        );
        t.sent(&note("3"), false, ms(1000));
        assert_eq!(t.due(&[7; 16], ms(3000)), None, "nothing more held");
        assert_eq!(
            t.posted(&note("4"), false, ms(2500)),
            Decision::Send,
            "a quiet second: at once"
        );
        // Another id has its own window.
        let other = Posted {
            id: [8; 16],
            ..note("x")
        };
        assert_eq!(t.posted(&other, false, ms(2500)), Decision::Send);
        // Back to what was sent while one waits: nothing goes out.
        t.sent(&note("4"), false, ms(2500));
        assert!(matches!(
            t.posted(&note("5"), false, ms(2600)),
            Decision::Hold(_)
        ));
        assert_eq!(t.posted(&note("4"), false, ms(2700)), Decision::Drop);
        assert_eq!(t.due(&[7; 16], ms(3500)), None);
    }

    #[test]
    fn progress_only_at_start_and_end() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut t = Throttle::default();
        assert_eq!(t.posted(&note("0%"), true, t0), Decision::Send);
        t.sent(&note("0%"), true, t0);
        for (i, at) in (1..30).zip(1..30) {
            assert_eq!(
                t.posted(&note(&format!("{i}%")), true, s(at)),
                Decision::Drop
            );
        }
        assert_eq!(
            t.posted(&note("Pobrano"), false, s(31)),
            Decision::Send,
            "the end"
        );
        t.sent(&note("Pobrano"), false, s(31));
        // A new progress run after the end starts again.
        assert_eq!(t.posted(&note("0%"), true, s(40)), Decision::Send);
    }

    #[test]
    fn removed_only_for_what_was_sent() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        assert!(
            !t.removed(&[7; 16]),
            "never sent (skipped, filtered or from before the session)"
        );
        t.sent(&note("a"), false, t0);
        assert!(t.removed(&[7; 16]));
        assert!(!t.removed(&[7; 16]), "once");
        // Removed while one is held: nothing more goes out for it.
        t.sent(&note("a"), false, t0);
        assert!(matches!(
            t.posted(&note("b"), false, t0 + Duration::from_millis(10)),
            Decision::Hold(_)
        ));
        assert!(t.removed(&[7; 16]));
        assert_eq!(t.due(&[7; 16], t0 + WINDOW), None);
        assert_eq!(
            t.posted(&note("c"), false, t0 + Duration::from_millis(20)),
            Decision::Send,
            "new after removal"
        );
    }

    #[test]
    fn remembers_a_bounded_number() {
        let t0 = Instant::now();
        let mut t = Throttle::default();
        for i in 0..(REMEMBERED as u32 + 10) {
            let mut id = [0; 16];
            id[..4].copy_from_slice(&i.to_be_bytes());
            t.sent(
                &Posted { id, ..note("x") },
                false,
                t0 + Duration::from_millis(i.into()),
            );
        }
        assert_eq!(t.sent.len(), REMEMBERED);
        assert!(!t.removed(&[0; 16]), "the oldest went first");
    }
}
