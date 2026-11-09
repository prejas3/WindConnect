//! Arbitrary text as a pairing link (PROTOCOL.md §12.4), as a QR scanner
//! or another app may hand it over. Parsing never panics; a link that
//! parses writes back to a link that parses to the same offer, with a
//! clean name.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::pairing::wire::MAX_LINK_ADDRS;
use wind_core::pairing::{clean_name, PairingOffer};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // Also reach past the prefix check most of the time.
    let link = if text.starts_with("windconnect:") {
        text.to_string()
    } else {
        format!("windconnect:pair?{text}")
    };
    let Ok(offer) = PairingOffer::parse(&link) else {
        return;
    };
    assert!(!offer.addrs.is_empty() && offer.addrs.len() <= MAX_LINK_ADDRS);
    if let Some(name) = offer.name.as_deref() {
        assert_eq!(clean_name(name).as_deref(), Some(name));
    }
    let again = PairingOffer::parse(&offer.to_link()).expect("a written link parses");
    assert_eq!(again.key, offer.key);
    assert_eq!(again.secret(), offer.secret());
    assert_eq!(again.addrs, offer.addrs);
    assert_eq!(again.name, offer.name);
});
