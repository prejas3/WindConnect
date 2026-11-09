//! Arbitrary bytes as CBOR: whatever the strict decoder accepts is already
//! in deterministic form, so re-encoding gives back exactly the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::protocol::cbor;

fuzz_target!(|data: &[u8]| {
    if let Ok(value) = cbor::decode(data) {
        assert_eq!(cbor::to_vec(&value), data);
    }
});
