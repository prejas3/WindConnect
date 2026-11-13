//! Messages of the `control` (§7) and `input` (§8) channels and their
//! `[type, body]` form (§4).

use std::fmt;

use super::cbor::Value;
use super::{Channel, ErrorCode, FrameError};

type Result<T> = std::result::Result<T, FrameError>;

/// Most capabilities one list may hold (§6).
pub const MAX_CAPS: usize = 64;
/// Longest `message` text of `Error` and `Goodbye`, in UTF-8 bytes.
pub const MAX_MESSAGE_TEXT: usize = 256;
const NAME_MAX: usize = 64;
const APP_MAX: usize = 32;
const CAP_NAME_MAX: usize = 24;

const WRONG_TYPE: FrameError = FrameError::Malformed("a field has the wrong type");

/// A message on a known channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Control(ControlMessage),
    Input(InputMessage),
}

impl Message {
    /// The channel it travels on.
    pub fn channel(&self) -> Channel {
        match self {
            Self::Control(_) => Channel::Control,
            Self::Input(_) => Channel::Input,
        }
    }

    /// Its `type` number within its channel.
    pub fn type_id(&self) -> u64 {
        match self {
            Self::Control(m) => m.type_id(),
            Self::Input(m) => m.type_id(),
        }
    }
}

/// Messages of the `control` channel (§7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMessage {
    Hello(Hello),
    HelloAck(Hello),
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    /// The full new list of what the sender accepts.
    Capabilities {
        caps: Vec<Capability>,
    },
    Error(ErrorMessage),
    Goodbye(Goodbye),
}

impl ControlMessage {
    pub fn type_id(&self) -> u64 {
        match self {
            Self::Hello(_) => 0,
            Self::HelloAck(_) => 1,
            Self::Ping { .. } => 2,
            Self::Pong { .. } => 3,
            Self::Capabilities { .. } => 4,
            Self::Error(_) => 5,
            Self::Goodbye(_) => 6,
        }
    }
}

/// The body of `Hello` and `HelloAck` (§7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub version: Version,
    /// Display name, 1–64 UTF-8 bytes. Not proof of identity.
    pub name: String,
    pub kind: DeviceKind,
    /// What the sender accepts (§6); may be empty.
    pub caps: Vec<Capability>,
    /// Application version, 1–32 UTF-8 bytes.
    pub app: Option<String>,
}

/// A `major.minor` protocol version (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
}

/// Kind of device in `Hello`; an unknown number reads as a computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Computer,
    Phone,
    Tablet,
}

impl DeviceKind {
    fn code(self) -> u64 {
        match self {
            Self::Computer => 0,
            Self::Phone => 1,
            Self::Tablet => 2,
        }
    }

    fn from_code(code: u64) -> Self {
        match code {
            1 => Self::Phone,
            2 => Self::Tablet,
            _ => Self::Computer,
        }
    }
}

/// A capability `name/N` (§6): `name` of 1–24 bytes from `a-z`, `0-9`
/// and `-`; `N` a positive decimal without leading zeros.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Capability {
    name: String,
    version: u64,
}

impl Capability {
    /// `None` if `name` or `version` is outside the grammar.
    pub fn new(name: &str, version: u64) -> Option<Self> {
        let valid_name = (1..=CAP_NAME_MAX).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        (valid_name && version >= 1).then(|| Self {
            name: name.to_owned(),
            version,
        })
    }

    /// Parses `name/N`; `None` if it is outside the grammar.
    pub fn parse(text: &str) -> Option<Self> {
        let (name, n) = text.split_once('/')?;
        if n.is_empty() || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Self::new(name, n.parse().ok()?)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.name, self.version)
    }
}

/// `Error` (§7.6): a problem that does not close the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorMessage {
    pub code: ErrorCode,
    /// For logs, at most 256 UTF-8 bytes.
    pub message: Option<String>,
    /// Channel of the message it is about.
    pub channel: Option<u64>,
    /// Type of the message it is about.
    pub ty: Option<u64>,
}

/// `Goodbye` (§7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goodbye {
    pub code: ErrorCode,
    /// At most 256 UTF-8 bytes.
    pub message: Option<String>,
}

/// Messages of the `input` channel (§8.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMessage {
    PointerMotion(Motion),
    Scroll(Motion),
    PointerButton(PointerButton),
    InputSession(InputSession),
    Gesture(InputGesture),
}

impl InputMessage {
    pub fn type_id(&self) -> u64 {
        match self {
            Self::PointerMotion(_) => 0,
            Self::Scroll(_) => 1,
            Self::PointerButton(_) => 2,
            Self::InputSession(_) => 3,
            Self::Gesture(_) => 4,
        }
    }
}

/// The body of `PointerMotion` and `Scroll`: cumulative state (§8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Motion {
    pub seq: u64,
    pub x: i64,
    pub y: i64,
    /// Sender's monotonic time, µs.
    pub t: Option<u64>,
    /// Session.
    pub s: u32,
    /// Epoch.
    pub b: u64,
}

/// `PointerButton` (§8.8): a button transition with the full state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerButton {
    pub seq: u64,
    pub button: Button,
    pub down: bool,
    pub x: i64,
    pub y: i64,
    /// Epoch after this transition.
    pub b: u64,
    pub sx: i64,
    pub sy: i64,
}

/// CCS/1 input/2:fixed native gestures only; no text, keycodes or executable payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    TaskView,
    ShowDesktop,
    PreviousApp,
    NextApp,
    Other(u64),
}
impl Gesture {
    fn code(self) -> u64 {
        match self {
            Self::TaskView => 0,
            Self::ShowDesktop => 1,
            Self::PreviousApp => 2,
            Self::NextApp => 3,
            Self::Other(n) => n,
        }
    }
    fn from_code(n: u64) -> Self {
        match n {
            0 => Self::TaskView,
            1 => Self::ShowDesktop,
            2 => Self::PreviousApp,
            3 => Self::NextApp,
            n => Self::Other(n),
        }
    }
}
/// CCS/1 FLOW:reliable epoch barrier applies cumulative motion before executing the gesture once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputGesture {
    pub seq: u64,
    pub gesture: Gesture,
    pub x: i64,
    pub y: i64,
    pub b: u64,
    pub sx: i64,
    pub sy: i64,
}

/// A pointer button. Decoding yields [`Button::Other`] only for numbers
/// above 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
    Other(u64),
}

impl Button {
    fn code(self) -> u64 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Middle => 2,
            Self::Other(n) => n,
        }
    }

    fn from_code(code: u64) -> Self {
        match code {
            0 => Self::Left,
            1 => Self::Right,
            2 => Self::Middle,
            n => Self::Other(n),
        }
    }
}

/// `InputSession` (§8.2), the first frame of every `input` stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputSession {
    pub s: u32,
    /// Number of the receiver's announcement the session is based on.
    pub g: u64,
}

// ---- decoding -------------------------------------------------------------

/// Reads a decoded payload `[type, body]` as a message of `channel`.
pub(super) fn decode(channel: Channel, payload: Value) -> Result<Message> {
    let Value::Array(items) = payload else {
        return Err(FrameError::Malformed("payload is not a [type, body] array"));
    };
    let [ty, body]: [Value; 2] = items
        .try_into()
        .map_err(|_| FrameError::Malformed("payload is not a [type, body] array"))?;
    let ty = as_u64(&ty).ok_or(FrameError::Malformed(
        "message type is not a natural number",
    ))?;
    let mut body = Body::new(body)?;
    let unknown = FrameError::UnknownMessage { channel, ty };
    match channel {
        Channel::Control => Ok(Message::Control(match ty {
            0 => ControlMessage::Hello(body.hello()?),
            1 => ControlMessage::HelloAck(body.hello()?),
            2 => ControlMessage::Ping {
                nonce: body.u64(0)?,
            },
            3 => ControlMessage::Pong {
                nonce: body.u64(0)?,
            },
            4 => ControlMessage::Capabilities {
                caps: body.caps(0)?,
            },
            5 => ControlMessage::Error(ErrorMessage {
                code: ErrorCode::from_code(body.u64(0)?),
                message: body.opt_text(1, 0, MAX_MESSAGE_TEXT)?,
                channel: body.opt_u64(2)?,
                ty: body.opt_u64(3)?,
            }),
            6 => ControlMessage::Goodbye(Goodbye {
                code: ErrorCode::from_code(body.u64(0)?),
                message: body.opt_text(1, 0, MAX_MESSAGE_TEXT)?,
            }),
            _ => return Err(unknown),
        })),
        Channel::Input => Ok(Message::Input(match ty {
            0 => InputMessage::PointerMotion(body.motion()?),
            1 => InputMessage::Scroll(body.motion()?),
            2 => InputMessage::PointerButton(PointerButton {
                seq: body.u64(0)?,
                button: Button::from_code(body.u64(1)?),
                down: body.bool(2)?,
                x: body.i64(3)?,
                y: body.i64(4)?,
                b: body.u64(5)?,
                sx: body.i64(6)?,
                sy: body.i64(7)?,
            }),
            3 => InputMessage::InputSession(InputSession {
                s: body.u32(0)?,
                g: body.u64(1)?,
            }),
            4 => InputMessage::Gesture(InputGesture {
                seq: body.u64(0)?,
                gesture: Gesture::from_code(body.u64(1)?),
                x: body.i64(2)?,
                y: body.i64(3)?,
                b: body.u64(4)?,
                sx: body.i64(5)?,
                sy: body.i64(6)?,
            }),
            _ => return Err(unknown),
        })),
        // Feature channels have no message types in v1.0 (§9).
        _ => Err(unknown),
    }
}

fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Int(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// A body map with natural-number keys. Unknown keys are left unread.
struct Body(Vec<(u64, Value)>);

impl Body {
    fn new(body: Value) -> Result<Self> {
        let Value::Map(pairs) = body else {
            return Err(FrameError::Malformed("message body is not a map"));
        };
        pairs
            .into_iter()
            .map(|(k, v)| {
                as_u64(&k)
                    .map(|k| (k, v))
                    .ok_or(FrameError::Malformed("body key is not a natural number"))
            })
            .collect::<Result<_>>()
            .map(Self)
    }

    fn take(&mut self, key: u64) -> Option<Value> {
        let i = self.0.iter().position(|(k, _)| *k == key)?;
        Some(self.0.swap_remove(i).1)
    }

    fn required(&mut self, key: u64) -> Result<Value> {
        self.take(key)
            .ok_or(FrameError::Malformed("a required field is missing"))
    }

    fn u64(&mut self, key: u64) -> Result<u64> {
        as_u64(&self.required(key)?).ok_or(WRONG_TYPE)
    }

    fn opt_u64(&mut self, key: u64) -> Result<Option<u64>> {
        self.take(key)
            .map(|v| as_u64(&v).ok_or(WRONG_TYPE))
            .transpose()
    }

    fn u32(&mut self, key: u64) -> Result<u32> {
        u32::try_from(self.u64(key)?).map_err(|_| WRONG_TYPE)
    }

    fn i64(&mut self, key: u64) -> Result<i64> {
        match self.required(key)? {
            Value::Int(n) => i64::try_from(n).map_err(|_| WRONG_TYPE),
            _ => Err(WRONG_TYPE),
        }
    }

    fn bool(&mut self, key: u64) -> Result<bool> {
        match self.required(key)? {
            Value::Bool(b) => Ok(b),
            _ => Err(WRONG_TYPE),
        }
    }

    fn opt_text(&mut self, key: u64, min: usize, max: usize) -> Result<Option<String>> {
        self.take(key).map(|v| text(v, min, max)).transpose()
    }

    fn text(&mut self, key: u64, min: usize, max: usize) -> Result<String> {
        text(self.required(key)?, min, max)
    }

    fn caps(&mut self, key: u64) -> Result<Vec<Capability>> {
        let Value::Array(items) = self.required(key)? else {
            return Err(WRONG_TYPE);
        };
        if items.len() > MAX_CAPS {
            return Err(FrameError::Malformed("more than 64 capabilities"));
        }
        let mut caps: Vec<Capability> = Vec::with_capacity(items.len());
        for item in items {
            let Value::Text(t) = item else {
                return Err(WRONG_TYPE);
            };
            let cap =
                Capability::parse(&t).ok_or(FrameError::Malformed("capability is not name/N"))?;
            if caps.contains(&cap) {
                return Err(FrameError::Malformed("repeated capability"));
            }
            caps.push(cap);
        }
        Ok(caps)
    }

    fn hello(&mut self) -> Result<Hello> {
        let Value::Array(version) = self.required(0)? else {
            return Err(WRONG_TYPE);
        };
        let [major, minor] = version.as_slice() else {
            return Err(WRONG_TYPE);
        };
        Ok(Hello {
            version: Version {
                major: as_u64(major).ok_or(WRONG_TYPE)?,
                minor: as_u64(minor).ok_or(WRONG_TYPE)?,
            },
            name: self.text(1, 1, NAME_MAX)?,
            kind: DeviceKind::from_code(self.u64(2)?),
            caps: self.caps(3)?,
            app: self.opt_text(4, 1, APP_MAX)?,
        })
    }

    fn motion(&mut self) -> Result<Motion> {
        Ok(Motion {
            seq: self.u64(0)?,
            x: self.i64(1)?,
            y: self.i64(2)?,
            t: self.opt_u64(3)?,
            s: self.u32(4)?,
            b: self.u64(5)?,
        })
    }
}

fn text(v: Value, min: usize, max: usize) -> Result<String> {
    match v {
        Value::Text(t) if (min..=max).contains(&t.len()) => Ok(t),
        Value::Text(_) => Err(FrameError::Malformed("text outside its length limits")),
        _ => Err(WRONG_TYPE),
    }
}

// ---- encoding -------------------------------------------------------------

/// The payload `[type, body]` of `msg`, after checking its fields against
/// the limits a receiver enforces.
pub(super) fn encode(msg: &Message) -> Result<Value> {
    let mut b = Fields::default();
    match msg {
        Message::Control(m) => match m {
            ControlMessage::Hello(h) | ControlMessage::HelloAck(h) => {
                b.put(
                    0,
                    Value::Array(vec![uint(h.version.major), uint(h.version.minor)]),
                );
                b.text(1, &h.name, 1, NAME_MAX)?;
                b.put(2, uint(h.kind.code()));
                b.caps(3, &h.caps)?;
                if let Some(app) = &h.app {
                    b.text(4, app, 1, APP_MAX)?;
                }
            }
            ControlMessage::Ping { nonce } | ControlMessage::Pong { nonce } => {
                b.put(0, uint(*nonce));
            }
            ControlMessage::Capabilities { caps } => b.caps(0, caps)?,
            ControlMessage::Error(e) => {
                b.put(0, uint(e.code.code()));
                if let Some(m) = &e.message {
                    b.text(1, m, 0, MAX_MESSAGE_TEXT)?;
                }
                if let Some(c) = e.channel {
                    b.put(2, uint(c));
                }
                if let Some(t) = e.ty {
                    b.put(3, uint(t));
                }
            }
            ControlMessage::Goodbye(g) => {
                b.put(0, uint(g.code.code()));
                if let Some(m) = &g.message {
                    b.text(1, m, 0, MAX_MESSAGE_TEXT)?;
                }
            }
        },
        Message::Input(m) => match m {
            InputMessage::PointerMotion(v) | InputMessage::Scroll(v) => {
                b.put(0, uint(v.seq));
                b.put(1, int(v.x));
                b.put(2, int(v.y));
                if let Some(t) = v.t {
                    b.put(3, uint(t));
                }
                b.put(4, uint(v.s.into()));
                b.put(5, uint(v.b));
            }
            InputMessage::PointerButton(p) => {
                b.put(0, uint(p.seq));
                b.put(1, uint(p.button.code()));
                b.put(2, Value::Bool(p.down));
                b.put(3, int(p.x));
                b.put(4, int(p.y));
                b.put(5, uint(p.b));
                b.put(6, int(p.sx));
                b.put(7, int(p.sy));
            }
            InputMessage::InputSession(s) => {
                b.put(0, uint(s.s.into()));
                b.put(1, uint(s.g));
            }
            InputMessage::Gesture(g) => {
                b.put(0, uint(g.seq));
                b.put(1, uint(g.gesture.code()));
                b.put(2, int(g.x));
                b.put(3, int(g.y));
                b.put(4, uint(g.b));
                b.put(5, int(g.sx));
                b.put(6, int(g.sy));
            }
        },
    }
    Ok(Value::Array(vec![uint(msg.type_id()), Value::Map(b.0)]))
}

fn uint(n: u64) -> Value {
    Value::Int(n.into())
}

fn int(n: i64) -> Value {
    Value::Int(n.into())
}

#[derive(Default)]
struct Fields(Vec<(Value, Value)>);

impl Fields {
    fn put(&mut self, key: u64, v: Value) {
        self.0.push((uint(key), v));
    }

    fn text(&mut self, key: u64, t: &str, min: usize, max: usize) -> Result<()> {
        if !(min..=max).contains(&t.len()) {
            return Err(FrameError::Malformed("text outside its length limits"));
        }
        self.put(key, Value::Text(t.to_owned()));
        Ok(())
    }

    fn caps(&mut self, key: u64, caps: &[Capability]) -> Result<()> {
        if caps.len() > MAX_CAPS {
            return Err(FrameError::Malformed("more than 64 capabilities"));
        }
        if caps.iter().enumerate().any(|(i, c)| caps[..i].contains(c)) {
            return Err(FrameError::Malformed("repeated capability"));
        }
        let list = caps.iter().map(|c| Value::Text(c.to_string())).collect();
        self.put(key, Value::Array(list));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_grammar() {
        let c = Capability::parse("clipboard/1").unwrap();
        assert_eq!((c.name(), c.version()), ("clipboard", 1));
        assert_eq!(c.to_string(), "clipboard/1");
        assert!(Capability::parse("a-b-9/12").is_some());
        assert!(Capability::parse("abcdefghijklmnopqrstuvwx/1").is_some());
        for bad in [
            "",
            "clipboard",
            "clipboard/",
            "/1",
            "clipboard/0",
            "clipboard/01",
            "Clipboard/1",
            "clip_board/1",
            "clipboard/1/2",
            "clipboard/+1",
            "clipboard/ 1",
            "abcdefghijklmnopqrstuvwxy/1",
            "clipboard/99999999999999999999999",
        ] {
            assert!(Capability::parse(bad).is_none(), "{bad:?} accepted");
        }
        assert!(Capability::new("files", 0).is_none());
    }
}
