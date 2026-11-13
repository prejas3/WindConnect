//! CCS/1 M:call/1, the phone's call state for the PC (§9.10, WN-48): the PC pauses its players while the phone rings or is in
//! a call. IO:[0,{0:state 0 idle|1 ringing|2 offhook}] on uni channel 0c, phone -> PC only.
//! RULE:no number, name or app: only the state. Sent on every change and at the start of each session (idle too), so a PC that
//! paused its players during a dropped session resumes them. Nothing persisted.
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

/// Routes `call/1` (the PC, while "Wstrzymaj odtwarzanie przy połączeniu" is on).
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
}
