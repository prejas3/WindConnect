//! `actions/1` messages (PROTOCOL.md §9.2) and their `[type, body]` form.
//!
//! Type numbers are per direction: the requester writes [`Request`], the
//! device that runs actions answers with [`Reply`] on the same stream.

use crate::protocol::cbor::Value;
use crate::protocol::{FrameError, RawMessage};

type Result<T> = std::result::Result<T, FrameError>;

/// Longest action id, in bytes.
pub const MAX_ID: usize = 64;
/// Longest `url`, in bytes.
pub const MAX_URL: usize = 2048;
/// Longest label, in UTF-8 bytes.
pub const MAX_LABEL: usize = 64;
/// Most actions in one `Actions` reply.
pub const MAX_LISTED: usize = 64;

const WRONG: FrameError =
    FrameError::Malformed("an actions field is missing or has the wrong type");

/// Whether `id` is an action id: 1–64 bytes of `a-z`, `0-9` and `-`.
pub fn is_action_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// From the requester; the first and only message of its direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Type 0: run action `action`; `url` only for `open-url`.
    Run { action: String, url: Option<String> },
    /// Type 1: which actions would run here now.
    List,
}

/// How a `Run` ended, from the side that runs actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 0: the action ran (for `sleep`: is about to).
    Done,
    /// 1: not on this device's allowlist now (unknown, switched off, or
    /// not possible on this device).
    Refused,
    /// 2: the argument is missing, not allowed, or not accepted (`url`).
    BadArgument,
    /// 3: allowed, but running it failed.
    Failed,
    /// 4: too many requests just now; try later.
    Busy,
}

impl Status {
    pub fn code(self) -> u64 {
        match self {
            Self::Done => 0,
            Self::Refused => 1,
            Self::BadArgument => 2,
            Self::Failed => 3,
            Self::Busy => 4,
        }
    }

    /// An unknown code reads as `Failed`.
    pub fn from_code(code: u64) -> Self {
        match code {
            0 => Self::Done,
            1 => Self::Refused,
            2 => Self::BadArgument,
            4 => Self::Busy,
            _ => Self::Failed,
        }
    }
}

/// An action offered by the device that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub label: String,
}

/// From the side that runs actions, on the way back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Type 0: the answer to `Run`.
    Result(Status),
    /// Type 1: the answer to `List`.
    Actions(Vec<Listed>),
}

fn uint(n: u64) -> Value {
    Value::Int(n.into())
}

fn map(fields: Vec<(u64, Value)>) -> Value {
    Value::Map(fields.into_iter().map(|(k, v)| (uint(k), v)).collect())
}

impl Request {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::Run { action, url } => {
                let mut f = vec![(0, Value::Text(action.clone()))];
                if let Some(url) = url {
                    f.push((1, Value::Text(url.clone())));
                }
                RawMessage::new(0, map(f))
            }
            Self::List => RawMessage::new(1, Value::Map(Vec::new())),
        }
    }

    /// `Ok(None)` for an unknown type.
    pub fn decode(msg: RawMessage) -> Result<Option<Self>> {
        let mut b = Body::new(msg.body)?;
        Ok(Some(match msg.ty {
            0 => {
                let action = b.text(0, 1, MAX_ID)?;
                if !is_action_id(&action) {
                    return Err(FrameError::Malformed(
                        "action id has a disallowed character",
                    ));
                }
                Self::Run {
                    action,
                    url: b.opt_text(1, 1, MAX_URL)?,
                }
            }
            1 => Self::List,
            _ => return Ok(None),
        }))
    }
}

impl Reply {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::Result(status) => RawMessage::new(0, map(vec![(0, uint(status.code()))])),
            Self::Actions(list) => RawMessage::new(
                1,
                map(vec![(
                    0,
                    Value::Array(
                        list.iter()
                            .map(|a| {
                                Value::Array(vec![
                                    Value::Text(a.id.clone()),
                                    Value::Text(a.label.clone()),
                                ])
                            })
                            .collect(),
                    ),
                )]),
            ),
        }
    }

    /// `Ok(None)` for an unknown type.
    pub fn decode(msg: RawMessage) -> Result<Option<Self>> {
        let mut b = Body::new(msg.body)?;
        Ok(Some(match msg.ty {
            0 => Self::Result(Status::from_code(b.u64(0)?)),
            1 => {
                let Some(Value::Array(items)) = b.take(0) else {
                    return Err(WRONG);
                };
                if items.len() > MAX_LISTED {
                    return Err(FrameError::Malformed("too many actions listed"));
                }
                let list = items
                    .into_iter()
                    .map(|item| match item {
                        Value::Array(pair) => match <[Value; 2]>::try_from(pair) {
                            Ok([Value::Text(id), Value::Text(label)])
                                if is_action_id(&id) && (1..=MAX_LABEL).contains(&label.len()) =>
                            {
                                Ok(Listed { id, label })
                            }
                            _ => Err(WRONG),
                        },
                        _ => Err(WRONG),
                    })
                    .collect::<Result<Vec<_>>>()?;
                Self::Actions(list)
            }
            _ => return Ok(None),
        }))
    }
}

/// A body's fields by key; unknown keys are left alone (§4.3).
struct Body(Vec<(Value, Value)>);

impl Body {
    fn new(body: Value) -> Result<Self> {
        match body {
            Value::Map(pairs) => Ok(Body(pairs)),
            _ => Err(WRONG),
        }
    }

    fn take(&mut self, key: u64) -> Option<Value> {
        let i = self
            .0
            .iter()
            .position(|(k, _)| matches!(k, Value::Int(n) if *n == i128::from(key)))?;
        Some(self.0.swap_remove(i).1)
    }

    fn u64(&mut self, key: u64) -> Result<u64> {
        match self.take(key) {
            Some(Value::Int(n)) => u64::try_from(n).map_err(|_| WRONG),
            _ => Err(WRONG),
        }
    }

    fn opt_text(&mut self, key: u64, min: usize, max: usize) -> Result<Option<String>> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Text(t)) if (min..=max).contains(&t.len()) => Ok(Some(t)),
            Some(_) => Err(WRONG),
        }
    }

    fn text(&mut self, key: u64, min: usize, max: usize) -> Result<String> {
        self.opt_text(key, min, max)?.ok_or(WRONG)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{decode_raw_payload, encode_raw_frame, Channel};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn request(m: Request) -> String {
        let frame = encode_raw_frame(Channel::Actions, &m.encode()).unwrap();
        let back = decode_raw_payload(&frame[1..]).unwrap();
        assert_eq!(Request::decode(back).unwrap(), Some(m));
        hex(&frame)
    }

    fn reply(m: Reply) -> String {
        let frame = encode_raw_frame(Channel::Actions, &m.encode()).unwrap();
        let back = decode_raw_payload(&frame[1..]).unwrap();
        assert_eq!(Reply::decode(back).unwrap(), Some(m));
        hex(&frame)
    }

    /// The binary examples of PROTOCOL.md §9.2.
    #[test]
    fn spec_examples() {
        assert_eq!(
            request(Request::Run {
                action: "lock".into(),
                url: None
            }),
            "098200a100646c6f636b"
        );
        assert_eq!(
            request(Request::Run {
                action: "open-url".into(),
                url: Some("https://a.pl/".into())
            }),
            "1c8200a200686f70656e2d75726c016d68747470733a2f2f612e706c2f"
        );
        assert_eq!(request(Request::List), "038201a0");
        assert_eq!(reply(Reply::Result(Status::Done)), "058200a10000");
        assert_eq!(reply(Reply::Result(Status::Refused)), "058200a10001");
        assert_eq!(
            reply(Reply::Actions(vec![Listed {
                id: "lock".into(),
                label: "Zablokuj".into()
            }])),
            "148201a1008182646c6f636b685a61626c6f6b756a"
        );
    }

    #[test]
    fn rejects_bad_ids_and_lengths() {
        for id in ["", "Lock", "lock now", "lock;rm", "x/y", "zażółć"] {
            let m = RawMessage::new(0, map(vec![(0, Value::Text(id.into()))]));
            assert!(Request::decode(m).is_err(), "{id:?}");
        }
        let long = "a".repeat(MAX_ID + 1);
        let m = RawMessage::new(0, map(vec![(0, Value::Text(long))]));
        assert!(Request::decode(m).is_err());
        let url = "h".repeat(MAX_URL + 1);
        let m = RawMessage::new(
            0,
            map(vec![
                (0, Value::Text("open-url".into())),
                (1, Value::Text(url)),
            ]),
        );
        assert!(Request::decode(m).is_err());
        // A command in place of an id is not even well-formed.
        let m = RawMessage::new(0, map(vec![(0, uint(1))]));
        assert!(Request::decode(m).is_err());
        assert_eq!(
            Request::decode(RawMessage::new(9, Value::Map(Vec::new()))).unwrap(),
            None
        );
        assert_eq!(Status::from_code(99), Status::Failed);
    }
}
