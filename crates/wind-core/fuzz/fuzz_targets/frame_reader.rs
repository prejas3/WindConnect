//! Arbitrary bytes as a stream on any channel, delivered in arbitrary
//! chunks. The reader never buffers more than the frame limit, and every
//! message read must re-encode and read back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::protocol::{decode_stream_header, encode_frame, frame_limit, FrameReader, Message};

fn check(msg: &Message) {
    let frame = encode_frame(msg).expect("a decoded message re-encodes");
    let mut r = FrameReader::new(msg.channel());
    let mut rest = &frame[..];
    assert_eq!(
        r.read(&mut rest).expect("and reads back").as_ref(),
        Some(msg)
    );
    assert!(rest.is_empty());
    r.finish().expect("with nothing left over");
}

fuzz_target!(|data: &[u8]| {
    let [selector, chunking, rest @ ..] = data else {
        return;
    };
    // The first byte picks a channel through the stream-header check, so
    // unknown channels and versions get exercised too.
    let Ok(channel) = decode_stream_header([1, selector % 8]) else {
        return;
    };
    let max = frame_limit(channel, false).max(frame_limit(channel, true)) + 8;
    let mut reader = FrameReader::new(channel);
    for piece in rest.chunks((*chunking as usize).max(1)) {
        let mut piece = piece;
        loop {
            match reader.read(&mut piece) {
                Ok(Some(msg)) => check(&msg),
                Ok(None) => break,
                Err(e) if e.is_fatal() => return,
                Err(_) => {}
            }
            assert!(reader.buffered() <= max);
        }
    }
    let _ = reader.finish();
});
