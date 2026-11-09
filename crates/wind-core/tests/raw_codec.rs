//! Raw messages (`[type, body]` for feature channels, WC-11): what is sent
//! is exactly what the peer reads, and a value outside the §4.1 subset is an
//! error, never a changed value or a panic.

use wind_core::protocol::cbor::{Value, MAX_DEPTH};
use wind_core::protocol::{
    decode_raw_payload, encode_raw_frame, varint, Channel, FrameError, RawMessage,
};

fn int(n: i128) -> Value {
    Value::Int(n)
}

fn body(pairs: Vec<(Value, Value)>) -> Value {
    Value::Map(pairs)
}

fn encode(msg: &RawMessage) -> Result<Vec<u8>, FrameError> {
    encode_raw_frame(Channel::Clipboard, msg)
}

/// The frame's payload, decoded back.
fn round_trip(msg: &RawMessage) -> RawMessage {
    let frame = encode(msg).unwrap();
    let (len, used) = varint::decode(&frame).unwrap().unwrap();
    assert_eq!(len as usize, frame.len() - used);
    decode_raw_payload(&frame[used..]).unwrap()
}

fn assert_malformed(msg: RawMessage, what: &str) {
    match encode(&msg) {
        Err(FrameError::Malformed(_)) => {}
        other => panic!("{what}: {other:?}"),
    }
}

#[test]
fn boundary_values_round_trip_unchanged() {
    let msg = RawMessage::new(
        u64::MAX,
        body(vec![
            (int(0), int(u64::MAX.into())),
            (int(1), int(i64::MIN.into())),
            (int(2), Value::Array(vec![int(-1), int(0), Value::Null])),
            (
                int(u64::MAX.into()),
                body(vec![(Value::Text("k".into()), int(1))]),
            ),
            (int(3), Value::Bytes(vec![0, 255])),
        ]),
    );
    let back = round_trip(&msg);
    assert_eq!(back.ty, u64::MAX);
    // Same content; the map comes back in canonical key order.
    let sorted = |v: &Value| match v {
        Value::Map(p) => {
            let mut p = p.clone();
            p.sort_by_key(|(k, _)| wind_core::protocol::cbor::to_vec(k));
            Value::Map(p)
        }
        other => other.clone(),
    };
    assert_eq!(sorted(&back.body), sorted(&msg.body));
}

#[test]
fn out_of_range_integers_are_refused() {
    let too_big = i128::from(u64::MAX) + 1;
    let too_small = i128::from(i64::MIN) - 1;
    for (what, b) in [
        ("value above u64", body(vec![(int(0), int(too_big))])),
        ("value below i64", body(vec![(int(0), int(too_small))])),
        ("value i128::MIN", body(vec![(int(0), int(i128::MIN))])),
        ("value i128::MAX", body(vec![(int(0), int(i128::MAX))])),
        (
            "nested value",
            body(vec![(int(0), Value::Array(vec![int(too_big)]))]),
        ),
        ("body key above u64", body(vec![(int(too_big), int(1))])),
        ("negative body key", body(vec![(int(-1), int(1))])),
        (
            "nested key",
            body(vec![(int(0), body(vec![(int(too_small), int(1))]))]),
        ),
    ] {
        assert_malformed(RawMessage::new(1, b), what);
    }
}

#[test]
fn repeated_keys_are_refused_without_panicking() {
    assert_malformed(
        RawMessage::new(1, body(vec![(int(0), int(1)), (int(0), int(2))])),
        "top level",
    );
    assert_malformed(
        RawMessage::new(
            1,
            body(vec![(
                int(0),
                body(vec![
                    (Value::Text("a".into()), int(1)),
                    (Value::Text("a".into()), int(2)),
                ]),
            )]),
        ),
        "nested map",
    );
}

#[test]
fn nesting_limit_matches_the_decoder() {
    // [type, {0: x}] uses two levels; x adds the rest.
    let nested = |levels: usize| {
        let mut v = int(1);
        for _ in 0..levels {
            v = Value::Array(vec![v]);
        }
        RawMessage::new(1, body(vec![(int(0), v)]))
    };
    let deepest = nested(MAX_DEPTH - 2);
    assert_eq!(round_trip(&deepest), deepest);
    assert_malformed(nested(MAX_DEPTH - 1), "one level too deep");
    assert_malformed(nested(MAX_DEPTH + 40), "far too deep");
}

#[test]
fn non_map_body_and_control_are_refused() {
    assert_malformed(RawMessage::new(1, int(5)), "body not a map");
    assert!(matches!(
        encode_raw_frame(Channel::Control, &RawMessage::new(2, body(vec![]))),
        Err(FrameError::Malformed(_))
    ));
    let empty = RawMessage::new(0, body(vec![]));
    assert_eq!(round_trip(&empty), empty);
}

/// Takes a value apart level by level, so dropping a deliberately deep
/// value doesn't recurse.
fn dismantle(mut v: Value) {
    loop {
        v = match v {
            Value::Array(mut items) => match items.pop() {
                Some(inner) => inner,
                None => return,
            },
            Value::Map(mut pairs) => match pairs.pop() {
                Some((_, inner)) => inner,
                None => return,
            },
            _ => return,
        };
    }
}

#[test]
fn arbitrarily_deep_input_is_refused_without_deep_recursion() {
    // Far deeper than any recursive walk of it could survive on a test
    // thread's stack.
    let mut deep = int(1);
    for _ in 0..1_000_000 {
        deep = Value::Array(vec![deep]);
    }
    let msg = RawMessage::new(1, body(vec![(int(0), deep)]));
    assert_malformed_ref(&msg, "deep array");

    let RawMessage { body: b, .. } = msg;
    dismantle(b);

    let mut deep_map = int(1);
    for _ in 0..1_000_000 {
        deep_map = Value::Map(vec![(int(0), deep_map)]);
    }
    let msg = RawMessage::new(1, deep_map);
    assert_malformed_ref(&msg, "deep map body");
    let RawMessage { body: b, .. } = msg;
    dismantle(b);
}

fn assert_malformed_ref(msg: &RawMessage, what: &str) {
    match encode(msg) {
        Err(FrameError::Malformed(_)) => {}
        other => panic!("{what}: {other:?}"),
    }
}
