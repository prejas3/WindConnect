//! QUIC variable-length integers (RFC 9000 §16), used for frame lengths.

use super::FrameError;

/// Largest value a QUIC varint can hold.
pub const MAX_VARINT: u64 = (1 << 62) - 1;

/// Appends `v` in its shortest encoding.
///
/// # Panics
///
/// If `v` exceeds [`MAX_VARINT`].
pub fn encode(v: u64, out: &mut Vec<u8>) {
    assert!(v <= MAX_VARINT, "varint out of range");
    if v < 1 << 6 {
        out.push(v as u8);
    } else if v < 1 << 14 {
        out.extend_from_slice(&((v as u16) | 0x4000).to_be_bytes());
    } else if v < 1 << 30 {
        out.extend_from_slice(&((v as u32) | 0x8000_0000).to_be_bytes());
    } else {
        out.extend_from_slice(&(v | 0xc000_0000_0000_0000).to_be_bytes());
    }
}

/// Decodes a varint at the start of `buf`.
///
/// `Ok(None)`: not all of its bytes are here yet. `Ok(Some((value, len)))`:
/// the value and how many bytes it took. A longer encoding than the value
/// needs is `MALFORMED_FRAME` (PROTOCOL.md §4).
pub fn decode(buf: &[u8]) -> Result<Option<(u64, usize)>, FrameError> {
    let Some(&first) = buf.first() else {
        return Ok(None);
    };
    let len = 1usize << (first >> 6);
    if buf.len() < len {
        return Ok(None);
    }
    let mut v = (first & 0x3f) as u64;
    for b in &buf[1..len] {
        v = (v << 8) | *b as u64;
    }
    let shortest = match v {
        0..=0x3f => 1,
        0x40..=0x3fff => 2,
        0x4000..=0x3fff_ffff => 4,
        _ => 8,
    };
    if len != shortest {
        return Err(FrameError::Malformed(
            "frame length not in its shortest encoding",
        ));
    }
    Ok(Some((v, len)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        encode(v, &mut out);
        out
    }

    #[test]
    fn spec_examples() {
        assert_eq!(enc(6), [0x06]);
        assert_eq!(enc(300), [0x41, 0x2c]);
        assert_eq!(enc(70_000), [0x80, 0x01, 0x11, 0x70]);
    }

    #[test]
    fn round_trips_at_boundaries() {
        for v in [0, 63, 64, 16383, 16384, (1 << 30) - 1, 1 << 30, MAX_VARINT] {
            let bytes = enc(v);
            assert_eq!(decode(&bytes).unwrap(), Some((v, bytes.len())));
            assert_eq!(decode(&bytes[..bytes.len() - 1]).unwrap(), None);
        }
    }

    #[test]
    fn rejects_longer_encodings() {
        assert!(decode(&[0x40, 0x06]).is_err());
        assert!(decode(&[0x80, 0x00, 0x01, 0x2c]).is_err());
        assert!(decode(&[0xc0, 0, 0, 0, 0, 0x01, 0x11, 0x70]).is_err());
    }
}
