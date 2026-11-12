//! CCS/1 M:battery/1, the phone's battery level and charging on the PC (§9.4, WN-49).
//! IO:[0,{0:level 0..=100,1:charging bool}] on uni channel 07, phone -> PC only.
//! RULE:one on start, then only on a change of (level, charging); no timer. Nothing persisted.
use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::{
    protocol::{cbor::Value, Channel, RawMessage},
    session::{ChannelMessage, Router, Session, SessionError},
};

/// Messages queued for the receiver; statuses are tiny and only the last matters.
const QUEUE: usize = 4;
/// Low-battery alert at or below this level, while not charging.
pub const LOW: u8 = 15;
/// The low alert re-arms above this level (or once charging), so 15 <-> 16 doesn't flap.
pub const LOW_REARM: u8 = 20;
/// The full alert re-arms below this level (or once unplugged).
pub const FULL_REARM: u8 = 95;

/// The battery as the phone reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// Percent, 0..=100.
    pub level: u8,
    /// Plugged in (or full on the charger).
    pub charging: bool,
}

impl Status {
    /// `None` for a level above 100.
    pub fn new(level: u8, charging: bool) -> Option<Self> {
        (level <= 100).then_some(Self { level, charging })
    }

    pub fn encode(&self) -> RawMessage {
        RawMessage::new(
            0,
            Value::Map(vec![
                (Value::Int(0), Value::Int(self.level.into())),
                (Value::Int(1), Value::Bool(self.charging)),
            ]),
        )
    }

    /// A `Battery` message; `None` for another type or a malformed one
    /// (missing or repeated key, level over 100). Unknown keys are skipped (§4.3).
    pub fn decode(message: &RawMessage) -> Option<Self> {
        if message.ty != 0 {
            return None;
        }
        let Value::Map(fields) = &message.body else {
            return None;
        };
        let (mut level, mut charging) = (None, None);
        for (key, value) in fields {
            match (key, value) {
                (Value::Int(0), Value::Int(n)) if level.is_none() => {
                    level = Some(u8::try_from(*n).ok().filter(|l| *l <= 100)?)
                }
                (Value::Int(1), Value::Bool(b)) if charging.is_none() => charging = Some(*b),
                (Value::Int(0 | 1), _) => return None,
                (Value::Int(_), _) => {}
                _ => return None,
            }
        }
        Self::new(level?, charging?)
    }
}

/// Routes `battery/1` (the PC accepts it); the queue gets the peer's messages.
pub fn route(router: &mut Router) -> mpsc::Receiver<ChannelMessage> {
    router.route(Channel::Battery, 1, QUEUE)
}

/// The sending side of one session: sends a status only when it differs
/// from the last one sent, and nothing when the peer doesn't take `battery/1`.
#[derive(Debug, Default)]
pub struct Sender {
    last: Mutex<Option<Status>>,
}

impl Sender {
    /// `Ok(true)` when sent, `Ok(false)` when unchanged or not accepted.
    pub async fn send(&self, session: &Session, status: Status) -> Result<bool, SessionError> {
        if *self.last.lock().unwrap() == Some(status) {
            return Ok(false);
        }
        match session.send(Channel::Battery, &status.encode()).await {
            Ok(()) => {
                *self.last.lock().unwrap() = Some(status);
                Ok(true)
            }
            Err(SessionError::CapabilityMissing(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// An alert worth a notification on the PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// Dropped to [`LOW`] or below, not charging.
    Low(u8),
    /// Reached 100% on the charger.
    Full,
}

/// When to alert, for one session. The first status alerts nothing (the
/// panel shows it); each alert fires once until it re-arms (hysteresis).
#[derive(Debug, Default)]
pub struct Alerts {
    seen: bool,
    low_fired: bool,
    full_fired: bool,
}

impl Alerts {
    pub fn update(&mut self, s: Status) -> Option<Alert> {
        let low = !s.charging && s.level <= LOW;
        let full = s.charging && s.level >= 100;
        if s.charging || s.level > LOW_REARM {
            self.low_fired = false;
        }
        if !s.charging || s.level < FULL_REARM {
            self.full_fired = false;
        }
        if !self.seen {
            self.seen = true;
            self.low_fired |= low;
            self.full_fired |= full;
            return None;
        }
        if low && !self.low_fired {
            self.low_fired = true;
            return Some(Alert::Low(s.level));
        }
        if full && !self.full_fired {
            self.full_fired = true;
            return Some(Alert::Full);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(level: u8, charging: bool) -> Status {
        Status::new(level, charging).unwrap()
    }

    #[test]
    fn round_trip_and_bytes() {
        for status in [s(0, false), s(82, true), s(100, true)] {
            assert_eq!(Status::decode(&status.encode()), Some(status));
        }
        // §9.4 example: type 0, {0: 82, 1: true}.
        let frame =
            crate::protocol::encode_raw_frame(Channel::Battery, &s(82, true).encode()).unwrap();
        assert_eq!(
            frame,
            [0x08, 0x82, 0x00, 0xa2, 0x00, 0x18, 0x52, 0x01, 0xf5]
        );
    }

    #[test]
    fn malformed_is_none() {
        let map = |f: Vec<(Value, Value)>| RawMessage::new(0, Value::Map(f));
        let (lvl, chg) = (Value::Int(0), Value::Int(1));
        assert_eq!(
            Status::decode(&map(vec![
                (lvl.clone(), Value::Int(101)),
                (chg.clone(), Value::Bool(false))
            ])),
            None
        );
        assert_eq!(
            Status::decode(&map(vec![(lvl.clone(), Value::Int(50))])),
            None,
            "charging missing"
        );
        assert_eq!(
            Status::decode(&map(vec![(chg.clone(), Value::Bool(true))])),
            None,
            "level missing"
        );
        assert_eq!(
            Status::decode(&map(vec![
                (lvl.clone(), Value::Int(-1)),
                (chg.clone(), Value::Bool(true))
            ])),
            None
        );
        assert_eq!(
            Status::decode(&map(vec![
                (lvl.clone(), Value::Int(5)),
                (chg.clone(), Value::Int(1))
            ])),
            None,
            "charging not bool"
        );
        assert_eq!(
            Status::decode(&map(vec![
                (lvl.clone(), Value::Int(5)),
                (lvl.clone(), Value::Int(6)),
                (chg.clone(), Value::Bool(true))
            ])),
            None,
            "repeated key"
        );
        assert_eq!(
            Status::decode(&RawMessage::new(1, Value::Map(vec![]))),
            None,
            "other type"
        );
        assert_eq!(
            Status::decode(&map(vec![
                (lvl, Value::Int(5)),
                (chg, Value::Bool(false)),
                (Value::Int(9), Value::Text("x".into()))
            ])),
            Some(s(5, false)),
            "unknown key skipped"
        );
        assert_eq!(Status::new(101, false), None);
    }

    #[test]
    fn low_alert_once_per_discharge() {
        let mut a = Alerts::default();
        assert_eq!(a.update(s(30, false)), None);
        assert_eq!(a.update(s(16, false)), None);
        assert_eq!(a.update(s(15, false)), Some(Alert::Low(15)));
        assert_eq!(a.update(s(14, false)), None);
        assert_eq!(a.update(s(16, false)), None, "not re-armed at 16");
        assert_eq!(a.update(s(15, false)), None);
        assert_eq!(a.update(s(21, false)), None);
        assert_eq!(
            a.update(s(15, false)),
            Some(Alert::Low(15)),
            "re-armed above 20"
        );
        assert_eq!(a.update(s(15, true)), None, "charging");
        assert_eq!(
            a.update(s(14, false)),
            Some(Alert::Low(14)),
            "re-armed by charging"
        );
    }

    #[test]
    fn full_alert_once_per_charge() {
        let mut a = Alerts::default();
        assert_eq!(a.update(s(90, true)), None);
        assert_eq!(a.update(s(100, true)), Some(Alert::Full));
        assert_eq!(a.update(s(99, true)), None);
        assert_eq!(a.update(s(100, true)), None, "not re-armed at 99");
        assert_eq!(a.update(s(100, false)), None, "unplugged");
        assert_eq!(
            a.update(s(100, true)),
            Some(Alert::Full),
            "re-armed by unplugging"
        );
        assert_eq!(a.update(s(94, true)), None);
        assert_eq!(
            a.update(s(100, true)),
            Some(Alert::Full),
            "re-armed below 95"
        );
    }

    #[test]
    fn first_status_alerts_nothing() {
        let mut low = Alerts::default();
        assert_eq!(low.update(s(10, false)), None);
        assert_eq!(
            low.update(s(9, false)),
            None,
            "already low when the session began"
        );
        let mut full = Alerts::default();
        assert_eq!(full.update(s(100, true)), None);
        assert_eq!(full.update(s(100, true)), None);
    }
}
