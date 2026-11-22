use wind_core::{
    input::{InputEvent, InputReceiver, SenderCore},
    protocol::*,
};

fn action(text: &str) -> KeyboardAction {
    KeyboardAction::Text(text.into())
}

#[test]
fn keyboard_codec_limits_and_datagram_refusal() {
    let vector = encode_payload(&Message::Input(SenderCore::new(0).keyboard(action("ą")))).unwrap(); // i18n: data
    assert_eq!(
        vector,
        vec![0x82, 5, 0xa7, 0, 1, 1, 0x62, 0xc4, 0x85, 2, 0, 3, 0, 4, 1, 5, 0, 6, 0]
    );
    for value in [
        action("Zażółć gęślą jaźń 😀"), // i18n: data
        action(&"ą".repeat(2048)),      // i18n: data
        KeyboardAction::Key {
            code: 67,
            modifiers: 1,
        },
        KeyboardAction::Key {
            code: u64::MAX,
            modifiers: 7,
        },
    ] {
        let msg = SenderCore::new(0).keyboard(value);
        let payload = encode_payload(&Message::Input(msg.clone())).unwrap();
        assert!(payload.len() <= MAX_INPUT_TEXT + 128);
        assert_eq!(
            decode_payload(Channel::Input, &payload).unwrap(),
            Message::Input(msg.clone())
        );
        assert!(encode_datagram(&msg).is_err());
        let mut datagram = vec![1, 4];
        datagram.extend(payload);
        assert!(decode_datagram(&datagram, MAX_DATAGRAM).is_err());
    }
    for value in [
        action(""),
        action("nul\0text"),
        action(&"ą".repeat(2049)), // i18n: data
        KeyboardAction::Key {
            code: 67,
            modifiers: 8,
        },
    ] {
        assert!(encode_payload(&Message::Input(SenderCore::new(0).keyboard(value))).is_err());
    }
    // CCS/1 SEC:decoder enforces limits independently of the typed sender.
    use cbor::Value;
    for payload in [Value::Text("x".repeat(4097)), Value::Text("\0".into())] {
        let fields = Value::Map(vec![
            (Value::Int(0), Value::Int(1)),
            (Value::Int(1), payload),
            (Value::Int(2), Value::Int(0)),
            (Value::Int(3), Value::Int(0)),
            (Value::Int(4), Value::Int(1)),
            (Value::Int(5), Value::Int(0)),
            (Value::Int(6), Value::Int(0)),
        ]);
        assert!(decode_payload(
            Channel::Input,
            &cbor::to_vec(&Value::Array(vec![Value::Int(5), fields]))
        )
        .is_err());
    }
    let mut raw = decode_raw_payload(&vector).unwrap();
    if let Value::Map(fields) = &mut raw.body {
        fields.push((Value::Int(99), Value::Bytes(vec![0; MAX_KEYBOARD_FRAME])));
    }
    let oversized = cbor::to_vec(&Value::Array(vec![Value::Int(5), raw.body]));
    assert!(matches!(
        decode_payload(Channel::Input, &oversized),
        Err(FrameError::FrameTooLarge { .. })
    ));
    let key = encode_payload(&Message::Input(SenderCore::new(0).keyboard(
        KeyboardAction::Key {
            code: 67,
            modifiers: 1,
        },
    )))
    .unwrap();
    assert_eq!(
        key,
        vec![0x82, 6, 0xa8, 0, 1, 1, 0x18, 0x43, 2, 0, 3, 0, 4, 1, 5, 0, 6, 0, 7, 1]
    );
    let mut raw = decode_raw_payload(&key).unwrap();
    if let Value::Map(fields) = &mut raw.body {
        fields
            .iter_mut()
            .find(|(key, _)| *key == Value::Int(7))
            .unwrap()
            .1 = Value::Int(8);
    }
    assert!(decode_payload(
        Channel::Input,
        &cbor::to_vec(&Value::Array(vec![Value::Int(6), raw.body]))
    )
    .is_err());
}

#[test]
fn keyboard_barriers_order_totals_unknown_keys_and_old_versions() {
    let mut sender = SenderCore::new(0);
    sender.pointer(100, 0, None);
    sender.scroll(0, 200, None);
    let text = sender.keyboard(action("ą😀ą")); // i18n: data
    let future = sender.pointer(50, 0, None);
    let mut out = vec![];
    let mut receiver = InputReceiver::with_version(true, 3);
    receiver.stream_started();
    receiver
        .stream_frame(&sender.input_session(0), &mut out)
        .unwrap();
    out.clear();
    receiver.datagram(&future, &mut out);
    assert!(out.is_empty());
    receiver.stream_frame(&text, &mut out).unwrap();
    assert!(matches!(
        out.as_slice(),
        [
            InputEvent::Pointer { dx: 100, .. },
            InputEvent::Scroll { dy: 200, .. },
            InputEvent::Keyboard(_),
            InputEvent::Pointer { dx: 50, .. }
        ]
    ));
    assert_eq!(out[2], InputEvent::Keyboard(action("ą😀ą"))); // i18n: data
    out.clear();
    receiver
        .stream_frame(
            &sender.keyboard(KeyboardAction::Key {
                code: 9999,
                modifiers: 0,
            }),
            &mut out,
        )
        .unwrap();
    assert!(out.is_empty());
    receiver
        .stream_frame(
            &sender.keyboard(KeyboardAction::Key {
                code: 9,
                modifiers: 2,
            }),
            &mut out,
        )
        .unwrap();
    assert_eq!(
        out,
        vec![InputEvent::Keyboard(KeyboardAction::Key {
            code: 9,
            modifiers: 2
        })]
    );
    assert_eq!(
        receiver.stream_frame(&text, &mut out),
        Err(ErrorCode::ProtocolViolation)
    );
    for version in [1, 2] {
        let mut old = InputReceiver::with_version(true, version);
        old.stream_started();
        old.stream_frame(&sender.input_session(0), &mut vec![])
            .unwrap();
        assert_eq!(
            old.stream_frame(&text, &mut vec![]),
            Err(ErrorCode::ProtocolViolation)
        );
    }
}

/// CCS/1 TEST:input/4 Laser is a reliable barrier with a fixed vector; older receivers refuse it.
#[test]
fn laser_vector_barrier_and_old_versions() {
    let mut sender = SenderCore::new(0);
    let on = SenderCore::new(0).laser(true);
    let vector = encode_payload(&Message::Input(on.clone())).unwrap();
    assert_eq!(
        vector,
        vec![0x82, 7, 0xa7, 0, 1, 1, 0xf5, 2, 0, 3, 0, 4, 1, 5, 0, 6, 0]
    );
    assert_eq!(
        decode_payload(Channel::Input, &vector).unwrap(),
        Message::Input(on.clone())
    );
    assert!(encode_datagram(&on).is_err());
    sender.pointer(70, 0, None);
    let laser = sender.laser(true);
    let future = sender.pointer(30, -10, None);
    let off = sender.laser(false);
    let mut out = vec![];
    let mut receiver = InputReceiver::with_version(true, 4);
    receiver.stream_started();
    receiver
        .stream_frame(&sender.input_session(0), &mut out)
        .unwrap();
    out.clear();
    receiver.datagram(&future, &mut out);
    assert!(out.is_empty());
    receiver.stream_frame(&laser, &mut out).unwrap();
    assert!(matches!(
        out.as_slice(),
        [
            InputEvent::Pointer { dx: 70, .. },
            InputEvent::Laser(true),
            InputEvent::Pointer {
                dx: 30,
                dy: -10,
                ..
            }
        ]
    ));
    out.clear();
    receiver.stream_frame(&off, &mut out).unwrap();
    assert_eq!(out, vec![InputEvent::Laser(false)]);
    assert_eq!(
        receiver.stream_frame(&laser, &mut out),
        Err(ErrorCode::ProtocolViolation)
    );
    for version in [1, 2, 3] {
        let mut old = InputReceiver::with_version(true, version);
        old.stream_started();
        old.stream_frame(&sender.input_session(0), &mut vec![])
            .unwrap();
        assert_eq!(
            old.stream_frame(&laser, &mut vec![]),
            Err(ErrorCode::ProtocolViolation)
        );
    }
}
