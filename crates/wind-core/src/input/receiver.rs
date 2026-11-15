//! The receiver of the `input` channel (PROTOCOL.md §8.2–8.6): one state
//! machine fed by the `input` stream and by datagrams, turning cumulative
//! state into deltas for the system and dropping what is late or stale.
//!
//! It is pure: no I/O, no clock. The session feeds it and passes on the
//! [`InputEvent`]s it produces; an `Err(code)` from a stream frame means
//! "send `STOP_SENDING` with this code on the stream".

use std::collections::{BTreeMap, BTreeSet};

use crate::protocol::{
    Button, ErrorCode, Gesture, InputGesture, InputMessage, InputSession, Motion, PointerButton,
};

/// What reaches the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Move the pointer by (`dx`, `dy`), in 1/100 dp of the sender's screen.
    /// `t` is the sender's timestamp of the state (µs, its own clock).
    Pointer {
        dx: i64,
        dy: i64,
        t: Option<u64>,
    },
    /// Scroll by (`dx`, `dy`); `dy` positive is down.
    Scroll {
        dx: i64,
        dy: i64,
        t: Option<u64>,
    },
    /// Press or release a known button.
    Button {
        button: Button,
        down: bool,
    },
    Gesture(Gesture),
    /// CCS/1 input/2 only:let a platform owner cancel smoothing and release state even without a mouse button down.
    Begin(u32),
    End(u32),
    /// CCS/1 RULE:local withdrawal fence, after every older queued event, even without a live session.
    Withdrawn,
}

/// The two classes of continuous state (§8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Pointer = 0,
    Scroll = 1,
}

#[derive(Debug, Clone, Copy, Default)]
struct Cumulative {
    seq: Option<u64>,
    x: i64,
    y: i64,
}

/// Parked datagrams: at most one per class (§8.5).
type Slots = [Option<Motion>; 2];

/// Future sessions with parked datagrams kept at once (§8.5).
const MAX_FUTURE_SESSIONS: usize = 4;

fn park(slots: &mut Slots, class: Class, m: Motion) {
    let slot = &mut slots[class as usize];
    // The later state wins (§8.5).
    if slot.is_none_or(|old| m.seq > old.seq) {
        *slot = Some(m);
    }
}

/// The receiver's state for one sender (§8.6).
#[derive(Debug)]
pub struct InputReceiver {
    version: u64,
    /// Whether this side announces `input/1`.
    accepting: bool,
    /// Number of the announcement that last enabled `input/1` (§8.2).
    g_min: u64,
    /// The current session and whether it is live.
    cur: Option<u32>,
    live: bool,
    epoch: u64,
    state: [Cumulative; 2],
    pending_live: Slots,
    pending_future: BTreeMap<u32, Slots>,
    pressed: BTreeSet<u64>,
    /// The stream being read has not delivered its `InputSession` yet.
    awaiting_session: bool,
}

impl InputReceiver {
    /// A receiver that announces `input/1` from its `Hello` / `HelloAck`
    /// (`accepting`), or not.
    pub fn new(accepting: bool) -> Self {
        Self::with_version(accepting, 1)
    }
    pub fn with_version(accepting: bool, version: u64) -> Self {
        InputReceiver {
            version,
            accepting,
            g_min: 0,
            cur: None,
            live: false,
            epoch: 0,
            state: [Cumulative::default(); 2],
            pending_live: [None; 2],
            pending_future: BTreeMap::new(),
            pressed: BTreeSet::new(),
            awaiting_session: false,
        }
    }

    /// The current session, if one was taken, and whether it is live.
    pub fn session(&self) -> Option<(u32, bool)> {
        self.cur.map(|s| (s, self.live))
    }

    /// Datagrams parked now (at most 10, §8.5).
    pub fn parked(&self) -> usize {
        let count = |s: &Slots| s.iter().flatten().count();
        count(&self.pending_live) + self.pending_future.values().map(count).sum::<usize>()
    }

    /// A `PointerMotion` or `Scroll` datagram.
    pub fn datagram(&mut self, msg: &InputMessage, out: &mut Vec<InputEvent>) {
        let (class, m) = match msg {
            InputMessage::PointerMotion(m) => (Class::Pointer, *m),
            InputMessage::Scroll(m) => (Class::Scroll, *m),
            // Not datagram messages; the codec doesn't produce them here.
            _ => return,
        };
        if !self.accepting {
            return;
        }
        match self.cur {
            None => self.park_future(class, m),
            Some(cur) if m.s > cur => self.park_future(class, m),
            Some(cur) if m.s < cur || !self.live => {}
            Some(_) => {
                if m.b == self.epoch {
                    self.apply(class, m.seq, m.x, m.y, m.t, out);
                } else if m.b > self.epoch {
                    park(&mut self.pending_live, class, m);
                }
                // m.b < epoch: a later button brought this state already.
            }
        }
    }

    fn park_future(&mut self, class: Class, m: Motion) {
        if !self.pending_future.contains_key(&m.s)
            && self.pending_future.len() >= MAX_FUTURE_SESSIONS
        {
            // At the fifth, the oldest session's parked datagrams go.
            let oldest = *self.pending_future.keys().next().expect("non-empty");
            if oldest > m.s {
                return; // this one is the oldest
            }
            self.pending_future.remove(&oldest);
        }
        park(self.pending_future.entry(m.s).or_default(), class, m);
    }

    /// A new `input` stream began; its first frame must be `InputSession`.
    pub fn stream_started(&mut self) {
        self.awaiting_session = true;
    }

    /// A frame of the current `input` stream. `Err(code)`: stop the stream
    /// with that code (it is then over: call [`stream_ended`](Self::stream_ended)).
    pub fn stream_frame(
        &mut self,
        msg: &InputMessage,
        out: &mut Vec<InputEvent>,
    ) -> Result<(), ErrorCode> {
        if self.awaiting_session {
            let InputMessage::InputSession(session) = msg else {
                return Err(ErrorCode::ProtocolViolation);
            };
            self.awaiting_session = false;
            return self.input_session(*session, out);
        }
        if !self.live {
            return Ok(()); // a closed session's frames are skipped
        }
        match msg {
            InputMessage::InputSession(_) => Err(ErrorCode::ProtocolViolation),
            InputMessage::PointerButton(b) => self.button(*b, out),
            InputMessage::Gesture(g) => self.gesture(*g, out),
            InputMessage::PointerMotion(m) => {
                self.apply(Class::Pointer, m.seq, m.x, m.y, m.t, out);
                Ok(())
            }
            InputMessage::Scroll(m) => {
                self.apply(Class::Scroll, m.seq, m.x, m.y, m.t, out);
                Ok(())
            }
        }
    }

    fn input_session(
        &mut self,
        session: InputSession,
        out: &mut Vec<InputEvent>,
    ) -> Result<(), ErrorCode> {
        if self.cur.is_some_and(|cur| session.s <= cur) {
            return Err(ErrorCode::ProtocolViolation);
        }
        // A new session never follows a live one on the same stream order
        // (§3.1), but release anything left, to be safe.
        self.release_all(out);
        self.cur = Some(session.s);
        self.epoch = 0;
        self.state = [Cumulative::default(); 2];
        self.pending_live = [None; 2];
        // Parked datagrams of sessions before this one are stale.
        self.pending_future.retain(|&s, _| s >= session.s);
        if !self.accepting || session.g < self.g_min {
            self.live = false;
            self.pending_future.remove(&session.s);
            return Err(ErrorCode::Cancelled);
        }
        self.live = true;
        if self.version >= 2 {
            out.push(InputEvent::Begin(session.s));
        }
        if let Some(slots) = self.pending_future.remove(&session.s) {
            self.pending_live = slots;
        }
        self.review(out);
        Ok(())
    }

    fn button(&mut self, b: PointerButton, out: &mut Vec<InputEvent>) -> Result<(), ErrorCode> {
        if b.b != self.epoch.wrapping_add(1) {
            return Err(ErrorCode::ProtocolViolation);
        }
        // State first, then the transition, then the epoch (§8.4 rule 2).
        self.apply(Class::Pointer, b.seq, b.x, b.y, None, out);
        self.apply(Class::Scroll, b.seq, b.sx, b.sy, None, out);
        if !matches!(b.button, Button::Other(_)) {
            let code = button_code(b.button);
            let changed = if b.down {
                self.pressed.insert(code)
            } else {
                self.pressed.remove(&code)
            };
            if changed {
                out.push(InputEvent::Button {
                    button: b.button,
                    down: b.down,
                });
            }
        }
        self.epoch = b.b;
        self.review(out);
        Ok(())
    }

    fn gesture(&mut self, g: InputGesture, out: &mut Vec<InputEvent>) -> Result<(), ErrorCode> {
        if self.version < 2 || g.b != self.epoch.wrapping_add(1) {
            return Err(ErrorCode::ProtocolViolation);
        }
        self.apply(Class::Pointer, g.seq, g.x, g.y, None, out);
        self.apply(Class::Scroll, g.seq, g.sx, g.sy, None, out);
        if !matches!(g.gesture, Gesture::Other(_)) {
            out.push(InputEvent::Gesture(g.gesture));
        }
        self.epoch = g.b;
        self.review(out);
        Ok(())
    }

    /// Applies cumulative state (§8.3): a stale or repeated `seq` is skipped.
    fn apply(
        &mut self,
        class: Class,
        seq: u64,
        x: i64,
        y: i64,
        t: Option<u64>,
        out: &mut Vec<InputEvent>,
    ) {
        let st = &mut self.state[class as usize];
        if st.seq.is_some_and(|last| seq <= last) {
            return;
        }
        let (dx, dy) = (x.wrapping_sub(st.x), y.wrapping_sub(st.y));
        *st = Cumulative {
            seq: Some(seq),
            x,
            y,
        };
        if dx == 0 && dy == 0 {
            return;
        }
        out.push(match class {
            Class::Pointer => InputEvent::Pointer { dx, dy, t },
            Class::Scroll => InputEvent::Scroll { dx, dy, t },
        });
    }

    /// Looks at the parked datagrams of the live session (§8.5).
    fn review(&mut self, out: &mut Vec<InputEvent>) {
        for class in [Class::Pointer, Class::Scroll] {
            let Some(m) = self.pending_live[class as usize] else {
                continue;
            };
            if m.b < self.epoch {
                self.pending_live[class as usize] = None;
            } else if m.b == self.epoch {
                self.pending_live[class as usize] = None;
                self.apply(class, m.seq, m.x, m.y, m.t, out);
            }
        }
    }

    /// The current stream ended (FIN, reset, or our `STOP_SENDING`).
    pub fn stream_ended(&mut self, out: &mut Vec<InputEvent>) {
        self.awaiting_session = false;
        self.close(out);
    }

    fn close(&mut self, out: &mut Vec<InputEvent>) {
        let was_live = self.live;
        if self.live {
            self.live = false;
        }
        self.release_all(out);
        if was_live && self.version >= 2 {
            if let Some(s) = self.cur {
                out.push(InputEvent::End(s));
            }
        }
        self.pending_live = [None; 2];
        if let Some(cur) = self.cur {
            self.pending_future.retain(|&s, _| s > cur);
        }
    }

    fn release_all(&mut self, out: &mut Vec<InputEvent>) {
        for code in std::mem::take(&mut self.pressed) {
            out.push(InputEvent::Button {
                button: button_from(code),
                down: false,
            });
        }
    }

    /// This side withdrew `input/1`.
    pub fn withdraw(&mut self, out: &mut Vec<InputEvent>) {
        self.close(out);
        if self.version >= 2 {
            out.push(InputEvent::Withdrawn);
        }
        self.accepting = false;
        self.pending_future.clear();
    }

    /// This side enabled `input/1` in its announcement number `announcement`.
    pub fn enable(&mut self, announcement: u64) {
        self.accepting = true;
        self.g_min = announcement;
    }

    /// The connection ended: release buttons, forget everything.
    pub fn connection_ended(&mut self, out: &mut Vec<InputEvent>) {
        self.close(out);
        self.pending_future.clear();
        self.cur = None;
    }
}

fn button_code(b: Button) -> u64 {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
        Button::Other(n) => n,
    }
}

fn button_from(code: u64) -> Button {
    match code {
        0 => Button::Left,
        1 => Button::Right,
        2 => Button::Middle,
        n => Button::Other(n),
    }
}
