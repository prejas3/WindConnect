//! The frame codec against PROTOCOL.md: every example of §11 as a test
//! vector, round trips for every message type, limits and rejections.

use wind_core::protocol::*;

#[test]
fn gesture_v2_roundtrip_and_datagram_rejection() {
    for gesture in [
        Gesture::TaskView,
        Gesture::ShowDesktop,
        Gesture::PreviousApp,
        Gesture::NextApp,
        Gesture::Other(99),
    ] {
        let message = InputMessage::Gesture(InputGesture {
            seq: 5,
            gesture,
            x: -100,
            y: 200,
            b: 2,
            sx: 50,
            sy: -60,
        });
        let bytes = encode_payload(&Message::Input(message.clone())).unwrap();
        assert_eq!(
            decode_payload(Channel::Input, &bytes).unwrap(),
            Message::Input(message.clone())
        );
        assert!(encode_datagram(&message).is_err());
        let mut datagram = vec![1, 4];
        datagram.extend(bytes);
        assert!(decode_datagram(&datagram, MAX_DATAGRAM).is_err());
    }
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn caps(list: &[&str]) -> Vec<Capability> {
    list.iter().map(|c| Capability::parse(c).unwrap()).collect()
}

fn control(m: ControlMessage) -> Message {
    Message::Control(m)
}

fn input(m: InputMessage) -> Message {
    Message::Input(m)
}

/// A whole stream body (frames only) read in one piece.
fn read_all(channel: Channel, bytes: &[u8]) -> Result<Vec<Message>, FrameError> {
    let mut r = FrameReader::new(channel);
    let mut rest = bytes;
    let mut out = Vec::new();
    while let Some(m) = r.read(&mut rest)? {
        out.push(m);
    }
    assert!(rest.is_empty());
    r.finish()?;
    Ok(out)
}

/// `bytes` is exactly the frame of `msg`, and reads back as `msg`.
fn assert_frame(msg: Message, bytes: &[u8]) {
    assert_eq!(encode_frame(&msg).unwrap(), bytes, "{msg:?}");
    assert_eq!(read_all(msg.channel(), bytes).unwrap(), vec![msg]);
}

fn assert_datagram(msg: InputMessage, bytes: &[u8]) {
    assert_eq!(encode_datagram(&msg).unwrap(), bytes, "{msg:?}");
    assert_eq!(decode_datagram(bytes, MAX_DATAGRAM).unwrap(), msg);
}

fn szymon_pc_hello() -> Hello {
    Hello {
        version: Version { major: 1, minor: 0 },
        name: "SZYMON-PC".into(),
        kind: DeviceKind::Computer,
        caps: caps(&["clipboard/1", "input/1"]),
        app: Some("0.5.0".into()),
    }
}

// ---- §11: the spec's examples are the codec's test vectors -----------------

#[test]
fn vector_hello() {
    let bytes = hex("31 82 00 a5 00 82 01 00 01 69 53 5a 59 4d 4f 4e
         2d 50 43 02 00 03 82 6b 63 6c 69 70 62 6f 61 72
         64 2f 31 67 69 6e 70 75 74 2f 31 04 65 30 2e 35
         2e 30");
    assert_eq!(bytes.len(), 50);
    assert_frame(control(ControlMessage::Hello(szymon_pc_hello())), &bytes);
}

#[test]
fn vector_hello_ack() {
    let bytes = hex("2c 82 01 a5 00 82 01 00 01 67 50 69 78 65 6c 20
         39 02 01 03 82 67 69 6e 70 75 74 2f 31 68 6e 6f
         74 69 66 79 2f 31 04 65 30 2e 35 2e 30");
    assert_eq!(bytes.len(), 45);
    let ack = Hello {
        version: Version { major: 1, minor: 0 },
        name: "Pixel 9".into(),
        kind: DeviceKind::Phone,
        caps: caps(&["input/1", "notify/1"]),
        app: Some("0.5.0".into()),
    };
    assert_frame(control(ControlMessage::HelloAck(ack)), &bytes);
}

#[test]
fn vector_control_stream_start() {
    // §7.1: the client's header and Hello, read the way a server reads them.
    let mut stream = hex("01 00");
    stream.extend(encode_frame(&control(ControlMessage::Hello(szymon_pc_hello()))).unwrap());
    assert_eq!(&stream[..4], hex("01 00 31 82"));
    let channel = decode_stream_header([stream[0], stream[1]]).unwrap();
    assert_eq!(channel, Channel::Control);
    assert_eq!(
        read_all(channel, &stream[2..]).unwrap(),
        vec![control(ControlMessage::Hello(szymon_pc_hello()))]
    );
}

#[test]
fn vector_ping_pong() {
    assert_frame(
        control(ControlMessage::Ping { nonce: 42 }),
        &hex("06 82 02 a1 00 18 2a"),
    );
    assert_frame(
        control(ControlMessage::Pong { nonce: 42 }),
        &hex("06 82 03 a1 00 18 2a"),
    );
}

#[test]
fn vector_capabilities() {
    assert_frame(
        control(ControlMessage::Capabilities {
            caps: caps(&["input/1"]),
        }),
        &hex("0d 82 04 a1 00 81 67 69 6e 70 75 74 2f 31"),
    );
}

#[test]
fn vector_error() {
    let bytes = hex("17 82 05 a4 00 06 01 6c 75 6e 6b 6e 6f 77 6e 20
         74 79 70 65 02 00 03 09");
    assert_eq!(bytes.len(), 24);
    assert_frame(
        control(ControlMessage::Error(ErrorMessage {
            code: ErrorCode::UnknownMessage,
            message: Some("unknown type".into()),
            channel: Some(0),
            ty: Some(9),
        })),
        &bytes,
    );
}

#[test]
fn vector_goodbye() {
    assert_frame(
        control(ControlMessage::Goodbye(Goodbye {
            code: ErrorCode::GoingAway,
            message: None,
        })),
        &hex("05 82 06 a1 00 0d"),
    );
}

#[test]
fn vector_pointer_motion_datagrams() {
    let first = hex("01 04 82 00 a6 00 01 01 39 01 5d 02 18 78 03 1a
         00 4c 4b bb 04 00 05 00");
    assert_eq!(first.len(), 24);
    assert_datagram(
        InputMessage::PointerMotion(Motion {
            seq: 1,
            x: -350,
            y: 120,
            t: Some(5_000_123),
            s: 0,
            b: 0,
        }),
        &first,
    );

    let drag = hex("01 04 82 00 a5 00 03 01 39 01 35 02 18 5f 04 00 05 01");
    assert_eq!(drag.len(), 18);
    assert_datagram(
        InputMessage::PointerMotion(Motion {
            seq: 3,
            x: -310,
            y: 95,
            t: None,
            s: 0,
            b: 1,
        }),
        &drag,
    );
}

#[test]
fn vector_scroll_datagram() {
    let bytes = hex("01 04 82 01 a5 00 04 01 00 02 19 09 60 04 00 05 01");
    assert_eq!(bytes.len(), 17);
    assert_datagram(
        InputMessage::Scroll(Motion {
            seq: 4,
            x: 0,
            y: 2400,
            t: None,
            s: 0,
            b: 1,
        }),
        &bytes,
    );
}

#[test]
fn vector_pointer_button() {
    let bytes = hex("16 82 02 a8 00 02 01 00 02 f5 03 39 01 5d 04 18
         78 05 01 06 00 07 00");
    assert_eq!(bytes.len(), 23);
    assert_frame(
        input(InputMessage::PointerButton(PointerButton {
            seq: 2,
            button: Button::Left,
            down: true,
            x: -350,
            y: 120,
            b: 1,
            sx: 0,
            sy: 0,
        })),
        &bytes,
    );
}

#[test]
fn vector_input_stream_start() {
    let stream = hex("01 04 07 82 03 a2 00 00 01 00");
    assert_eq!(stream_header(Channel::Input), [stream[0], stream[1]]);
    assert_eq!(
        decode_stream_header([stream[0], stream[1]]),
        Ok(Channel::Input)
    );
    assert_frame(
        input(InputMessage::InputSession(InputSession { s: 0, g: 0 })),
        &stream[2..],
    );
}

// ---- round trips of every type, at the edges of their fields --------------

fn round_trip(msg: Message) {
    let frame = encode_frame(&msg).unwrap();
    assert_eq!(read_all(msg.channel(), &frame).unwrap(), vec![msg.clone()]);
    // Also byte by byte, as a stream may deliver it.
    let mut r = FrameReader::new(msg.channel());
    let mut got = Vec::new();
    for b in &frame {
        r.push(std::slice::from_ref(b)).unwrap();
        while let Some(m) = r.next_message().unwrap() {
            got.push(m);
        }
    }
    r.finish().unwrap();
    assert_eq!(got, vec![msg]);
}

#[test]
fn round_trip_every_control_type() {
    let long_caps: Vec<Capability> = (1..=MAX_CAPS as u64)
        .map(|n| Capability::new("x", n).unwrap())
        .collect();
    let messages = [
        ControlMessage::Hello(szymon_pc_hello()),
        ControlMessage::Hello(Hello {
            version: Version {
                major: 1,
                minor: u64::MAX,
            },
            name: "ż".repeat(32), // i18n: data
            kind: DeviceKind::Tablet,
            caps: long_caps.clone(),
            app: None,
        }),
        ControlMessage::HelloAck(Hello {
            version: Version { major: 1, minor: 3 },
            name: "x".into(),
            kind: DeviceKind::Phone,
            caps: vec![],
            app: Some("a".repeat(32)),
        }),
        ControlMessage::Ping { nonce: 0 },
        ControlMessage::Ping { nonce: u64::MAX },
        ControlMessage::Pong { nonce: 1 << 40 },
        ControlMessage::Capabilities { caps: vec![] },
        ControlMessage::Capabilities { caps: long_caps },
        ControlMessage::Error(ErrorMessage {
            code: ErrorCode::RateLimited,
            message: None,
            channel: None,
            ty: None,
        }),
        ControlMessage::Error(ErrorMessage {
            code: ErrorCode::CapabilityMissing,
            message: Some("é".repeat(128)),
            channel: Some(5),
            ty: Some(u64::MAX),
        }),
        ControlMessage::Goodbye(Goodbye {
            code: ErrorCode::NoError,
            message: Some(String::new()),
        }),
    ];
    for m in messages {
        round_trip(control(m));
    }
}

#[test]
fn round_trip_every_input_type() {
    let extreme = Motion {
        seq: u64::MAX,
        x: i64::MIN,
        y: i64::MAX,
        t: Some(u64::MAX),
        s: u32::MAX,
        b: u64::MAX,
    };
    let messages = [
        InputMessage::PointerMotion(extreme),
        InputMessage::Scroll(Motion {
            t: None,
            x: -1,
            y: 24,
            ..extreme
        }),
        InputMessage::PointerButton(PointerButton {
            seq: 7,
            button: Button::Middle,
            down: false,
            x: i64::MIN,
            y: -24,
            b: 2,
            sx: i64::MAX,
            sy: -25,
        }),
        InputMessage::PointerButton(PointerButton {
            seq: 8,
            button: Button::Other(9),
            down: true,
            x: 0,
            y: 0,
            b: 3,
            sx: 0,
            sy: 0,
        }),
        InputMessage::InputSession(InputSession {
            s: u32::MAX,
            g: u64::MAX,
        }),
    ];
    for m in messages {
        round_trip(input(m.clone()));
        if matches!(m, InputMessage::PointerMotion(_) | InputMessage::Scroll(_)) {
            let d = encode_datagram(&m).unwrap();
            assert_eq!(decode_datagram(&d, MAX_DATAGRAM).unwrap(), m);
        } else {
            assert!(encode_datagram(&m).is_err());
        }
    }
}

#[test]
fn several_frames_in_one_chunk() {
    let a = control(ControlMessage::Ping { nonce: 1 });
    let b = control(ControlMessage::Pong { nonce: 2 });
    let mut bytes = encode_frame(&a).unwrap();
    bytes.extend(encode_frame(&b).unwrap());
    assert_eq!(read_all(Channel::Control, &bytes).unwrap(), vec![a, b]);
}

// ---- forward compatibility -------------------------------------------------

#[test]
fn unknown_body_keys_are_skipped() {
    // Ping with nonce 42 and an extra key 9: [1, "x"].
    let payload = hex("82 02 a2 00 18 2a 09 82 01 61 78");
    assert_eq!(
        decode_payload(Channel::Control, &payload).unwrap(),
        control(ControlMessage::Ping { nonce: 42 })
    );
}

#[test]
fn unknown_types_are_skipped_not_fatal() {
    // Control type 9, then a Ping: the reader reports the first and goes on.
    let mut bytes = hex("03 82 09 a0");
    bytes.extend(hex("06 82 02 a1 00 18 2a"));
    let mut r = FrameReader::new(Channel::Control);
    let mut rest = &bytes[..];
    let e = r.read(&mut rest).unwrap_err();
    assert_eq!(
        e,
        FrameError::UnknownMessage {
            channel: Channel::Control,
            ty: 9
        }
    );
    assert!(!e.is_fatal());
    assert_eq!(e.code(), ErrorCode::UnknownMessage);
    assert_eq!(
        r.read(&mut rest).unwrap(),
        Some(control(ControlMessage::Ping { nonce: 42 }))
    );
    assert!(rest.is_empty());
    r.finish().unwrap();

    // Feature channels have no types yet: every type is unknown.
    for ch in [
        Channel::Clipboard,
        Channel::Notify,
        Channel::Media,
        Channel::Files,
    ] {
        assert_eq!(
            decode_payload(ch, &hex("82 00 a0")),
            Err(FrameError::UnknownMessage { channel: ch, ty: 0 })
        );
    }
    // An unknown type must still be well-formed.
    assert!(matches!(
        decode_payload(Channel::Input, &hex("82 09 80")),
        Err(FrameError::Malformed(_))
    ));
}

#[test]
fn unknown_kind_reads_as_computer_and_unknown_code_as_internal() {
    let mut payload = hex("82 00 a4 00 82 01 00 01 61 78 02 18 63 03 80");
    let Message::Control(ControlMessage::Hello(h)) =
        decode_payload(Channel::Control, &payload).unwrap()
    else {
        panic!()
    };
    assert_eq!(h.kind, DeviceKind::Computer);
    payload = hex("82 06 a1 00 18 ff");
    assert_eq!(
        decode_payload(Channel::Control, &payload).unwrap(),
        control(ControlMessage::Goodbye(Goodbye {
            code: ErrorCode::InternalError,
            message: None
        }))
    );
}

// ---- rejections ------------------------------------------------------------

fn malformed(channel: Channel, payload: &str) {
    let r = decode_payload(channel, &hex(payload));
    assert!(
        matches!(r, Err(FrameError::Malformed(_))),
        "{payload}: {r:?}"
    );
}

#[test]
fn rejects_bad_message_shapes_and_fields() {
    use Channel::{Control, Input};
    malformed(Control, "a0"); // not an array
    malformed(Control, "81 02"); // one element
    malformed(Control, "83 02 a1 00 00 00"); // three elements
    malformed(Control, "82 20 a0"); // negative type
    malformed(Control, "82 02 80"); // body not a map
    malformed(Control, "82 02 a1 20 00"); // negative key
    malformed(Control, "82 02 a1 61 61 00"); // text key
    malformed(Control, "82 02 a0"); // Ping without nonce
    malformed(Control, "82 02 a1 00 20"); // negative nonce
    malformed(Control, "82 02 a1 00 61 61"); // text nonce
    malformed(Control, "82 02 a2 00 01 00 02"); // repeated key
    malformed(Control, "82 02 a2 01 00 00 01"); // keys out of order
    malformed(Control, "82 02 a1 00 18 05"); // non-shortest nonce
    malformed(Control, "82 02 a1 00 01 00"); // trailing byte
    malformed(Control, "82 02 bf 00 01 ff"); // indefinite map
    malformed(Control, "82 02 a1 00 c1 01"); // tagged nonce
    malformed(Control, "82 02 a1 00 f9 3c 00"); // float nonce
    malformed(
        Input,
        "82 00 a5 00 01 01 3b 80 00 00 00 00 00 00 00 02 00 04 00 05 00",
    ); // x < i64::MIN
    malformed(
        Input,
        "82 00 a5 00 01 01 1b 80 00 00 00 00 00 00 00 02 00 04 00 05 00",
    ); // x > i64::MAX
    malformed(Input, "82 03 a2 00 1b 00 00 00 01 00 00 00 00 01 00"); // s > u32::MAX
    malformed(
        Input,
        "82 02 a8 00 02 01 00 02 01 03 00 04 00 05 01 06 00 07 00",
    ); // down not bool
}

#[test]
fn rejects_hello_fields_outside_limits() {
    use Channel::Control;
    let ok = "82 00 a4 00 82 01 00 01 61 78 02 00 03 80";
    decode_payload(Control, &hex(ok)).unwrap();
    malformed(Control, "82 00 a4 00 82 01 00 01 60 02 00 03 80"); // empty name
    malformed(Control, "82 00 a4 00 81 01 01 61 78 02 00 03 80"); // version [1]
    malformed(Control, "82 00 a4 00 83 01 00 00 01 61 78 02 00 03 80"); // version [1,0,0]
    malformed(Control, "82 00 a3 00 82 01 00 01 61 78 02 00"); // no caps
    malformed(Control, "82 00 a4 00 82 01 00 01 61 78 02 00 03 81 61 78"); // cap "x"
    malformed(
        Control,
        "82 00 a4 00 82 01 00 01 61 78 02 00 03 82 63 78 2f 31 63 78 2f 31",
    ); // repeated cap
    malformed(Control, "82 00 a5 00 82 01 00 01 61 78 02 00 03 80 04 60"); // empty app

    let mut name = "82 00 a4 00 82 01 00 01 78 41".to_string();
    name.push_str(&" 61".repeat(65));
    name.push_str(" 02 00 03 80");
    malformed(Control, &name);

    let mut many = "82 04 a1 00 98 41".to_string();
    for n in 1..=65 {
        let cap = format!("x/{n}");
        many.push_str(&format!(" {:02x}", 0x60 + cap.len()));
        for b in cap.bytes() {
            many.push_str(&format!(" {b:02x}"));
        }
    }
    malformed(Control, &many);

    let msg = |name: String| {
        encode_frame(&control(ControlMessage::Hello(Hello {
            name,
            ..szymon_pc_hello()
        })))
    };
    assert!(msg(String::new()).is_err());
    assert!(msg("x".repeat(65)).is_err());
    assert!(msg("x".repeat(64)).is_ok());
    let error = |m: String| {
        encode_frame(&control(ControlMessage::Error(ErrorMessage {
            code: ErrorCode::InternalError,
            message: Some(m),
            channel: None,
            ty: None,
        })))
    };
    assert!(error("x".repeat(256)).is_ok());
    assert!(error("x".repeat(257)).is_err());
}

#[test]
fn frame_lengths_zero_or_non_shortest_are_malformed() {
    for bytes in [hex("00"), hex("40 06 82 02 a1 00 18 2a")] {
        let mut r = FrameReader::new(Channel::Control);
        assert!(matches!(r.push(&bytes), Err(FrameError::Malformed(_))));
        // The stream is dead from then on.
        assert!(r.next_message().is_err());
        assert!(r.push(&[]).is_err());
    }
}

#[test]
fn limits_are_enforced_before_the_payload_arrives() {
    // First control frame: 4 096 B.
    let mut r = FrameReader::new(Channel::Control);
    assert_eq!(
        r.push(&hex("50 01")),
        Err(FrameError::FrameTooLarge {
            len: 4097,
            max: MAX_HELLO_FRAME
        })
    );
    let mut r = FrameReader::new(Channel::Control);
    assert_eq!(r.push(&hex("50 00")), Ok(2)); // 4 096: waits for the payload
    assert_eq!(r.next_message(), Ok(None));
    assert!(r.finish().is_err(), "FIN inside a frame");

    // Later control frames: 16 384 B.
    let mut r = FrameReader::new(Channel::Control);
    r.push(&hex("06 82 02 a1 00 18 2a")).unwrap();
    r.next_message().unwrap().unwrap();
    r.push(&hex("7f ff")).unwrap(); // 16 383
    let mut r2 = FrameReader::new(Channel::Control);
    r2.push(&hex("06 82 02 a1 00 18 2a")).unwrap();
    r2.next_message().unwrap().unwrap();
    assert_eq!(
        r2.push(&hex("80 00 40 01")),
        Err(FrameError::FrameTooLarge {
            len: 16385,
            max: MAX_CONTROL_FRAME
        })
    );

    // Other streams: 1 MiB, also for their first frame.
    let mut r = FrameReader::new(Channel::Input);
    r.push(&hex("80 10 00 00")).unwrap();
    let mut r = FrameReader::new(Channel::Clipboard);
    assert_eq!(
        r.push(&hex("80 10 00 01")),
        Err(FrameError::FrameTooLarge {
            len: (1 << 20) + 1,
            max: MAX_STREAM_FRAME
        })
    );
    assert_eq!(
        r.next_message().unwrap_err().code(),
        ErrorCode::FrameTooLarge
    );
}

#[test]
fn a_late_hello_over_its_own_limit_is_too_large() {
    // A Hello as a later control frame still has the 4 096 B limit.
    let caps: Vec<Capability> = (1..=MAX_CAPS as u64)
        .map(|n| Capability::new(&"c".repeat(24), n).unwrap())
        .collect();
    let mut hello = Hello {
        caps,
        app: None,
        ..szymon_pc_hello()
    };
    hello.name = "n".repeat(64);
    let msg = control(ControlMessage::Hello(hello));
    let payload = encode_payload(&msg).unwrap();
    assert!(payload.len() <= MAX_HELLO_FRAME, "fits: {}", payload.len());

    // Build an over-limit Hello by hand: unknown keys pad it past 4 096 B.
    let mut big = hex("82 00 a5 00 82 01 00 01 61 78 02 00 03 80 05 59 10 00");
    big.extend(std::iter::repeat_n(0u8, 4096));
    let mut frame = Vec::new();
    varint::encode(big.len() as u64, &mut frame);
    frame.extend(&big);
    let mut stream = encode_frame(&control(ControlMessage::Ping { nonce: 1 })).unwrap();
    stream.extend(&frame);
    let mut r = FrameReader::new(Channel::Control);
    let mut rest = &stream[..];
    r.read(&mut rest).unwrap().unwrap();
    assert!(matches!(
        r.read(&mut rest),
        Err(FrameError::FrameTooLarge {
            max: MAX_HELLO_FRAME,
            ..
        })
    ));
    assert!(encode_frame(&msg).is_ok());
}

#[test]
fn rejects_bad_datagrams() {
    let good = encode_datagram(&InputMessage::Scroll(Motion {
        seq: 1,
        x: 0,
        y: 0,
        t: None,
        s: 0,
        b: 0,
    }))
    .unwrap();
    assert!(decode_datagram(&good, MAX_DATAGRAM).is_ok());

    let mut v2 = good.clone();
    v2[0] = 2;
    assert_eq!(
        decode_datagram(&v2, MAX_DATAGRAM),
        Err(FrameError::UnsupportedVersion(2))
    );
    let mut ch = good.clone();
    ch[1] = 0;
    assert_eq!(
        decode_datagram(&ch, MAX_DATAGRAM),
        Err(FrameError::UnknownChannel(0))
    );
    assert!(decode_datagram(&[0x01], MAX_DATAGRAM).is_err());
    assert!(decode_datagram(&[], MAX_DATAGRAM).is_err());
    // A length prefix is not part of a datagram.
    let mut prefixed = vec![0x01, 0x04, (good.len() - 2) as u8];
    prefixed.extend(&good[2..]);
    assert!(decode_datagram(&prefixed, MAX_DATAGRAM).is_err());
    // Stream-only types are not accepted in datagrams.
    assert!(decode_datagram(&hex("01 04 82 03 a2 00 00 01 00"), MAX_DATAGRAM).is_err());
    // Size: the smaller of the announced limit and 1 200 B.
    assert_eq!(
        decode_datagram(&good, good.len() - 1),
        Err(FrameError::FrameTooLarge {
            len: good.len() as u64,
            max: good.len() - 1
        })
    );
    let mut huge = good.clone();
    huge.resize(1201, 0);
    assert_eq!(
        decode_datagram(&huge, usize::MAX),
        Err(FrameError::FrameTooLarge {
            len: 1201,
            max: MAX_DATAGRAM
        })
    );
}

#[test]
fn error_codes_of_failures() {
    assert_eq!(FrameError::UnsupportedVersion(2).code().code(), 0x02);
    assert_eq!(
        FrameError::FrameTooLarge { len: 1, max: 0 }.code().code(),
        0x03
    );
    assert_eq!(FrameError::Malformed("x").code().code(), 0x04);
    assert_eq!(FrameError::UnknownChannel(9).code().code(), 0x05);
}

#[test]
fn push_stops_at_each_frame_end() {
    let ping = hex("06 82 02 a1 00 18 2a");
    let mut two = ping.clone();
    two.extend(&ping);
    let mut r = FrameReader::new(Channel::Control);
    assert_eq!(r.push(&two), Ok(7), "takes only the first frame");
    assert_eq!(r.push(&two[7..]), Ok(0), "nothing until it is collected");
    assert_eq!(r.buffered(), 7);
    assert!(r.next_message().unwrap().is_some());
    assert_eq!(r.buffered(), 0);
    assert_eq!(r.push(&two[7..]), Ok(7));
    assert!(r.next_message().unwrap().is_some());
    r.finish().unwrap();
}

#[test]
fn an_oversized_frame_after_a_valid_one_in_the_same_chunk() {
    // A Ping, then a control frame declaring 16 385 B, then padding: the
    // Ping is read, the next length rejected, and the padding never taken.
    let mut chunk = hex("06 82 02 a1 00 18 2a 80 00 40 01");
    chunk.resize(chunk.len() + (1 << 20), 0);
    let mut r = FrameReader::new(Channel::Control);
    let mut rest = &chunk[..];
    assert!(r.read(&mut rest).unwrap().is_some());
    assert_eq!(
        r.read(&mut rest),
        Err(FrameError::FrameTooLarge {
            len: 16385,
            max: MAX_CONTROL_FRAME
        })
    );
    // A failed read leaves `rest` at the oversized frame; nothing is kept.
    assert_eq!(rest.len(), 4 + (1 << 20));
    assert_eq!(r.buffered(), 0);
}

#[test]
fn many_legal_frames_in_one_large_chunk_are_all_read() {
    let ping = encode_frame(&control(ControlMessage::Ping { nonce: 7 })).unwrap();
    let chunk: Vec<u8> = ping
        .iter()
        .copied()
        .cycle()
        .take(ping.len() * 50_000)
        .collect();
    let mut r = FrameReader::new(Channel::Control);
    let mut rest = &chunk[..];
    let mut n = 0;
    while let Some(m) = r.read(&mut rest).unwrap() {
        assert_eq!(m, control(ControlMessage::Ping { nonce: 7 }));
        assert!(r.buffered() == 0);
        n += 1;
    }
    assert_eq!(n, 50_000);
    r.finish().unwrap();
}
