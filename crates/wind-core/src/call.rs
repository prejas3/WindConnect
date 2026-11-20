//! CCS/1 M:call/1, the phone's call state for the PC (§9.10, WN-48): the PC pauses its players while the phone rings or is in
//! a call. IO:[0,{0:state 0 idle|1 ringing|2 offhook, 1?:call, 2?:number, 3?:name, 4?:actions}], [1,{0:[[number,name,date]]}]
//! on uni channel 0c, phone -> PC only; `call-action` (0e, §9.12) PC -> phone: Reject [0,{0:call}], Silence [1,{0:call}].
//! RULE:the caller (WN-57) only with the phone's "Calls on the computer" and its permissions; without it, only the state.
//! Sent on every change and at the start of each session (idle too), so a PC that paused its players during a dropped session
//! resumes them. Nothing persisted, no number logged.
use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session, SessionError},
};
use tokio::sync::mpsc;

/// The phone's call state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallState {
    Idle,
    Ringing,
    /// In a call (phone or messenger).
    Offhook,
}

impl CallState {
    pub fn code(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Ringing => 1,
            Self::Offhook => 2,
        }
    }

    pub fn from_code(code: u64) -> Option<Self> {
        Some(match code {
            0 => Self::Idle,
            1 => Self::Ringing,
            2 => Self::Offhook,
            _ => return None,
        })
    }

    pub fn encode(self) -> RawMessage {
        RawMessage::new(
            0,
            Value::Map(vec![(Value::Int(0), Value::Int(self.code().into()))]),
        )
    }

    /// A `State`; `None` for another type or a malformed one (missing, repeated or unknown state). Unknown keys are skipped.
    pub fn decode(message: &RawMessage) -> Option<Self> {
        if message.ty != 0 {
            return None;
        }
        let Value::Map(fields) = &message.body else {
            return None;
        };
        let mut state = None;
        for (key, value) in fields {
            match (key, value) {
                (Value::Int(0), Value::Int(n)) if state.is_none() => {
                    state = Some(Self::from_code(u64::try_from(*n).ok()?)?)
                }
                (Value::Int(0), _) => return None,
                (Value::Int(_), _) => {}
                _ => return None,
            }
        }
        state
    }
}

/// Bytes of a caller's number.
pub const MAX_NUMBER: usize = 64;
/// Bytes of a caller's name.
pub const MAX_NAME: usize = 128;
/// Most missed calls in one `Missed`.
pub const MAX_MISSED: usize = 20;
/// [`Caller::actions`]: the phone can reject the call.
pub const CAN_REJECT: u8 = 1;
/// [`Caller::actions`]: the phone can silence its ringer.
pub const CAN_SILENCE: u8 = 2;

/// Who calls (WN-57), with the phone's "Calls on the computer" on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Caller {
    /// The phone's number for this call; actions name it, so a click meant for an old call does nothing.
    pub call: u64,
    pub number: Option<String>,
    /// From the phone's contacts.
    pub name: Option<String>,
    /// [`CAN_REJECT`] | [`CAN_SILENCE`]: what `call-action` can do now.
    pub actions: u8,
}

/// A `State` with its caller, when the phone shares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub state: CallState,
    pub caller: Option<Caller>,
}

/// A call nobody answered (WN-57).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missed {
    pub number: String,
    pub name: Option<String>,
    pub date_ms: u64,
}

/// What comes on `call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    State(Call),
    /// The newest missed calls, newest first, at most [`MAX_MISSED`]; the whole list each time.
    Missed(Vec<Missed>),
}

fn int(n: u64) -> Value {
    Value::Int(n.into())
}

fn text_ok(v: &Value, max: usize) -> Option<String> {
    match v {
        Value::Text(t) if !t.is_empty() && t.len() <= max => Some(t.clone()),
        _ => None,
    }
}

impl Message {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::State(call) => {
                let mut f = vec![(Value::Int(0), int(call.state.code().into()))];
                if let Some(c) = &call.caller {
                    f.push((Value::Int(1), int(c.call)));
                    if let Some(n) = &c.number {
                        f.push((Value::Int(2), Value::Text(n.clone())));
                    }
                    if let Some(n) = &c.name {
                        f.push((Value::Int(3), Value::Text(n.clone())));
                    }
                    f.push((Value::Int(4), int(c.actions.into())));
                }
                RawMessage::new(0, Value::Map(f))
            }
            Self::Missed(items) => RawMessage::new(
                1,
                Value::Map(vec![(
                    Value::Int(0),
                    Value::Array(
                        items
                            .iter()
                            .map(|m| {
                                Value::Array(vec![
                                    Value::Text(m.number.clone()),
                                    m.name.clone().map_or(Value::Null, Value::Text),
                                    int(m.date_ms),
                                ])
                            })
                            .collect(),
                    ),
                )]),
            ),
        }
    }

    /// `None` for an unknown type or a malformed message (texts empty or over their limits are refused, not cut).
    /// A `State` without `1 call` has no caller (a WN-48 phone, or the setting off).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let Value::Map(pairs) = &m.body else {
            return None;
        };
        let mut f: Vec<(u64, &Value)> = Vec::with_capacity(pairs.len());
        for (k, v) in pairs {
            let Value::Int(k) = k else { return None };
            let k = u64::try_from(*k).ok()?;
            if f.iter().any(|(seen, _)| *seen == k) {
                return None;
            }
            f.push((k, v));
        }
        let get = |k: u64| f.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        let uint = |v: &Value| match v {
            Value::Int(n) => u64::try_from(*n).ok(),
            _ => None,
        };
        match m.ty {
            0 => {
                let state = CallState::from_code(uint(get(0)?)?)?;
                let caller = match get(1) {
                    None => None,
                    Some(call) => Some(Caller {
                        call: uint(call)?,
                        number: match get(2) {
                            None => None,
                            Some(v) => Some(text_ok(v, MAX_NUMBER)?),
                        },
                        name: match get(3) {
                            None => None,
                            Some(v) => Some(text_ok(v, MAX_NAME)?),
                        },
                        actions: match get(4) {
                            None => 0,
                            Some(v) => (uint(v)? & u64::from(CAN_REJECT | CAN_SILENCE)) as u8,
                        },
                    }),
                };
                Some(Self::State(Call { state, caller }))
            }
            1 => {
                let Value::Array(list) = get(0)? else {
                    return None;
                };
                if list.len() > MAX_MISSED {
                    return None;
                }
                list.iter()
                    .map(|item| {
                        let Value::Array(parts) = item else {
                            return None;
                        };
                        let [number, name, date] = parts.as_slice() else {
                            return None;
                        };
                        Some(Missed {
                            number: text_ok(number, MAX_NUMBER)?,
                            name: match name {
                                Value::Null => None,
                                v => Some(text_ok(v, MAX_NAME)?),
                            },
                            date_ms: uint(date)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(Self::Missed)
            }
            _ => None,
        }
    }
}

/// What the PC asks of the phone's ringing call (`call-action/1`, §9.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Reject { call: u64 },
    Silence { call: u64 },
}

impl Action {
    pub fn encode(self) -> RawMessage {
        let (ty, call) = match self {
            Self::Reject { call } => (0, call),
            Self::Silence { call } => (1, call),
        };
        RawMessage::new(ty, Value::Map(vec![(Value::Int(0), int(call))]))
    }

    /// `None` for an unknown type or a malformed one.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let Value::Map(pairs) = &m.body else {
            return None;
        };
        let mut call = None;
        for (k, v) in pairs {
            match (k, v) {
                (Value::Int(0), Value::Int(n)) if call.is_none() => {
                    call = Some(u64::try_from(*n).ok()?)
                }
                (Value::Int(0), _) => return None,
                (Value::Int(_), _) => {}
                _ => return None,
            }
        }
        let call = call?;
        match m.ty {
            0 => Some(Self::Reject { call }),
            1 => Some(Self::Silence { call }),
            _ => None,
        }
    }
}

/// Routes `call-action/1` (the phone, with "Calls on the computer" on).
pub fn route_actions(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::CallAction, 1, 8)
}

/// Sends an action; `Ok(false)` when the phone doesn't take `call-action/1`.
pub async fn act(session: &Session, action: Action) -> Result<bool, SessionError> {
    match session.send(Channel::CallAction, &action.encode()).await {
        Ok(()) => Ok(true),
        Err(SessionError::CapabilityMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Sends a `State` with its caller, or `Missed`; `Ok(false)` when the peer doesn't take `call/1`.
pub async fn send_message(session: &Session, message: &Message) -> Result<bool, SessionError> {
    match session.send(Channel::Call, &message.encode()).await {
        Ok(()) => Ok(true),
        Err(SessionError::CapabilityMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Routes `call/1` (the PC, while "Pause playback during calls" is on).
pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::Call, 1, 8)
}

/// Sends the state; `Ok(false)` when the peer doesn't take `call/1`.
pub async fn send(session: &Session, state: CallState) -> Result<bool, SessionError> {
    match session.send(Channel::Call, &state.encode()).await {
        Ok(()) => Ok(true),
        Err(SessionError::CapabilityMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    #[test]
    fn round_trips_and_bytes() {
        for s in [CallState::Idle, CallState::Ringing, CallState::Offhook] {
            assert_eq!(CallState::decode(&s.encode()), Some(s));
        }
        // PROTOCOL.md §9.10: State ringing, with the frame length.
        assert_eq!(
            encode_raw_frame(Channel::Call, &CallState::Ringing.encode()).unwrap(),
            [0x05, 0x82, 0x00, 0xa1, 0x00, 0x01]
        );
    }

    #[test]
    fn malformed_is_none() {
        let bad = |body| RawMessage::new(0, Value::Map(body));
        assert_eq!(CallState::decode(&bad(vec![])), None);
        assert_eq!(
            CallState::decode(&bad(vec![(Value::Int(0), Value::Int(3))])),
            None
        );
        assert_eq!(
            CallState::decode(&bad(vec![(Value::Int(0), Value::Bool(true))])),
            None
        );
        assert_eq!(
            CallState::decode(&bad(vec![
                (Value::Int(0), Value::Int(1)),
                (Value::Int(0), Value::Int(1))
            ])),
            None
        );
        assert_eq!(
            CallState::decode(&bad(vec![
                (Value::Int(9), Value::Int(5)),
                (Value::Int(0), Value::Int(2))
            ])),
            Some(CallState::Offhook),
            "unknown keys skipped"
        );
        assert_eq!(
            CallState::decode(&RawMessage::new(1, Value::Map(vec![]))),
            None
        );
    }

    #[test]
    fn callers_missed_and_actions() {
        let caller = Caller {
            call: 7,
            number: Some("+48600100200".into()),
            name: Some("Ola".into()),
            actions: CAN_REJECT | CAN_SILENCE,
        };
        for m in [
            Message::State(Call {
                state: CallState::Ringing,
                caller: Some(caller.clone()),
            }),
            Message::State(Call {
                state: CallState::Idle,
                caller: None,
            }),
            Message::Missed(vec![Missed {
                number: "+48600".into(),
                name: None,
                date_ms: 1_700_000_000_000,
            }]),
        ] {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        // A WN-48 PC reads the state and skips the caller.
        let with = Message::State(Call {
            state: CallState::Ringing,
            caller: Some(caller),
        });
        assert_eq!(CallState::decode(&with.encode()), Some(CallState::Ringing));
        // ... and a WN-57 PC reads a WN-48 phone's state.
        assert_eq!(
            Message::decode(&CallState::Offhook.encode()),
            Some(Message::State(Call {
                state: CallState::Offhook,
                caller: None
            }))
        );
        for a in [Action::Reject { call: 7 }, Action::Silence { call: 7 }] {
            assert_eq!(Action::decode(&a.encode()), Some(a));
        }
        assert_eq!(
            encode_raw_frame(Channel::CallAction, &Action::Reject { call: 7 }.encode()).unwrap(),
            [0x05, 0x82, 0x00, 0xa1, 0x00, 0x07]
        );
        assert_eq!(
            encode_raw_frame(Channel::CallAction, &Action::Silence { call: 7 }.encode()).unwrap(),
            [0x05, 0x82, 0x01, 0xa1, 0x00, 0x07]
        );
        let long = Message::State(Call {
            state: CallState::Ringing,
            caller: Some(Caller {
                call: 1,
                number: Some("1".repeat(MAX_NUMBER + 1)),
                name: None,
                actions: 0,
            }),
        });
        assert_eq!(Message::decode(&long.encode()), None, "refused, not cut");
        let too_many = Message::Missed(
            (0..=MAX_MISSED)
                .map(|i| Missed {
                    number: "1".into(),
                    name: None,
                    date_ms: i as u64,
                })
                .collect(),
        );
        assert_eq!(Message::decode(&too_many.encode()), None);
    }
}
