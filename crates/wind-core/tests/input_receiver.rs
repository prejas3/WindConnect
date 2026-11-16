//! The `input` receiver state machine (PROTOCOL.md §8.2–8.6, WC-13): the
//! spec's worked examples, the state table's edge cases, and a property
//! test with lost, repeated and reordered datagrams.

use wind_core::input::{InputEvent, InputReceiver, SenderCore};
use wind_core::protocol::{Button, ErrorCode, InputMessage, InputSession, Motion, PointerButton};

fn motion(seq: u64, x: i64, y: i64, s: u32, b: u64) -> InputMessage {
    InputMessage::PointerMotion(Motion {
        seq,
        x,
        y,
        t: None,
        s,
        b,
    })
}

fn scroll(seq: u64, x: i64, y: i64, s: u32, b: u64) -> InputMessage {
    InputMessage::Scroll(Motion {
        seq,
        x,
        y,
        t: None,
        s,
        b,
    })
}

fn session(s: u32, g: u64) -> InputMessage {
    InputMessage::InputSession(InputSession { s, g })
}

#[allow(clippy::too_many_arguments)]
fn button(
    seq: u64,
    button: Button,
    down: bool,
    x: i64,
    y: i64,
    b: u64,
    sx: i64,
    sy: i64,
) -> InputMessage {
    InputMessage::PointerButton(PointerButton {
        seq,
        button,
        down,
        x,
        y,
        b,
        sx,
        sy,
    })
}

fn ptr(dx: i64, dy: i64) -> InputEvent {
    InputEvent::Pointer { dx, dy, t: None }
}

fn scr(dx: i64, dy: i64) -> InputEvent {
    InputEvent::Scroll { dx, dy, t: None }
}

fn btn(button: Button, down: bool) -> InputEvent {
    InputEvent::Button { button, down }
}

/// A receiver with session 0 live (stream started, `InputSession` 0/0).
fn live() -> InputReceiver {
    let mut r = InputReceiver::new(true);
    r.stream_started();
    r.stream_frame(&session(0, 0), &mut Vec::new()).unwrap();
    r
}

fn dg(r: &mut InputReceiver, m: InputMessage) -> Vec<InputEvent> {
    let mut out = Vec::new();
    r.datagram(&m, &mut out);
    out
}

fn fr(r: &mut InputReceiver, m: InputMessage) -> Result<Vec<InputEvent>, ErrorCode> {
    let mut out = Vec::new();
    r.stream_frame(&m, &mut out).map(|()| out)
}

#[test]
fn gesture_barrier_orders_lost_motion_future_datagrams_and_release() {
    use wind_core::protocol::Gesture;
    let mut sender = SenderCore::new(0);
    let mut receiver = InputReceiver::with_version(true, 2);
    receiver.stream_started();
    assert_eq!(
        fr(&mut receiver, sender.input_session(0)).unwrap(),
        [InputEvent::Begin(0)]
    );
    sender.pointer(100, 200, None); // lost datagram recovered by the reliable barrier
    let barrier = sender.gesture(Gesture::TaskView);
    let future = sender.pointer(30, 40, None);
    assert!(dg(&mut receiver, future.clone()).is_empty());
    assert_eq!(
        fr(&mut receiver, barrier).unwrap(),
        [
            ptr(100, 200),
            InputEvent::Gesture(Gesture::TaskView),
            ptr(30, 40)
        ]
    );
    assert!(dg(&mut receiver, future).is_empty());
    assert_eq!(
        fr(&mut receiver, sender.button(Button::Left, true)).unwrap(),
        [btn(Button::Left, true)]
    );
    let mut out = Vec::new();
    receiver.withdraw(&mut out);
    assert_eq!(
        out,
        [
            btn(Button::Left, false),
            InputEvent::End(0),
            InputEvent::Withdrawn
        ]
    );
    out.clear();
    receiver.connection_ended(&mut out);
    assert!(out.is_empty());
}

#[test]
fn v1_rejects_gesture_and_unknown_v2_gesture_preserves_epoch() {
    use wind_core::protocol::Gesture;
    let mut sender = SenderCore::new(0);
    assert_eq!(
        fr(&mut live(), sender.gesture(Gesture::TaskView)),
        Err(ErrorCode::ProtocolViolation)
    );
    let mut sender = SenderCore::new(0);
    let mut receiver = InputReceiver::with_version(true, 2);
    receiver.stream_started();
    fr(&mut receiver, sender.input_session(0)).unwrap();
    sender.scroll(0, 500, None);
    assert_eq!(
        fr(&mut receiver, sender.gesture(Gesture::Other(99))).unwrap(),
        [scr(0, 500)]
    );
    assert_eq!(
        fr(&mut receiver, sender.gesture(Gesture::NextApp)).unwrap(),
        [InputEvent::Gesture(Gesture::NextApp)]
    );
}

#[test]
fn spec_8_3_loss_and_reordering() {
    let mut r = live();
    // Earlier state to start from: pointer at x=0 is the session's zero.
    assert_eq!(dg(&mut r, scroll(12, 0, 240, 0, 0)), [scr(0, 240)]);
    assert_eq!(dg(&mut r, motion(11, 101, 0, 0, 0)), [ptr(101, 0)]);
    assert_eq!(dg(&mut r, motion(10, 100, 0, 0, 0)), []); // 10 ≤ 11
                                                          // A repeat of the last state changes nothing.
    assert_eq!(dg(&mut r, motion(11, 101, 0, 0, 0)), []);
}

#[test]
fn spec_8_4_drag_with_datagrams_ahead_of_buttons() {
    let mut r = live();
    // seq 11 (b=1) and 13 (b=2) arrive before the buttons: parked, 13 wins.
    assert_eq!(dg(&mut r, motion(11, 200, 0, 0, 1)), []);
    assert_eq!(dg(&mut r, motion(13, 300, 0, 0, 2)), []);
    assert_eq!(r.parked(), 1);
    assert_eq!(
        fr(&mut r, button(10, Button::Left, true, 100, 0, 1, 0, 0)).unwrap(),
        [ptr(100, 0), btn(Button::Left, true)]
    );
    assert_eq!(
        fr(&mut r, button(12, Button::Left, false, 200, 0, 2, 0, 0)).unwrap(),
        [ptr(100, 0), btn(Button::Left, false), ptr(100, 0)]
    );
    assert_eq!(r.parked(), 0);
}

#[test]
fn spec_8_4_scroll_overtaken_by_a_button() {
    let mut r = live();
    assert_eq!(
        fr(&mut r, button(2, Button::Left, true, 0, 0, 1, 0, 240)).unwrap(),
        [scr(0, 240), btn(Button::Left, true)]
    );
    assert_eq!(dg(&mut r, scroll(1, 0, 240, 0, 0)), []); // 0 < epoch 1
}

#[test]
fn spec_8_5_datagram_of_a_new_session_before_its_input_session() {
    let mut r = live();
    let mut out = Vec::new();
    r.stream_ended(&mut out); // session 0 closed, cur = 0
                              // Datagram s=1 and its two repeats, before InputSession s=1.
    for _ in 0..3 {
        assert_eq!(dg(&mut r, motion(1, 1, 0, 1, 0)), []);
    }
    r.stream_started();
    assert_eq!(fr(&mut r, session(1, 0)).unwrap(), [ptr(1, 0)]);
    assert_eq!(r.session(), Some((1, true)));
}

#[test]
fn spec_8_5_closed_session_datagram_after_reenabling() {
    let mut r = live();
    assert_eq!(dg(&mut r, motion(7, 100, 0, 0, 0)), [ptr(100, 0)]);
    let mut out = Vec::new();
    r.withdraw(&mut out); // Capabilities no. 1
    r.enable(2); // Capabilities no. 2: g_min = 2
    assert_eq!(dg(&mut r, motion(8, 150, 0, 0, 0)), []); // s=0 ≤ cur, closed
                                                         // (The receiver's stream task ends the old stream.)
    r.stream_ended(&mut out);
    r.stream_started();
    assert_eq!(fr(&mut r, session(1, 2)).unwrap(), []);
    assert_eq!(dg(&mut r, motion(1, 1, 0, 1, 0)), [ptr(1, 0)]);
}

#[test]
fn spec_8_5_session_opened_before_withdrawal_is_cancelled() {
    let mut r = InputReceiver::new(true);
    let mut out = Vec::new();
    r.withdraw(&mut out);
    r.enable(2);
    // Datagrams of session 0 arrive while cur is unset: parked as future.
    assert_eq!(dg(&mut r, motion(1, 5, 0, 0, 0)), []);
    assert_eq!(r.parked(), 1);
    r.stream_started();
    assert_eq!(fr(&mut r, session(0, 0)), Err(ErrorCode::Cancelled));
    assert_eq!(r.session(), Some((0, false)));
    assert_eq!(r.parked(), 0);
    // Its later frames are skipped.
    assert_eq!(
        fr(&mut r, button(2, Button::Left, true, 0, 0, 1, 0, 0)).unwrap(),
        []
    );
    // The sender's next session, based on announcement 2, is live.
    r.stream_ended(&mut out);
    r.stream_started();
    fr(&mut r, session(1, 2)).unwrap();
    assert_eq!(r.session(), Some((1, true)));
}

#[test]
fn table_violations_stop_the_stream() {
    // InputSession with s ≤ cur.
    let mut r = live();
    let mut out = Vec::new();
    r.stream_ended(&mut out);
    r.stream_started();
    assert_eq!(fr(&mut r, session(0, 0)), Err(ErrorCode::ProtocolViolation));

    // A first frame other than InputSession.
    let mut r = InputReceiver::new(true);
    r.stream_started();
    assert_eq!(
        fr(&mut r, motion(1, 1, 1, 0, 0)),
        Err(ErrorCode::ProtocolViolation)
    );

    // A button whose epoch skips one.
    let mut r = live();
    assert_eq!(
        fr(&mut r, button(1, Button::Left, true, 0, 0, 2, 0, 0)),
        Err(ErrorCode::ProtocolViolation)
    );

    // A second InputSession on the same stream.
    let mut r = live();
    assert_eq!(fr(&mut r, session(1, 0)), Err(ErrorCode::ProtocolViolation));
}

#[test]
fn pressed_buttons_are_released_when_the_session_or_connection_ends() {
    let mut r = live();
    fr(&mut r, button(1, Button::Left, true, 0, 0, 1, 0, 0)).unwrap();
    fr(&mut r, button(2, Button::Right, true, 0, 0, 2, 0, 0)).unwrap();
    let mut out = Vec::new();
    r.stream_ended(&mut out);
    assert_eq!(out, [btn(Button::Left, false), btn(Button::Right, false)]);

    let mut r = live();
    fr(&mut r, button(1, Button::Middle, true, 0, 0, 1, 0, 0)).unwrap();
    let mut out = Vec::new();
    r.connection_ended(&mut out);
    assert_eq!(out, [btn(Button::Middle, false)]);
}

#[test]
fn unknown_button_moves_state_and_epoch_only() {
    let mut r = live();
    assert_eq!(
        fr(&mut r, button(1, Button::Other(9), true, 5, 0, 1, 0, 0)).unwrap(),
        [ptr(5, 0)]
    );
    // Epoch moved: a datagram with b=1 applies now.
    assert_eq!(dg(&mut r, motion(2, 6, 0, 0, 1)), [ptr(1, 0)]);
}

#[test]
fn not_accepting_drops_datagrams_and_parking_is_bounded() {
    let mut r = InputReceiver::new(false);
    assert_eq!(dg(&mut r, motion(1, 1, 1, 0, 0)), []);
    assert_eq!(r.parked(), 0);

    // Future sessions: at most 4 kept, one per class each; the 5th drops
    // the oldest.
    let mut r = InputReceiver::new(true);
    for s in 0..6u32 {
        dg(&mut r, motion(1, 1, 0, s, 0));
        dg(&mut r, scroll(2, 0, 1, s, 0));
        dg(&mut r, motion(3, 2, 0, s, 0)); // replaces seq 1
    }
    assert_eq!(r.parked(), 8);
    // Sessions 0 and 1 were dropped: session 2 is the oldest kept.
    r.stream_started();
    assert_eq!(fr(&mut r, session(2, 0)).unwrap(), [ptr(2, 0), scr(0, 1)]);
}

// --- Property: loss, repeats and reordering ---------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Whatever is lost, repeated or reordered among the datagrams, with the
/// stream in order and the gesture's last datagrams arriving at least once:
/// every button is pressed and released where the sender did it, in order,
/// and the pointer and scroll end where the sender's did.
#[test]
fn any_loss_or_reordering_ends_in_the_senders_state() {
    for seed in 1..=300u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut core = SenderCore::new(0);
        let mut stream = vec![core.input_session(0)];
        let mut datagrams = Vec::new();
        let mut presses = Vec::new(); // (button, down, pointer at the time)
        let mut pointer = (0i64, 0i64);
        let mut scrolled = (0i64, 0i64);
        let mut down = false;
        let (mut last_ptr, mut last_scr) = (None, None);
        for _ in 0..(20 + rng.below(60)) {
            match rng.below(10) {
                0..=5 => {
                    let (dx, dy) = (rng.below(41) as i64 - 20, rng.below(41) as i64 - 20);
                    pointer = (pointer.0 + dx, pointer.1 + dy);
                    let m = core.pointer(dx, dy, None);
                    last_ptr = Some(datagrams.len());
                    datagrams.push(m);
                }
                6..=7 => {
                    let dy = rng.below(200) as i64;
                    scrolled.1 += dy;
                    let m = core.scroll(0, dy, None);
                    last_scr = Some(datagrams.len());
                    datagrams.push(m);
                }
                _ => {
                    down = !down;
                    presses.push((Button::Left, down, pointer));
                    stream.push(core.button(Button::Left, down));
                }
            }
        }

        // Datagrams: each lost (30 %), once, or twice; the last of each
        // class at least once. Then shuffled.
        let mut sent = Vec::new();
        for (i, m) in datagrams.iter().enumerate() {
            let must = Some(i) == last_ptr || Some(i) == last_scr;
            let copies = match rng.below(10) {
                0..=2 if !must => 0,
                3 => 2,
                _ => 1,
            };
            for _ in 0..copies {
                sent.push(m.clone());
            }
        }
        for i in (1..sent.len()).rev() {
            sent.swap(i, rng.below(i as u64 + 1) as usize);
        }

        // Interleave the in-order stream with the shuffled datagrams.
        let mut r = InputReceiver::new(true);
        r.stream_started();
        let (mut si, mut di) = (0, 0);
        let mut at = (0i64, 0i64);
        let mut sc = (0i64, 0i64);
        let mut got = Vec::new();
        while si < stream.len() || di < sent.len() {
            let take_stream = di >= sent.len() || (si < stream.len() && rng.below(3) == 0);
            let mut out = Vec::new();
            if take_stream {
                r.stream_frame(&stream[si], &mut out).expect("valid stream");
                si += 1;
            } else {
                r.datagram(&sent[di], &mut out);
                di += 1;
            }
            for e in out {
                match e {
                    InputEvent::Pointer { dx, dy, .. } => at = (at.0 + dx, at.1 + dy),
                    InputEvent::Scroll { dx, dy, .. } => sc = (sc.0 + dx, sc.1 + dy),
                    InputEvent::Button { button, down } => got.push((button, down, at)),
                    InputEvent::Gesture(_)
                    | InputEvent::Keyboard(_)
                    | InputEvent::Laser(_)
                    | InputEvent::Begin(_)
                    | InputEvent::End(_)
                    | InputEvent::Withdrawn => {
                        panic!("input/1 emitted a v2 event")
                    }
                }
            }
        }
        assert_eq!(got, presses, "seed {seed}: buttons");
        assert_eq!(at, pointer, "seed {seed}: pointer");
        assert_eq!(sc, scrolled, "seed {seed}: scroll");
        assert!(r.parked() <= 10);
    }
}
