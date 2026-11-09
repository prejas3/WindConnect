//! Arbitrary bytes as an `input` datagram; anything accepted round-trips.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::protocol::{decode_datagram, encode_datagram, MAX_DATAGRAM};

fuzz_target!(|data: &[u8]| {
    if let Ok(msg) = decode_datagram(data, MAX_DATAGRAM) {
        let bytes = encode_datagram(&msg).expect("a decoded datagram re-encodes");
        assert_eq!(decode_datagram(&bytes, MAX_DATAGRAM), Ok(msg));
    }
});
