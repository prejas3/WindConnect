//! The CBOR subset of PROTOCOL.md §4.1.
//!
//! The decoder accepts only deterministic encoding (RFC 8949 §4.2.1):
//! shortest arguments, definite lengths, map keys strictly ascending by
//! their encoded bytes (so no repeated keys), integers within i64 / u64,
//! byte and text strings, arrays, maps, `false`, `true` and `null`, at most
//! [`MAX_DEPTH`] levels of arrays and maps, and exactly one item with no
//! trailing bytes. Everything else is an error. The encoder produces
//! exactly this form, so `encode(decode(x)) == x` for every accepted `x`.

/// Deepest nesting of arrays and maps a payload may have.
pub const MAX_DEPTH: usize = 16;

/// A decoded CBOR item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// An integer in `i64::MIN..=u64::MAX`.
    Int(i128),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    /// Pairs in the order they are encoded (ascending by encoded key).
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}

/// Why a payload is not in the accepted subset.
pub type CborError = &'static str;

const MAJOR_UINT: u8 = 0;
const MAJOR_NINT: u8 = 1;
const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_ARRAY: u8 = 4;
const MAJOR_MAP: u8 = 5;
const MAJOR_TAG: u8 = 6;
const MAJOR_SIMPLE: u8 = 7;

/// Decodes exactly one CBOR item filling all of `input`.
pub fn decode(input: &[u8]) -> Result<Value, CborError> {
    let mut d = Decoder { buf: input, pos: 0 };
    let value = d.item(0)?;
    if d.pos != input.len() {
        return Err("trailing bytes after the CBOR item");
    }
    Ok(value)
}

/// Encodes `value` deterministically, appending to `out`.
pub fn encode(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Int(v) => {
            if *v >= 0 {
                head(out, MAJOR_UINT, *v as u64);
            } else {
                head(out, MAJOR_NINT, (-1 - *v) as u64);
            }
        }
        Value::Bytes(b) => {
            head(out, MAJOR_BYTES, b.len() as u64);
            out.extend_from_slice(b);
        }
        Value::Text(t) => {
            head(out, MAJOR_TEXT, t.len() as u64);
            out.extend_from_slice(t.as_bytes());
        }
        Value::Array(items) => {
            head(out, MAJOR_ARRAY, items.len() as u64);
            for item in items {
                encode(item, out);
            }
        }
        Value::Map(pairs) => {
            let mut encoded: Vec<(Vec<u8>, &Value)> = pairs
                .iter()
                .map(|(k, v)| {
                    let mut kb = Vec::new();
                    encode(k, &mut kb);
                    (kb, v)
                })
                .collect();
            encoded.sort_by(|a, b| a.0.cmp(&b.0));
            debug_assert!(
                encoded.windows(2).all(|w| w[0].0 != w[1].0),
                "map with repeated keys"
            );
            head(out, MAJOR_MAP, encoded.len() as u64);
            for (kb, v) in encoded {
                out.extend_from_slice(&kb);
                encode(v, out);
            }
        }
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Null => out.push(0xf6),
    }
}

/// Encodes `value` into a new buffer.
pub fn to_vec(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode(value, &mut out);
    out
}

fn head(out: &mut Vec<u8>, major: u8, arg: u64) {
    let m = major << 5;
    if arg < 24 {
        out.push(m | arg as u8);
    } else if arg <= u8::MAX as u64 {
        out.extend_from_slice(&[m | 24, arg as u8]);
    } else if arg <= u16::MAX as u64 {
        out.push(m | 25);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= u32::MAX as u64 {
        out.push(m | 26);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

struct Decoder<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: u64) -> Result<&'a [u8], CborError> {
        if n > self.remaining() as u64 {
            return Err("CBOR item runs past the end of the payload");
        }
        let n = n as usize;
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn uint_be(&mut self, n: u64) -> Result<u64, CborError> {
        Ok(self
            .take(n)?
            .iter()
            .fold(0u64, |acc, b| (acc << 8) | *b as u64))
    }

    fn item(&mut self, depth: usize) -> Result<Value, CborError> {
        let initial = self.take(1)?[0];
        let major = initial >> 5;
        let info = initial & 0x1f;

        if major == MAJOR_SIMPLE {
            return match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                25..=27 => Err("floating-point numbers are not allowed"),
                31 => Err("indefinite lengths are not allowed"),
                _ => Err("only false, true and null simple values are allowed"),
            };
        }
        if major == MAJOR_TAG {
            return Err("tags are not allowed");
        }

        let arg = match info {
            0..=23 => info as u64,
            24 => {
                let v = self.uint_be(1)?;
                if v < 24 {
                    return Err("non-shortest CBOR argument");
                }
                v
            }
            25 => {
                let v = self.uint_be(2)?;
                if v <= u8::MAX as u64 {
                    return Err("non-shortest CBOR argument");
                }
                v
            }
            26 => {
                let v = self.uint_be(4)?;
                if v <= u16::MAX as u64 {
                    return Err("non-shortest CBOR argument");
                }
                v
            }
            27 => {
                let v = self.uint_be(8)?;
                if v <= u32::MAX as u64 {
                    return Err("non-shortest CBOR argument");
                }
                v
            }
            31 => return Err("indefinite lengths are not allowed"),
            _ => return Err("reserved CBOR additional information"),
        };

        match major {
            MAJOR_UINT => Ok(Value::Int(arg as i128)),
            MAJOR_NINT => {
                if arg > i64::MAX as u64 {
                    return Err("negative integer below i64::MIN");
                }
                Ok(Value::Int(-1 - arg as i128))
            }
            MAJOR_BYTES => Ok(Value::Bytes(self.take(arg)?.to_vec())),
            MAJOR_TEXT => {
                let raw = self.take(arg)?;
                let text = std::str::from_utf8(raw).map_err(|_| "text is not valid UTF-8")?;
                Ok(Value::Text(text.to_owned()))
            }
            MAJOR_ARRAY => {
                let depth = depth + 1;
                if depth > MAX_DEPTH {
                    return Err("arrays and maps nested too deeply");
                }
                // Each item takes at least one byte: this bounds allocation.
                if arg > self.remaining() as u64 {
                    return Err("CBOR item runs past the end of the payload");
                }
                let mut items = Vec::with_capacity(arg as usize);
                for _ in 0..arg {
                    items.push(self.item(depth)?);
                }
                Ok(Value::Array(items))
            }
            MAJOR_MAP => {
                let depth = depth + 1;
                if depth > MAX_DEPTH {
                    return Err("arrays and maps nested too deeply");
                }
                if arg > (self.remaining() / 2) as u64 {
                    return Err("CBOR item runs past the end of the payload");
                }
                let mut pairs = Vec::with_capacity(arg as usize);
                let mut prev_key: Option<&[u8]> = None;
                for _ in 0..arg {
                    let start = self.pos;
                    let key = self.item(depth)?;
                    let key_bytes = &self.buf[start..self.pos];
                    if let Some(prev) = prev_key {
                        if key_bytes <= prev {
                            return Err("map keys repeated or not in canonical order");
                        }
                    }
                    prev_key = Some(key_bytes);
                    let value = self.item(depth)?;
                    pairs.push((key, value));
                }
                Ok(Value::Map(pairs))
            }
            _ => unreachable!("major type is three bits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(bytes: &[u8]) -> Value {
        let v = decode(bytes).expect("accepted");
        assert_eq!(to_vec(&v), bytes, "re-encoding differs");
        v
    }

    #[test]
    fn integers_round_trip_at_every_width() {
        for v in [
            0i128,
            23,
            24,
            255,
            256,
            65535,
            65536,
            u32::MAX as i128,
            u32::MAX as i128 + 1,
            u64::MAX as i128,
            -1,
            -24,
            -25,
            -256,
            -257,
            i64::MIN as i128,
        ] {
            let bytes = to_vec(&Value::Int(v));
            assert_eq!(rt(&bytes), Value::Int(v));
        }
        assert_eq!(to_vec(&Value::Int(-350)), [0x39, 0x01, 0x5d]);
        assert_eq!(to_vec(&Value::Int(5_000_123)), [0x1a, 0, 0x4c, 0x4b, 0xbb]);
    }

    #[test]
    fn rejects_non_shortest_arguments() {
        assert!(decode(&[0x18, 0x17]).is_err());
        assert!(decode(&[0x19, 0x00, 0xff]).is_err());
        assert!(decode(&[0x1a, 0x00, 0x00, 0xff, 0xff]).is_err());
        assert!(decode(&[0x1b, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(decode(&[0x78, 0x01, b'a']).is_err());
    }

    #[test]
    fn rejects_values_outside_the_subset() {
        assert!(
            decode(&[0x3b, 0x80, 0, 0, 0, 0, 0, 0, 0]).is_err(),
            "below i64"
        );
        assert!(decode(&[0xc0, 0x00]).is_err(), "tag");
        assert!(decode(&[0xf9, 0x3c, 0x00]).is_err(), "half float");
        assert!(decode(&[0xfb, 0, 0, 0, 0, 0, 0, 0, 0]).is_err(), "double");
        assert!(decode(&[0xf7]).is_err(), "undefined");
        assert!(decode(&[0xf0]).is_err(), "simple value");
        assert!(decode(&[0x9f, 0xff]).is_err(), "indefinite array");
        assert!(decode(&[0x5f, 0xff]).is_err(), "indefinite bytes");
        assert!(decode(&[0x1c]).is_err(), "reserved info");
        assert!(decode(&[0x62, 0xc3, 0x28]).is_err(), "invalid UTF-8");
        assert!(decode(&[0x00, 0x00]).is_err(), "trailing byte");
        assert!(decode(&[]).is_err(), "empty");
        assert!(decode(&[0x82, 0x00]).is_err(), "truncated array");
    }

    #[test]
    fn map_keys_must_be_strictly_ascending() {
        rt(&[0xa2, 0x00, 0x00, 0x01, 0x00]);
        assert!(
            decode(&[0xa2, 0x01, 0x00, 0x00, 0x00]).is_err(),
            "descending"
        );
        assert!(decode(&[0xa2, 0x00, 0x00, 0x00, 0x01]).is_err(), "repeated");
        // Bytewise order: 0x17 (23) sorts before 0x18 0x18 (24), and a
        // shorter text sorts before a longer one sharing its first byte.
        rt(&[0xa2, 0x17, 0xf6, 0x18, 0x18, 0xf6]);
        rt(&[0xa2, 0x61, b'b', 0xf6, 0x62, b'a', b'a', 0xf6]);
    }

    #[test]
    fn encoder_sorts_map_keys() {
        let v = Value::Map(vec![
            (Value::Int(24), Value::Null),
            (Value::Text("a".into()), Value::Null),
            (Value::Int(1), Value::Null),
        ]);
        let bytes = to_vec(&v);
        assert_eq!(
            bytes,
            [0xa3, 0x01, 0xf6, 0x18, 0x18, 0xf6, 0x61, b'a', 0xf6]
        );
        rt(&bytes);
    }

    #[test]
    fn nesting_is_limited_to_sixteen_levels() {
        let mut ok = vec![0x81; MAX_DEPTH - 1];
        ok.push(0x80);
        rt(&ok);
        let mut deep = vec![0x81; MAX_DEPTH];
        deep.push(0x80);
        assert!(decode(&deep).is_err());
    }

    #[test]
    fn huge_declared_lengths_fail_without_allocating() {
        assert!(decode(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(decode(&[0xbb, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(decode(&[0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
    }
}
