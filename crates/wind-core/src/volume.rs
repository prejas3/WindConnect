//! CCS/1 M:volume/1, each side's output volume shown and set by the other (§9.9, WN-48).
//! IO:uni channel 0b both ways: [0 Level,{0:level 0..=100,1:muted}] from the side whose volume it is; [1 Set,{0:level?,1:muted?}]
//! from the controlling side. A side sends `Level` only when the peer has `volume/1`, and takes `Set` only having announced it.
//! RULE:`Set` changes only what it names: a level alone never unmutes. Out-of-range is malformed, never clamped by the receiver.
//! At most ~10 `Set` a second are applied, the latest wins ([`latest`]).
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session, SessionError},
};

/// `Set`s applied at most this often (the latest of those queued meanwhile wins); `Level`s are sent at most this often too.
pub const PERIOD: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    /// The sender's volume, 0..=100, and whether it is muted.
    Level { level: u8, muted: bool },
    /// Change the receiver's volume; only what is given.
    Set {
        level: Option<u8>,
        muted: Option<bool>,
    },
}

impl Message {
    pub fn encode(&self) -> RawMessage {
        let mut f = Vec::new();
        let (ty, level, muted) = match *self {
            Self::Level { level, muted } => (0, Some(level), Some(muted)),
            Self::Set { level, muted } => (1, level, muted),
        };
        if let Some(l) = level {
            f.push((Value::Int(0), Value::Int(l.into())));
        }
        if let Some(m) = muted {
            f.push((Value::Int(1), Value::Bool(m)));
        }
        RawMessage::new(ty, Value::Map(f))
    }

    /// `None` for an unknown type or a malformed message (a level over 100, a wrong type, a repeated key, a `Level` missing a
    /// field, a `Set` with nothing in it). Unknown keys are skipped (§4.3).
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let Value::Map(fields) = &m.body else {
            return None;
        };
        let (mut level, mut muted) = (None, None);
        for (key, value) in fields {
            match (key, value) {
                (Value::Int(0), Value::Int(n)) if level.is_none() => {
                    level = Some(u8::try_from(*n).ok().filter(|l| *l <= 100)?)
                }
                (Value::Int(1), Value::Bool(b)) if muted.is_none() => muted = Some(*b),
                (Value::Int(0 | 1), _) => return None,
                (Value::Int(_), _) => {}
                _ => return None,
            }
        }
        match m.ty {
            0 => Some(Self::Level {
                level: level?,
                muted: muted?,
            }),
            1 if level.is_some() || muted.is_some() => Some(Self::Set { level, muted }),
            _ => None,
        }
    }
}

/// What a `Set` makes of the current (level, muted): only what it names changes.
pub fn apply(current: (u8, bool), level: Option<u8>, muted: Option<bool>) -> (u8, bool) {
    (level.unwrap_or(current.0), muted.unwrap_or(current.1))
}

/// 0..=100 to a device's 0..=max steps (rounded), and back.
pub fn to_steps(level: u8, max: u32) -> u32 {
    (u32::from(level.min(100)) * max + 50) / 100
}

pub fn from_steps(steps: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    ((steps.min(max) * 100 + max / 2) / max) as u8
}

/// Routes `volume/1`: the queue gets the peer's `Level`s and `Set`s.
pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::Volume, 1, 32)
}

/// Sends one; `Ok(false)` when the peer doesn't take `volume/1`.
pub async fn send(session: &Session, message: &Message) -> Result<bool, SessionError> {
    match session.send(Channel::Volume, &message.encode()).await {
        Ok(()) => Ok(true),
        Err(SessionError::CapabilityMissing(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Takes the next message and, after it, whatever else is queued: the latest wins. The caller waits [`PERIOD`] after applying
/// it, so a dragged slider is applied at most ~10 times a second. `None` when the queue is closed.
pub async fn latest<T>(queue: &mut mpsc::Receiver<T>) -> Option<T> {
    let mut last = queue.recv().await?;
    while let Ok(next) = queue.try_recv() {
        last = next;
    }
    Some(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    #[test]
    fn round_trips_and_bounds() {
        for m in [
            Message::Level {
                level: 0,
                muted: false,
            },
            Message::Level {
                level: 100,
                muted: true,
            },
            Message::Set {
                level: Some(42),
                muted: None,
            },
            Message::Set {
                level: None,
                muted: Some(false),
            },
            Message::Set {
                level: Some(7),
                muted: Some(true),
            },
        ] {
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        let raw = |ty, f| RawMessage::new(ty, Value::Map(f));
        assert_eq!(
            Message::decode(&raw(1, vec![(Value::Int(0), Value::Int(101))])),
            None,
            "out of range is malformed"
        );
        assert_eq!(Message::decode(&raw(1, vec![])), None, "an empty Set");
        assert_eq!(
            Message::decode(&raw(0, vec![(Value::Int(0), Value::Int(5))])),
            None,
            "a Level needs both"
        );
        assert_eq!(
            Message::decode(&raw(1, vec![(Value::Int(1), Value::Int(1))])),
            None
        );
        assert_eq!(
            Message::decode(&raw(2, vec![(Value::Int(0), Value::Int(1))])),
            None
        );
        // PROTOCOL.md §9.9: Level 40 %, not muted.
        assert_eq!(
            encode_raw_frame(
                Channel::Volume,
                &Message::Level {
                    level: 40,
                    muted: false
                }
                .encode()
            )
            .unwrap(),
            [0x08, 0x82, 0x00, 0xa2, 0x00, 0x18, 0x28, 0x01, 0xf4]
        );
    }

    #[test]
    fn a_set_changes_only_what_it_names() {
        assert_eq!(
            apply((30, true), Some(80), None),
            (80, true),
            "a level alone never unmutes"
        );
        assert_eq!(apply((30, true), None, Some(false)), (30, false));
        assert_eq!(apply((30, false), Some(10), Some(true)), (10, true));
    }

    #[test]
    fn steps_scale_both_ways() {
        assert_eq!(to_steps(0, 15), 0);
        assert_eq!(to_steps(100, 15), 15);
        assert_eq!(to_steps(50, 15), 8);
        assert_eq!(from_steps(8, 15), 53);
        assert_eq!(from_steps(15, 15), 100);
        assert_eq!(from_steps(3, 0), 0);
        for max in [7, 15, 25, 100] {
            for steps in 0..=max {
                assert_eq!(
                    to_steps(from_steps(steps, max), max),
                    steps,
                    "a step survives the round trip ({max})"
                );
            }
        }
    }

    #[tokio::test]
    async fn the_latest_wins() {
        let (tx, mut rx) = mpsc::channel(8);
        for i in 0..5 {
            tx.send(i).await.unwrap();
        }
        assert_eq!(latest(&mut rx).await, Some(4));
        drop(tx);
        assert_eq!(latest(&mut rx).await, None);
    }
}
