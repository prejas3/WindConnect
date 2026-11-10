//! `actions` payloads (§9.2) from a paired device: no panic; whatever is
//! accepted round-trips; an accepted `Run` names a well-formed id; the URL
//! check never panics and accepts only http(s).
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::actions::{check_url, is_action_id, Reply, Request};
use wind_core::protocol::decode_raw_payload;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = decode_raw_payload(data) {
        if let Ok(Some(request)) = Request::decode(raw.clone()) {
            if let Request::Run { action, .. } = &request {
                assert!(is_action_id(action));
            }
            assert_eq!(Request::decode(request.encode()), Ok(Some(request)));
        }
        if let Ok(Some(reply)) = Reply::decode(raw) {
            assert_eq!(Reply::decode(reply.encode()), Ok(Some(reply)));
        }
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if check_url(text) {
            let lower = text.to_ascii_lowercase();
            assert!(lower.starts_with("http://") || lower.starts_with("https://"));
            assert!(text.bytes().all(|b| (0x21..=0x7e).contains(&b)));
        }
    }
});
