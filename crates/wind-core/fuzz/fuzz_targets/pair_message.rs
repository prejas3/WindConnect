//! Arbitrary bytes as pairing frames (PROTOCOL.md §12.3), read from an
//! unauthenticated peer. Reading never panics, never accepts a frame over
//! the limit, every message read re-encodes and reads back the same, and
//! every name read is already clean.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::pairing::wire::{decode_frame, PairMessage};
use wind_core::pairing::{clean_name, MAX_NAME, MAX_PAIR_FRAME};

fn name_of(msg: &PairMessage) -> Option<&str> {
    match msg {
        PairMessage::Hello { name, .. }
        | PairMessage::Nonce { name, .. }
        | PairMessage::Result { name, .. } => name.as_deref(),
        _ => None,
    }
}

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    while let Ok(Some((msg, used))) = decode_frame(rest) {
        assert!(used >= 2 && used <= MAX_PAIR_FRAME + 2 && used <= rest.len());
        if let Some(name) = name_of(&msg) {
            assert!(name.len() <= MAX_NAME);
            assert_eq!(clean_name(name).as_deref(), Some(name));
        }
        let frame = msg.encode();
        let (back, n) = decode_frame(&frame)
            .expect("a read message re-encodes")
            .expect("as a whole frame");
        assert_eq!(back, msg);
        assert_eq!(n, frame.len());
        rest = &rest[used..];
    }
});
