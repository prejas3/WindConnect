//! The sender of the `input` channel (PROTOCOL.md §8): cumulative state,
//! `seq`, epochs and sessions ([`SenderCore`], pure), and [`InputSender`],
//! which sends continuous state as unreliable QUIC datagrams and button
//! transitions on the `input` stream.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::protocol::{
    encode_datagram, encode_frame, stream_header, Button, Channel, Gesture, InputGesture,
    InputKeyboard, InputLaser, InputMessage, InputSession, KeyboardAction, Message, Motion,
    PointerButton,
};
use crate::session::{send_priority, SessionError};

use super::receiver::Class;

/// Repeats of the last datagram of a gesture (§8.3): about 20 ms and
/// 100 ms after the last change.
pub const REPEATS: [Duration; 2] = [Duration::from_millis(20), Duration::from_millis(100)];

/// The sender's counters for one session (§8.2–8.4). Pure.
#[derive(Debug, Clone)]
pub struct SenderCore {
    s: u32,
    seq: u64,
    epoch: u64,
    pointer: (i64, i64),
    scroll: (i64, i64),
}

impl SenderCore {
    /// Session `s`; counters start from zero.
    pub fn new(s: u32) -> Self {
        SenderCore {
            s,
            seq: 0,
            epoch: 0,
            pointer: (0, 0),
            scroll: (0, 0),
        }
    }

    pub fn session(&self) -> u32 {
        self.s
    }

    /// The `InputSession` frame that opens this session's stream.
    pub fn input_session(&self, g: u64) -> InputMessage {
        InputMessage::InputSession(InputSession { s: self.s, g })
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Moves the pointer by (`dx`, `dy`): the cumulative state to send.
    pub fn pointer(&mut self, dx: i64, dy: i64, t: Option<u64>) -> InputMessage {
        self.pointer = (
            self.pointer.0.wrapping_add(dx),
            self.pointer.1.wrapping_add(dy),
        );
        InputMessage::PointerMotion(self.motion(self.pointer, t))
    }

    /// Scrolls by (`dx`, `dy`).
    pub fn scroll(&mut self, dx: i64, dy: i64, t: Option<u64>) -> InputMessage {
        self.scroll = (
            self.scroll.0.wrapping_add(dx),
            self.scroll.1.wrapping_add(dy),
        );
        InputMessage::Scroll(self.motion(self.scroll, t))
    }

    fn motion(&mut self, (x, y): (i64, i64), t: Option<u64>) -> Motion {
        Motion {
            seq: self.next_seq(),
            x,
            y,
            t,
            s: self.s,
            b: self.epoch,
        }
    }

    /// A button transition, with the full state (§8.4).
    pub fn button(&mut self, button: Button, down: bool) -> InputMessage {
        self.epoch += 1;
        InputMessage::PointerButton(PointerButton {
            seq: self.next_seq(),
            button,
            down,
            x: self.pointer.0,
            y: self.pointer.1,
            b: self.epoch,
            sx: self.scroll.0,
            sy: self.scroll.1,
        })
    }
    pub fn gesture(&mut self, gesture: Gesture) -> InputMessage {
        self.epoch += 1;
        InputMessage::Gesture(InputGesture {
            seq: self.next_seq(),
            gesture,
            x: self.pointer.0,
            y: self.pointer.1,
            b: self.epoch,
            sx: self.scroll.0,
            sy: self.scroll.1,
        })
    }
    pub fn keyboard(&mut self, action: KeyboardAction) -> InputMessage {
        self.epoch += 1;
        InputMessage::Keyboard(InputKeyboard {
            seq: self.next_seq(),
            action,
            x: self.pointer.0,
            y: self.pointer.1,
            b: self.epoch,
            sx: self.scroll.0,
            sy: self.scroll.1,
        })
    }
    pub fn laser(&mut self, on: bool) -> InputMessage {
        self.epoch += 1;
        InputMessage::Laser(InputLaser {
            seq: self.next_seq(),
            on,
            x: self.pointer.0,
            y: self.pointer.1,
            b: self.epoch,
            sx: self.scroll.0,
            sy: self.scroll.1,
        })
    }
}

struct State {
    core: SenderCore,
    stream: Option<quinn::SendStream>,
    /// The `g` the current session was opened with.
    session_g: u64,
    /// The next session number (§8.2: 0, then one more each time).
    next_session: u32,
    /// The last datagram of each class and a counter, for the repeats.
    last: [Option<(u64, Vec<u8>)>; 2],
}

/// What the peer has announced about input (§8.2), while it accepts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAnnouncement {
    /// Its last announcement containing `input/1`: the `g` of a new session.
    pub last: u64,
    /// Its announcement that last enabled `input/1` (the receiver's
    /// `g_min`): a session with a smaller `g` would be refused.
    pub enabled: u64,
}

/// Sends input to the peer of one connection. Cheap to clone.
#[derive(Clone)]
pub struct InputSender {
    conn: quinn::Connection,
    datagrams: bool,
    /// The peer's input announcements now (§8.2), `None` while it doesn't
    /// accept input.
    g: Arc<dyn Fn() -> Option<InputAnnouncement> + Send + Sync>,
    state: Arc<Mutex<State>>,
    start: Instant,
    gestures: Arc<dyn Fn() -> bool + Send + Sync>,
    keyboard: Arc<dyn Fn() -> bool + Send + Sync>,
    laser: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl std::fmt::Debug for InputSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSender")
            .field("datagrams", &self.datagrams)
            .finish_non_exhaustive()
    }
}

impl InputSender {
    /// CCS/1 RULE:platform leases bind to the enabling announcement, so off/on invalidates an old gesture capture.
    pub fn announcement(&self) -> Option<InputAnnouncement> {
        (self.g)()
    }
    /// `g` gives the peer's input announcements now, or `None` while it
    /// doesn't accept input.
    pub(crate) fn new(
        conn: quinn::Connection,
        g: Arc<dyn Fn() -> Option<InputAnnouncement> + Send + Sync>,
        gestures: Arc<dyn Fn() -> bool + Send + Sync>,
        keyboard: Arc<dyn Fn() -> bool + Send + Sync>,
        laser: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        // §8.7: the path for continuous events is fixed per connection.
        let datagrams = conn.max_datagram_size().is_some();
        InputSender {
            conn,
            datagrams,
            g,
            gestures,
            keyboard,
            laser,
            state: Arc::new(Mutex::new(State {
                core: SenderCore::new(0),
                stream: None,
                session_g: 0,
                next_session: 0,
                last: [None, None],
            })),
            start: Instant::now(),
        }
    }

    /// Whether continuous events go as datagrams (else on the stream).
    pub fn uses_datagrams(&self) -> bool {
        self.datagrams
    }
    pub fn supports_gestures(&self) -> bool {
        (self.gestures)()
    }
    pub fn supports_keyboard(&self) -> bool {
        (self.keyboard)()
    }
    pub async fn keyboard(&self, action: KeyboardAction) -> Result<(), SessionError> {
        if !action.valid() || !(self.keyboard)() {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let mut state = self.state.lock().await;
        self.ensure_stream(&mut state).await?;
        if !(self.keyboard)() {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let msg = state.core.keyboard(action);
        self.write_stream(&mut state, &msg).await
    }
    pub fn supports_laser(&self) -> bool {
        (self.laser)()
    }
    /// CCS/1 input/4:checked before and after opening the stream, like the keyboard.
    pub async fn laser(&self, on: bool) -> Result<(), SessionError> {
        if !(self.laser)() {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let mut state = self.state.lock().await;
        self.ensure_stream(&mut state).await?;
        if !(self.laser)() {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let msg = state.core.laser(on);
        self.write_stream(&mut state, &msg).await
    }
    pub async fn gesture(&self, gesture: Gesture) -> Result<(), SessionError> {
        if !(self.gestures)() || matches!(gesture, Gesture::Other(_)) {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let mut state = self.state.lock().await;
        self.ensure_stream(&mut state).await?;
        // CCS/1 COMPAT:check again after opening; capability withdrawal never queues a v2 barrier for a v1 peer.
        if !(self.gestures)() {
            return Err(SessionError::CapabilityMissing(Channel::Input));
        }
        let msg = state.core.gesture(gesture);
        self.write_stream(&mut state, &msg).await
    }

    fn now_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    /// Moves the pointer by (`dx`, `dy`) (1/100 dp).
    pub async fn pointer(&self, dx: i64, dy: i64) -> Result<(), SessionError> {
        let t = Some(self.now_us());
        self.continuous(Class::Pointer, move |core| core.pointer(dx, dy, t))
            .await
    }

    /// Scrolls by (`dx`, `dy`).
    pub async fn scroll(&self, dx: i64, dy: i64) -> Result<(), SessionError> {
        let t = Some(self.now_us());
        self.continuous(Class::Scroll, move |core| core.scroll(dx, dy, t))
            .await
    }

    /// Presses or releases a button (always on the stream).
    pub async fn button(&self, button: Button, down: bool) -> Result<(), SessionError> {
        let mut state = self.state.lock().await;
        self.ensure_stream(&mut state).await?;
        let msg = state.core.button(button, down);
        self.write_stream(&mut state, &msg).await
    }

    /// Ends the current session (FIN on its stream); the next event starts
    /// a new one.
    pub async fn end_session(&self) {
        let mut state = self.state.lock().await;
        if let Some(mut stream) = state.stream.take() {
            let _ = stream.finish();
        }
        state.last = [None, None];
    }

    async fn continuous(
        &self,
        class: Class,
        make: impl FnOnce(&mut SenderCore) -> InputMessage,
    ) -> Result<(), SessionError> {
        let mut state = self.state.lock().await;
        self.ensure_stream(&mut state).await?;
        let msg = make(&mut state.core);
        if !self.datagrams {
            return self.write_stream(&mut state, &msg).await;
        }
        let bytes = encode_datagram(&msg).map_err(SessionError::Frame)?;
        let seq = match &msg {
            InputMessage::PointerMotion(m) | InputMessage::Scroll(m) => m.seq,
            _ => unreachable!("continuous events are motion or scroll"),
        };
        state.last[class as usize] = Some((seq, bytes.clone()));
        drop(state);
        self.send_datagram(bytes);
        self.schedule_repeats(class, seq);
        Ok(())
    }

    fn send_datagram(&self, bytes: Vec<u8>) {
        // Unreliable by design: a lost or refused datagram is made good by
        // the next one (cumulative state) or the repeats.
        let _ = self.conn.send_datagram(bytes.into());
    }

    /// Repeats the gesture's last datagram unchanged after 20 ms and 100 ms,
    /// unless a newer state of that class was sent meanwhile (§8.3).
    fn schedule_repeats(&self, class: Class, seq: u64) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut waited = Duration::ZERO;
            for at in REPEATS {
                tokio::time::sleep(at - waited).await;
                waited = at;
                let bytes = {
                    let state = this.state.lock().await;
                    match &state.last[class as usize] {
                        Some((last, bytes)) if *last == seq => bytes.clone(),
                        _ => return, // superseded
                    }
                };
                this.send_datagram(bytes);
            }
        });
    }

    /// Ends the current session's stream (`RESET_STREAM`) and forgets it;
    /// pending repeats see the change and stop.
    fn retire(state: &mut State, code: crate::protocol::ErrorCode) {
        if let Some(mut stream) = state.stream.take() {
            let _ = stream.reset(crate::session::varint_code(code));
        }
        state.last = [None, None];
    }

    /// Makes sure a live session fits the peer's announcements: none while
    /// it doesn't accept input (§6 rule 5: end our stream); a new one when it
    /// re-enabled input after our session began (its `g` is now below the
    /// receiver's `g_min`, so it would be refused, §8.2), and when it stopped
    /// our stream. An announcement that keeps `input/1` leaves the session
    /// (and a drag in progress) alone. Opens a stream with its header and
    /// `InputSession` when there is none.
    async fn ensure_stream(&self, state: &mut State) -> Result<(), SessionError> {
        let Some(announced) = (self.g)() else {
            Self::retire(state, crate::protocol::ErrorCode::Cancelled);
            return Err(SessionError::CapabilityMissing(Channel::Input));
        };
        if state.stream.is_some() && state.session_g < announced.enabled {
            Self::retire(state, crate::protocol::ErrorCode::Cancelled);
        }
        let g = announced.last;
        if state.stream.is_some() {
            return Ok(());
        }
        let s = state.next_session;
        state.next_session += 1;
        state.core = SenderCore::new(s);
        state.last = [None, None];
        let mut stream = self.conn.open_uni().await?;
        let _ = stream.set_priority(send_priority(Channel::Input));
        let first = encode_frame(&Message::Input(state.core.input_session(g)))
            .map_err(SessionError::Frame)?;
        let mut out = stream_header(Channel::Input).to_vec();
        out.extend_from_slice(&first);
        stream.write_all(&out).await?;
        // When the receiver stops this session's stream (or the connection
        // ends), retire it at once: a sender of datagrams only would never
        // notice otherwise, and its motion would go to a dead session.
        let stopped = stream.stopped();
        let id = stream.id();
        let state_ref = Arc::downgrade(&self.state);
        tokio::spawn(async move {
            if let Ok(None) = stopped.await {
                return; // finished by us and taken by the peer
            }
            let Some(state) = state_ref.upgrade() else {
                return;
            };
            let mut state = state.lock().await;
            if state.stream.as_ref().map(|s| s.id()) == Some(id) {
                state.stream = None;
                state.last = [None, None];
            }
        });
        state.stream = Some(stream);
        state.session_g = g;
        Ok(())
    }

    async fn write_stream(
        &self,
        state: &mut State,
        msg: &InputMessage,
    ) -> Result<(), SessionError> {
        let frame = encode_frame(&Message::Input(msg.clone())).map_err(SessionError::Frame)?;
        let stream = state.stream.as_mut().expect("ensured");
        if let Err(e) = stream.write_all(&frame).await {
            // The receiver stopped the session (or the connection is gone):
            // the next event opens a new session.
            state.stream = None;
            return Err(e.into());
        }
        Ok(())
    }
}
