//! CCS/1 M:media/1 + media-control/1, a player shown and controlled on the other side (§9.6, WN-46).
//! IO:channel 03 `media` (accepted with media/1): Now [0,{...}], Art [1,{0:key8,1:image}], Gone [2,{}];
//! channel 09 `media-control` (accepted with media-control/1): Command [0,{0:action,1?:position_ms}].
//! RULE:Now only on a change (never periodic; the receiver extrapolates progress); Art once per key. Texts and images come
//! from other apps: bounded here, shown as text only by the receiver; the image type comes from its magic bytes.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session, SessionError},
};

/// Title, artist, album: UTF-8 bytes at most.
pub const MAX_TEXT: usize = 256;
/// The player's label.
pub const MAX_APP: usize = 64;
/// Cover art, JPEG or PNG, at most this many bytes (and 256 px a side, by the sender).
pub const MAX_ART: usize = 96 * 1024;
/// A position further than this from the extrapolated one is a change (a seek or a jump).
pub const JUMP_MS: u64 = 2_000;

/// What the player allows, bits of [`Now::actions`].
pub mod allow {
    pub const PLAY: u64 = 1;
    pub const PAUSE: u64 = 2;
    pub const NEXT: u64 = 4;
    pub const PREVIOUS: u64 = 8;
    pub const SEEK: u64 = 16;
}

/// The sender's player now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Now {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub app: String,
    pub duration_ms: Option<u64>,
    /// At the time of sending.
    pub position_ms: u64,
    pub playing: bool,
    /// 1000 = normal speed (our CBOR has no floats).
    pub speed_permille: u64,
    /// [`allow`] bits.
    pub actions: u64,
    /// Key of the [`Message::Art`] that goes with it.
    pub art: Option<[u8; 8]>,
}

impl Now {
    /// The position `elapsed` after this was sent: moving only while playing, at its speed, up to the duration.
    pub fn extrapolate(&self, elapsed: Duration) -> u64 {
        let moved = if self.playing {
            (elapsed.as_millis() as u64).saturating_mul(self.speed_permille) / 1000
        } else {
            0
        };
        let at = self.position_ms.saturating_add(moved);
        self.duration_ms.map_or(at, |d| at.min(d))
    }

    /// Equal but for the position, and the position where extrapolation puts it.
    fn same_course(&self, newer: &Now, elapsed: Duration) -> bool {
        let expected = self.extrapolate(elapsed);
        Now {
            position_ms: 0,
            ..self.clone()
        } == Now {
            position_ms: 0,
            ..newer.clone()
        } && expected.abs_diff(newer.position_ms) <= JUMP_MS
    }
}

/// On channel `media`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Now(Now),
    Art { key: [u8; 8], image: Vec<u8> },
    Gone,
}

/// What to do with the receiver's player (channel `media-control`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Seek(u64),
}

impl Action {
    /// The [`allow`] bit it needs (toggle: play or pause).
    pub fn allowed_by(self, actions: u64) -> bool {
        match self {
            Self::Play => actions & allow::PLAY != 0,
            Self::Pause => actions & allow::PAUSE != 0,
            Self::Toggle => actions & (allow::PLAY | allow::PAUSE) != 0,
            Self::Next => actions & allow::NEXT != 0,
            Self::Previous => actions & allow::PREVIOUS != 0,
            Self::Seek(_) => actions & allow::SEEK != 0,
        }
    }

    pub fn encode(self) -> RawMessage {
        let code = match self {
            Self::Play => 0,
            Self::Pause => 1,
            Self::Toggle => 2,
            Self::Next => 3,
            Self::Previous => 4,
            Self::Seek(_) => 5,
        };
        let mut fields = vec![(Value::Int(0), Value::Int(code))];
        if let Self::Seek(at) = self {
            fields.push((Value::Int(1), Value::Int(at.into())));
        }
        RawMessage::new(0, Value::Map(fields))
    }

    pub fn decode(m: &RawMessage) -> Option<Self> {
        if m.ty != 0 {
            return None;
        }
        let f = fields(m)?;
        Some(match uint(&f, 0)? {
            0 => Self::Play,
            1 => Self::Pause,
            2 => Self::Toggle,
            3 => Self::Next,
            4 => Self::Previous,
            5 => Self::Seek(uint(&f, 1)?),
            _ => return None,
        })
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

fn uint(f: &Fields<'_>, key: u64) -> Option<u64> {
    match get(f, key)? {
        Value::Int(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// A text field: `Some(None)` when absent, `None` when malformed or longer than `max`.
fn text(f: &Fields<'_>, key: u64, max: usize) -> Option<Option<String>> {
    match get(f, key) {
        None => Some(None),
        Some(Value::Text(t)) if t.len() <= max => Some(Some(t.clone())),
        Some(_) => None,
    }
}

fn key8(v: &Value) -> Option<[u8; 8]> {
    match v {
        Value::Bytes(b) => b.as_slice().try_into().ok(),
        _ => None,
    }
}

impl Message {
    pub fn encode(&self) -> RawMessage {
        let t = |s: &str| Value::Text(s.to_owned());
        let u = |n: u64| Value::Int(n.into());
        match self {
            Self::Now(n) => {
                let mut f = vec![(0, t(&n.title))];
                if let Some(a) = &n.artist {
                    f.push((1, t(a)));
                }
                if let Some(a) = &n.album {
                    f.push((2, t(a)));
                }
                f.push((3, t(&n.app)));
                if let Some(d) = n.duration_ms {
                    f.push((4, u(d)));
                }
                f.push((5, u(n.position_ms)));
                f.push((6, Value::Bool(n.playing)));
                f.push((7, u(n.speed_permille)));
                f.push((8, u(n.actions)));
                if let Some(k) = n.art {
                    f.push((9, Value::Bytes(k.to_vec())));
                }
                RawMessage::new(
                    0,
                    Value::Map(f.into_iter().map(|(k, v)| (Value::Int(k), v)).collect()),
                )
            }
            Self::Art { key, image } => RawMessage::new(
                1,
                Value::Map(vec![
                    (Value::Int(0), Value::Bytes(key.to_vec())),
                    (Value::Int(1), Value::Bytes(image.clone())),
                ]),
            ),
            Self::Gone => RawMessage::new(2, Value::Map(vec![])),
        }
    }

    /// `None` for an unknown type or a malformed message (too long, wrong type, missing field).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let f = fields(m)?;
        match m.ty {
            0 => {
                let playing = match get(&f, 6)? {
                    Value::Bool(b) => *b,
                    _ => return None,
                };
                let art = match get(&f, 9) {
                    None => None,
                    Some(v) => Some(key8(v)?),
                };
                Some(Self::Now(Now {
                    title: text(&f, 0, MAX_TEXT)??,
                    artist: text(&f, 1, MAX_TEXT)?,
                    album: text(&f, 2, MAX_TEXT)?,
                    app: text(&f, 3, MAX_APP)??,
                    duration_ms: match get(&f, 4) {
                        None => None,
                        Some(_) => Some(uint(&f, 4)?),
                    },
                    position_ms: uint(&f, 5)?,
                    playing,
                    speed_permille: uint(&f, 7)?,
                    actions: uint(&f, 8)?,
                    art,
                }))
            }
            1 => {
                let key = key8(get(&f, 0)?)?;
                let image = match get(&f, 1)? {
                    Value::Bytes(b) if b.len() <= MAX_ART && image_mime(b).is_some() => b.clone(),
                    _ => return None,
                };
                Some(Self::Art { key, image })
            }
            2 => Some(Self::Gone),
            _ => None,
        }
    }
}

/// The image's type from its first bytes (JPEG `FF D8 FF`, PNG `89 50 4E 47`), never from the sender's word.
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]) {
        Some("image/png")
    } else {
        None
    }
}

/// The key an [`Message::Art`] goes by: the first 8 bytes of its SHA-256.
pub fn art_key(image: &[u8]) -> [u8; 8] {
    Sha256::digest(image)[..8]
        .try_into()
        .expect("8 of 32 bytes")
}

/// Routes `media/1`: the queue gets the peer's Now/Art/Gone.
pub fn route_player(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::Media, 1, 8)
}

/// Routes `media-control/1`: the queue gets the peer's commands for players here.
pub fn route_control(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::MediaControl, 1, 8)
}

/// Asks the peer's player to do `action` (it ignores what its `actions` don't allow).
pub async fn command(session: &Session, action: Action) -> Result<(), SessionError> {
    session.send(Channel::MediaControl, &action.encode()).await
}

#[derive(Default)]
struct Sent {
    now: Option<(Now, Instant)>,
    art: Option<[u8; 8]>,
    gone: bool,
}

/// Sends this side's player to the peer, only on a change.
#[derive(Default)]
pub struct Sender {
    sent: Mutex<Sent>,
}

impl Sender {
    /// Sends `now` (and `art` first, when its key is new) unless only its position moved as expected.
    /// `Ok(false)`: nothing to send, or the peer doesn't take `media/1`.
    pub async fn now(
        &self,
        session: &Session,
        mut now: Now,
        art: Option<&[u8]>,
    ) -> Result<bool, SessionError> {
        now.art = art.map(art_key);
        let (send_art, changed) = {
            let s = self.sent.lock().unwrap();
            let changed = !matches!(&s.now, Some((last, at)) if !s.gone && last.same_course(&now, at.elapsed()));
            (now.art.is_some() && s.art != now.art, changed)
        };
        if !changed && !send_art {
            return Ok(false);
        }
        let result = async {
            if send_art {
                let image = art.expect("a key means an image").to_vec();
                session
                    .send(
                        Channel::Media,
                        &Message::Art {
                            key: now.art.unwrap(),
                            image,
                        }
                        .encode(),
                    )
                    .await?;
            }
            session
                .send(Channel::Media, &Message::Now(now.clone()).encode())
                .await
        }
        .await;
        match result {
            Ok(()) => {
                let mut s = self.sent.lock().unwrap();
                if send_art {
                    s.art = now.art;
                }
                s.now = Some((now, Instant::now()));
                s.gone = false;
                Ok(true)
            }
            Err(SessionError::CapabilityMissing(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Forgets what was sent: the next `now` goes out even if unchanged (the peer took `media/1` back on).
    pub fn reset(&self) {
        *self.sent.lock().unwrap() = Sent::default();
    }

    /// No player any more (sent once).
    pub async fn gone(&self, session: &Session) -> Result<bool, SessionError> {
        if self.sent.lock().unwrap().gone {
            return Ok(false);
        }
        match session.send(Channel::Media, &Message::Gone.encode()).await {
            Ok(()) => {
                let mut s = self.sent.lock().unwrap();
                s.gone = true;
                s.now = None;
                Ok(true)
            }
            Err(SessionError::CapabilityMissing(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    pub(crate) fn song() -> Now {
        Now {
            title: "Kaonashi".into(),
            artist: Some("Daria Zawiałow".into()), // i18n: data
            album: None,
            app: "Spotify".into(),
            duration_ms: Some(200_000),
            position_ms: 30_000,
            playing: true,
            speed_permille: 1000,
            actions: allow::PLAY | allow::PAUSE | allow::NEXT | allow::PREVIOUS | allow::SEEK,
            art: Some([1; 8]),
        }
    }

    #[test]
    fn round_trips() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
        for m in [
            Message::Now(song()),
            Message::Now(Now {
                artist: None,
                duration_ms: None,
                art: None,
                ..song()
            }),
            Message::Art {
                key: art_key(&jpeg),
                image: jpeg.clone(),
            },
            Message::Gone,
        ] {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        for a in [
            Action::Play,
            Action::Pause,
            Action::Toggle,
            Action::Next,
            Action::Previous,
            Action::Seek(61_000),
        ] {
            assert_eq!(Action::decode(&a.encode()), Some(a));
        }
        assert_eq!(
            encode_raw_frame(Channel::Media, &Message::Gone.encode()).unwrap(),
            [0x03, 0x82, 0x02, 0xa0]
        );
        assert_eq!(
            encode_raw_frame(Channel::MediaControl, &Action::Next.encode()).unwrap(),
            [0x05, 0x82, 0x00, 0xa1, 0x00, 0x03]
        );
        assert_eq!(
            encode_raw_frame(Channel::MediaControl, &Action::Seek(61_000).encode()).unwrap(),
            [0x09, 0x82, 0x00, 0xa2, 0x00, 0x05, 0x01, 0x19, 0xee, 0x48]
        );
    }

    #[test]
    fn untrusted_input_is_bounded() {
        let too_long = Message::Now(Now {
            title: "x".repeat(MAX_TEXT + 1),
            ..song()
        });
        assert_eq!(Message::decode(&too_long.encode()), None);
        let app = Message::Now(Now {
            app: "x".repeat(MAX_APP + 1),
            ..song()
        });
        assert_eq!(Message::decode(&app.encode()), None);
        let gif = Message::Art {
            key: [0; 8],
            image: b"GIF89a....".to_vec(),
        };
        assert_eq!(
            Message::decode(&gif.encode()),
            None,
            "only JPEG/PNG by magic bytes"
        );
        let big = Message::Art {
            key: [0; 8],
            image: [&[0xff, 0xd8, 0xff][..], &vec![0; MAX_ART]].concat(),
        };
        assert_eq!(Message::decode(&big.encode()), None);
        assert_eq!(
            Action::decode(&RawMessage::new(
                0,
                Value::Map(vec![(Value::Int(0), Value::Int(9))])
            )),
            None
        );
        assert_eq!(
            Action::decode(&RawMessage::new(
                0,
                Value::Map(vec![(Value::Int(0), Value::Int(5))])
            )),
            None,
            "seek needs a position"
        );
        assert_eq!(
            image_mime(&[0x89, 0x50, 0x4e, 0x47, 0x0d]),
            Some("image/png")
        );
    }

    #[test]
    fn extrapolation_and_jumps() {
        let s = song();
        assert_eq!(s.extrapolate(Duration::from_secs(10)), 40_000);
        assert_eq!(
            Now {
                playing: false,
                ..s.clone()
            }
            .extrapolate(Duration::from_secs(10)),
            30_000
        );
        assert_eq!(
            Now {
                speed_permille: 1500,
                ..s.clone()
            }
            .extrapolate(Duration::from_secs(10)),
            45_000
        );
        assert_eq!(
            s.extrapolate(Duration::from_secs(1_000)),
            200_000,
            "never past the end"
        );
        let later = Now {
            position_ms: 41_000,
            ..s.clone()
        };
        assert!(
            s.same_course(&later, Duration::from_secs(10)),
            "1 s off: no change"
        );
        let seek = Now {
            position_ms: 90_000,
            ..s.clone()
        };
        assert!(
            !s.same_course(&seek, Duration::from_secs(10)),
            "a seek is a change"
        );
        assert!(!s.same_course(
            &Now {
                playing: false,
                ..later
            },
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn actions_gate_commands() {
        let a = allow::PLAY | allow::NEXT;
        assert!(
            Action::Play.allowed_by(a)
                && Action::Next.allowed_by(a)
                && Action::Toggle.allowed_by(a)
        );
        assert!(
            !Action::Pause.allowed_by(a)
                && !Action::Previous.allowed_by(a)
                && !Action::Seek(1).allowed_by(a)
        );
    }
}
