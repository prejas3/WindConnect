//! WN-62: every feature channel's payload decoder (§9) on the same input,
//! plus the broadcast discovery packet (§2.0.1) and the capability grammar
//! (§6), all of which read bytes another device chose. No panic; whatever a
//! decoder accepts encodes again and decodes to the same value.
#![no_main]

use libfuzzer_sys::fuzz_target;
use wind_core::protocol::{decode_raw_payload, Capability, RawMessage};
use wind_core::{
    battery, broadcast, call, clipboard, files, find, gallery, media, notify, sms, volume,
};

/// `decode(encode(x)) == x` for a value `decode` accepted.
macro_rules! round_trip {
    ($raw:expr, $ty:ty) => {
        if let Some(value) = <$ty>::decode($raw) {
            assert_eq!(
                <$ty>::decode(&value.encode()).as_ref(),
                Some(&value),
                stringify!($ty)
            );
        }
    };
}

fn features(raw: &RawMessage) {
    round_trip!(raw, battery::Status);
    round_trip!(raw, call::CallState);
    round_trip!(raw, call::Message);
    round_trip!(raw, call::Action);
    round_trip!(raw, clipboard::Message);
    round_trip!(raw, find::Request);
    round_trip!(raw, find::State);
    round_trip!(raw, gallery::Request);
    round_trip!(raw, gallery::Reply);
    round_trip!(raw, media::Action);
    round_trip!(raw, media::Message);
    round_trip!(raw, notify::Action);
    round_trip!(raw, notify::Message);
    round_trip!(raw, sms::Request);
    round_trip!(raw, sms::Reply);
    round_trip!(raw, volume::Message);
    if let Ok(Some(m)) = files::wire::ToReceiver::decode(raw.clone()) {
        assert_eq!(files::wire::ToReceiver::decode(m.encode()), Ok(Some(m)));
    }
    if let Ok(Some(m)) = files::wire::ToSender::decode(raw.clone()) {
        assert_eq!(files::wire::ToSender::decode(m.encode()), Ok(Some(m)));
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = decode_raw_payload(data) {
        features(&raw);
    }
    if let Some(packet) = broadcast::Packet::decode(data) {
        assert_ne!(packet.port, 0);
        assert_eq!(broadcast::Packet::decode(&packet.encode()), Some(packet));
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if let Some(cap) = Capability::parse(text) {
            assert_eq!(
                Capability::parse(&format!("{}/{}", cap.name(), cap.version())),
                Some(cap)
            );
        }
    }
});
