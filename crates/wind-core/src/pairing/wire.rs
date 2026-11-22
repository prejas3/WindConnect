//! Pairing messages and frames, device names, and the pairing link
//! (PROTOCOL.md §12.2–§12.4).
//!
//! Everything here is read from a peer that is not authenticated yet, so
//! every limit is checked before anything is buffered or shown: a frame is
//! at most [`MAX_PAIR_FRAME`] bytes, a name at most [`MAX_NAME`] bytes and
//! cleaned of control and text-direction characters ([`clean_name`]).

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use zeroize::{Zeroize, Zeroizing};

use crate::identity::PublicKey;
use crate::protocol::cbor::{self, Value};
use crate::protocol::{decode_raw_payload, varint, FrameError};

/// Largest pairing frame payload.
pub const MAX_PAIR_FRAME: usize = 1024;
/// Largest device name on the wire and in a link, in UTF-8 bytes.
pub const MAX_NAME: usize = 64;
/// Bytes of the pairing secret in a link.
pub const SECRET_LEN: usize = 16;
/// Bytes of a SAS nonce.
pub const NONCE_LEN: usize = 32;
/// Longest pairing link accepted.
pub const MAX_LINK: usize = 1024;
/// Most addresses in a pairing link.
pub const MAX_LINK_ADDRS: usize = 8;
/// Scheme and path of a pairing link.
pub const LINK_PREFIX: &str = "windconnect:pair?";

/// How a pairing attempt is authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The secret from the pairing link (QR).
    Qr,
    /// A 6-digit code both people compare (SAS).
    Sas,
}

impl Mode {
    fn number(self) -> u64 {
        match self {
            Mode::Qr => 0,
            Mode::Sas => 1,
        }
    }
}

/// A pairing message (§12.3). Proofs, nonces and MACs are wiped when the
/// message is dropped.
#[derive(Clone, PartialEq, Eq)]
pub enum PairMessage {
    /// 0, client → server: the mode, the client's name, and the proof: the
    /// client MAC (QR) or the nonce commitment (SAS).
    Hello {
        mode: Mode,
        name: Option<String>,
        proof: [u8; 32],
    },
    /// 1, server → client (SAS): the server's nonce and name.
    Nonce {
        nonce: [u8; NONCE_LEN],
        name: Option<String>,
    },
    /// 2, client → server (SAS): the nonce committed to in `Hello`.
    Reveal { nonce: [u8; NONCE_LEN] },
    /// 3, server → client (QR): the server MAC and the server's name.
    Result { mac: [u8; 32], name: Option<String> },
    /// 4, both ways (SAS): this side's person accepted the code or not.
    Confirm { accepted: bool },
}

impl Drop for PairMessage {
    fn drop(&mut self) {
        match self {
            PairMessage::Hello { proof, .. } => proof.zeroize(),
            PairMessage::Nonce { nonce, .. } | PairMessage::Reveal { nonce } => nonce.zeroize(),
            PairMessage::Result { mac, .. } => mac.zeroize(),
            PairMessage::Confirm { .. } => {}
        }
    }
}

impl fmt::Debug for PairMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Proofs and nonces are never printed.
        match self {
            PairMessage::Hello { mode, name, .. } => f
                .debug_struct("Hello")
                .field("mode", mode)
                .field("name", name)
                .finish_non_exhaustive(),
            PairMessage::Nonce { name, .. } => f
                .debug_struct("Nonce")
                .field("name", name)
                .finish_non_exhaustive(),
            PairMessage::Reveal { .. } => f.debug_struct("Reveal").finish_non_exhaustive(),
            PairMessage::Result { name, .. } => f
                .debug_struct("Result")
                .field("name", name)
                .finish_non_exhaustive(),
            PairMessage::Confirm { accepted } => f
                .debug_struct("Confirm")
                .field("accepted", accepted)
                .finish(),
        }
    }
}

fn int(n: u64) -> Value {
    Value::Int(n.into())
}

fn name_value(name: &Option<String>) -> Option<Value> {
    name.as_deref().and_then(clean_name).map(Value::Text)
}

impl PairMessage {
    fn type_number(&self) -> u64 {
        match self {
            PairMessage::Hello { .. } => 0,
            PairMessage::Nonce { .. } => 1,
            PairMessage::Reveal { .. } => 2,
            PairMessage::Result { .. } => 3,
            PairMessage::Confirm { .. } => 4,
        }
    }

    /// The body map, keys ascending (§4.1). Names are cleaned first.
    fn body(&self) -> Value {
        let mut pairs: Vec<(Value, Value)> = Vec::new();
        match self {
            PairMessage::Hello { mode, name, proof } => {
                pairs.push((int(0), int(mode.number())));
                if let Some(n) = name_value(name) {
                    pairs.push((int(1), n));
                }
                pairs.push((int(2), Value::Bytes(proof.to_vec())));
            }
            PairMessage::Nonce { nonce, name } => {
                pairs.push((int(0), Value::Bytes(nonce.to_vec())));
                if let Some(n) = name_value(name) {
                    pairs.push((int(1), n));
                }
            }
            PairMessage::Reveal { nonce } => {
                pairs.push((int(0), Value::Bytes(nonce.to_vec())));
            }
            PairMessage::Result { mac, name } => {
                pairs.push((int(0), Value::Bytes(mac.to_vec())));
                if let Some(n) = name_value(name) {
                    pairs.push((int(1), n));
                }
            }
            PairMessage::Confirm { accepted } => {
                pairs.push((int(0), Value::Bool(*accepted)));
            }
        }
        Value::Map(pairs)
    }

    /// The whole frame: length, then the CBOR payload `[type, body]`.
    pub fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut payload = Zeroizing::new(vec![0x82]); // array of 2
        cbor::encode(&int(self.type_number()), &mut payload);
        let mut body = self.body();
        cbor::encode(&body, &mut payload);
        wipe(&mut body);
        // Fixed fields and a name of at most 64 bytes: always far below
        // the limit.
        debug_assert!(payload.len() <= MAX_PAIR_FRAME);
        let mut out = Zeroizing::new(Vec::with_capacity(payload.len() + 2));
        varint::encode(payload.len() as u64, &mut out);
        out.extend_from_slice(&payload);
        out
    }

    /// Reads a frame payload (without its length) as a pairing message.
    pub fn decode_payload(payload: &[u8]) -> Result<Self, FrameError> {
        if payload.len() > MAX_PAIR_FRAME {
            return Err(FrameError::FrameTooLarge {
                len: payload.len() as u64,
                max: MAX_PAIR_FRAME,
            });
        }
        let mut raw = decode_raw_payload(payload)?;
        let result = Self::from_body(raw.ty, &raw.body);
        wipe(&mut raw.body);
        result
    }

    fn from_body(ty: u64, body: &Value) -> Result<Self, FrameError> {
        let Value::Map(pairs) = body else {
            return Err(FrameError::Malformed("message body is not a map"));
        };
        // Unknown keys are ignored (§4.3); known ones must have their type.
        let get = |key: i128| {
            pairs
                .iter()
                .find(|(k, _)| *k == Value::Int(key))
                .map(|(_, v)| v)
        };
        let need = |key: i128| get(key).ok_or(FrameError::Malformed("pairing field missing"));
        Ok(match ty {
            0 => PairMessage::Hello {
                mode: match need(0)? {
                    Value::Int(0) => Mode::Qr,
                    Value::Int(1) => Mode::Sas,
                    _ => return Err(FrameError::Malformed("unknown pairing mode")),
                },
                name: name_field(get(1))?,
                proof: bytes32(need(2)?)?,
            },
            1 => PairMessage::Nonce {
                nonce: bytes32(need(0)?)?,
                name: name_field(get(1))?,
            },
            2 => PairMessage::Reveal {
                nonce: bytes32(need(0)?)?,
            },
            3 => PairMessage::Result {
                mac: bytes32(need(0)?)?,
                name: name_field(get(1))?,
            },
            4 => PairMessage::Confirm {
                accepted: match need(0)? {
                    Value::Bool(b) => *b,
                    _ => return Err(FrameError::Malformed("confirm is not a bool")),
                },
            },
            _ => return Err(FrameError::Malformed("unknown pairing message")),
        })
    }
}

/// Wipes byte strings in a decoded or encoded body.
fn wipe(value: &mut Value) {
    match value {
        Value::Bytes(b) => b.zeroize(),
        Value::Map(pairs) => pairs.iter_mut().for_each(|(_, v)| wipe(v)),
        Value::Array(items) => items.iter_mut().for_each(wipe),
        _ => {}
    }
}

fn bytes32(v: &Value) -> Result<[u8; 32], FrameError> {
    match v {
        Value::Bytes(b) if b.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(b);
            Ok(out)
        }
        _ => Err(FrameError::Malformed("pairing field is not 32 bytes")),
    }
}

fn name_field(v: Option<&Value>) -> Result<Option<String>, FrameError> {
    match v {
        None => Ok(None),
        Some(Value::Text(t)) if t.len() <= MAX_NAME => Ok(clean_name(t)),
        Some(Value::Text(_)) => Err(FrameError::Malformed("device name over 64 bytes")),
        Some(_) => Err(FrameError::Malformed("device name is not text")),
    }
}

/// Reads one frame from the front of `buf`: the message and the bytes it
/// took, `Ok(None)` when `buf` holds less than a whole frame. The length is
/// checked before the payload is looked at.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(PairMessage, usize)>, FrameError> {
    let Some((len, header)) = varint::decode(buf)? else {
        return Ok(None);
    };
    let len = check_len(len)?;
    let Some(payload) = buf.get(header..header + len) else {
        return Ok(None);
    };
    Ok(Some((PairMessage::decode_payload(payload)?, header + len)))
}

fn check_len(len: u64) -> Result<usize, FrameError> {
    if len == 0 {
        return Err(FrameError::Malformed("empty frame"));
    }
    if len > MAX_PAIR_FRAME as u64 {
        return Err(FrameError::FrameTooLarge {
            len,
            max: MAX_PAIR_FRAME,
        });
    }
    Ok(len as usize)
}

/// Why a frame could not be read from a stream.
#[derive(Debug)]
pub enum ReadFail {
    /// The bytes break §12.3 (size, CBOR, fields).
    Frame(FrameError),
    /// The stream ended before a whole frame.
    Finished,
    /// The stream or connection failed (reset, closed, lost).
    Read(quinn::ReadExactError),
}

/// Reads the next pairing message from `recv`, never buffering more than
/// [`MAX_PAIR_FRAME`] bytes.
pub async fn read_message(recv: &mut quinn::RecvStream) -> Result<PairMessage, ReadFail> {
    let mut header = [0u8; 8];
    read_into(recv, &mut header[..1]).await?;
    let header_len = 1usize << (header[0] >> 6);
    read_into(recv, &mut header[1..header_len]).await?;
    let (len, _) = varint::decode(&header[..header_len])
        .map_err(ReadFail::Frame)?
        .ok_or(ReadFail::Frame(FrameError::Malformed("short length")))?;
    let len = check_len(len).map_err(ReadFail::Frame)?;
    let mut payload = Zeroizing::new(vec![0u8; len]);
    read_into(recv, &mut payload).await?;
    PairMessage::decode_payload(&payload).map_err(ReadFail::Frame)
}

async fn read_into(recv: &mut quinn::RecvStream, buf: &mut [u8]) -> Result<(), ReadFail> {
    if buf.is_empty() {
        return Ok(());
    }
    match recv.read_exact(buf).await {
        Ok(()) => Ok(()),
        Err(quinn::ReadExactError::FinishedEarly(_)) => Err(ReadFail::Finished),
        Err(e) => Err(ReadFail::Read(e)),
    }
}

/// Writes one pairing message to `send`.
pub async fn write_message(
    send: &mut quinn::SendStream,
    msg: &PairMessage,
) -> Result<(), quinn::WriteError> {
    send.write_all(&msg.encode()).await
}

/// A device name as it may be shown: control characters (Cc) and the
/// characters that change text direction (U+061C, U+200E, U+200F,
/// U+202A–U+202E, U+2066–U+2069) removed, white space trimmed, cut to
/// [`MAX_NAME`] bytes on a character boundary. `None` when nothing is left.
pub fn clean_name(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .filter(|&c| {
            !(c.is_control()
                || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}')
                || ('\u{202A}'..='\u{202E}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c))
        })
        .collect();
    let mut s = kept.trim();
    if s.len() > MAX_NAME {
        let mut end = MAX_NAME;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s = s[..end].trim_end();
    }
    (!s.is_empty()).then(|| s.to_string())
}

/// What a pairing link (QR) carries (§12.4): the server's key, the one-time
/// secret, where to reach the server and its name. The secret is wiped on
/// drop and never printed.
#[derive(Clone)]
pub struct PairingOffer {
    pub key: PublicKey,
    secret: Zeroizing<[u8; SECRET_LEN]>,
    pub addrs: Vec<SocketAddr>,
    pub name: Option<String>,
}

impl fmt::Debug for PairingOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingOffer")
            .field("key", &self.key)
            .field("addrs", &self.addrs)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Why a pairing link was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError(pub &'static str);

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pairing link: {}", self.0)
    }
}

impl std::error::Error for LinkError {}

impl PairingOffer {
    /// An offer (the listener makes these; tests may make their own).
    pub fn new(
        key: PublicKey,
        secret: [u8; SECRET_LEN],
        addrs: Vec<SocketAddr>,
        name: Option<String>,
    ) -> Self {
        PairingOffer {
            key,
            secret: Zeroizing::new(secret),
            addrs,
            name: name.as_deref().and_then(clean_name),
        }
    }

    /// The one-time secret.
    pub fn secret(&self) -> &[u8; SECRET_LEN] {
        &self.secret
    }

    /// The link: `windconnect:pair?v=1&k=…&s=…&a=…[&a=…][&n=…]`.
    pub fn to_link(&self) -> Zeroizing<String> {
        let mut out = String::from(LINK_PREFIX);
        out.push_str("v=1&k=");
        out.push_str(&base64url_encode(self.key.as_bytes()));
        out.push_str("&s=");
        let mut secret = base64url_encode(&self.secret[..]);
        out.push_str(&secret);
        secret.zeroize();
        for addr in self.addrs.iter().take(MAX_LINK_ADDRS) {
            out.push_str("&a=");
            out.push_str(&percent_encode(addr.to_string().as_bytes()));
        }
        if let Some(name) = self.name.as_deref().and_then(clean_name) {
            out.push_str("&n=");
            out.push_str(&percent_encode(name.as_bytes()));
        }
        Zeroizing::new(out)
    }

    /// Reads a pairing link. Unknown parameters are ignored; a known one
    /// given twice (other than `a`), a missing `k`, `s` or `a`, or a version
    /// other than 1 is an error.
    pub fn parse(link: &str) -> Result<Self, LinkError> {
        if link.len() > MAX_LINK {
            return Err(LinkError("too long"));
        }
        let query = link
            .strip_prefix(LINK_PREFIX)
            .ok_or(LinkError("not a windconnect:pair link"))?;
        let (mut v, mut k, mut s, mut n) = (None, None, None, None);
        let mut addrs = Vec::new();
        for param in query.split('&') {
            let (name, value) = param
                .split_once('=')
                .ok_or(LinkError("parameter without a value"))?;
            let value = percent_decode(value)?;
            let slot = match name {
                "v" => &mut v,
                "k" => &mut k,
                "s" => &mut s,
                "n" => &mut n,
                "a" => {
                    if addrs.len() == MAX_LINK_ADDRS {
                        return Err(LinkError("too many addresses"));
                    }
                    let text = std::str::from_utf8(&value).map_err(|_| LinkError("bad address"))?;
                    addrs.push(SocketAddr::from_str(text).map_err(|_| LinkError("bad address"))?);
                    continue;
                }
                _ => continue,
            };
            if slot.replace(value).is_some() {
                return Err(LinkError("parameter given twice"));
            }
        }
        if v.as_deref() != Some(b"1".as_slice()) {
            return Err(LinkError("unsupported version"));
        }
        let key_bytes = base64url_decode(k.as_deref().ok_or(LinkError("no key"))?)?;
        let key = <[u8; 32]>::try_from(key_bytes.as_slice())
            .ok()
            .and_then(|b| PublicKey::from_bytes(&b))
            .ok_or(LinkError("bad key"))?;
        let mut secret_bytes = base64url_decode(s.as_deref().ok_or(LinkError("no secret"))?)?;
        let secret = <[u8; SECRET_LEN]>::try_from(secret_bytes.as_slice())
            .map_err(|_| LinkError("bad secret"));
        secret_bytes.zeroize();
        if let Some(raw) = s.as_mut() {
            raw.zeroize();
        }
        let secret = secret?;
        if addrs.is_empty() {
            return Err(LinkError("no address"));
        }
        let name = match n {
            None => None,
            Some(raw) => {
                let text = String::from_utf8(raw).map_err(|_| LinkError("bad name"))?;
                if text.len() > MAX_NAME {
                    return Err(LinkError("name over 64 bytes"));
                }
                clean_name(&text)
            }
        };
        Ok(PairingOffer {
            key,
            secret: Zeroizing::new(secret),
            addrs,
            name,
        })
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (RFC 4648 §5).
pub fn base64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Strict base64url without padding: no `=`, only the alphabet, and unused
/// trailing bits zero, so each value has exactly one spelling.
pub fn base64url_decode(text: &[u8]) -> Result<Vec<u8>, LinkError> {
    if text.len() % 4 == 1 {
        return Err(LinkError("bad base64url length"));
    }
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let d = B64
                .iter()
                .position(|&b| b == c)
                .ok_or(LinkError("bad base64url character"))? as u32;
            n |= d << (18 - 6 * i);
        }
        let bytes = chunk.len() * 6 / 8;
        let unused_bits = 24 - 8 * bytes;
        if n & ((1 << unused_bits) - 1) != 0 {
            return Err(LinkError("non-canonical base64url"));
        }
        out.extend((0..bytes).map(|i| (n >> (16 - 8 * i)) as u8));
    }
    Ok(out)
}

/// The byte two ASCII hex digits spell; `None` for anything else (no sign,
/// no space: `u8::from_str_radix` would take `+f`).
pub(crate) fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    let digit = |c: u8| (c as char).to_digit(16).filter(|_| c.is_ascii_hexdigit());
    Some((digit(hi)? << 4 | digit(lo)?) as u8)
}

fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

/// Percent-encodes every byte but `A–Z a–z 0–9 - . _ ~`.
fn percent_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    for &b in data {
        if unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` escapes. Other bytes must be printable ASCII other than
/// `%`, `&`, `=`, `#` and space.
fn percent_decode(text: &str) -> Result<Vec<u8>, LinkError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = match bytes.get(i + 1..i + 3) {
                    Some(&[hi, lo]) => hex_byte(hi, lo),
                    _ => None,
                }
                .ok_or(LinkError("bad percent escape"))?;
                out.push(hex);
                i += 3;
            }
            b if b.is_ascii_graphic() && !matches!(b, b'&' | b'=' | b'#') => {
                out.push(b);
                i += 1;
            }
            _ => return Err(LinkError("bad character")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    #[test]
    fn messages_round_trip() {
        let msgs = [
            PairMessage::Hello {
                mode: Mode::Qr,
                name: Some("Pixel 9".into()),
                proof: [7; 32],
            },
            PairMessage::Hello {
                mode: Mode::Sas,
                name: None,
                proof: [1; 32],
            },
            PairMessage::Nonce {
                nonce: [2; 32],
                name: Some("SZYMON-PC".into()),
            },
            PairMessage::Reveal { nonce: [3; 32] },
            PairMessage::Result {
                mac: [4; 32],
                name: None,
            },
            PairMessage::Confirm { accepted: true },
            PairMessage::Confirm { accepted: false },
        ];
        for m in msgs {
            let frame = m.encode();
            let (back, used) = decode_frame(&frame).unwrap().unwrap();
            assert_eq!(back, m);
            assert_eq!(used, frame.len());
            assert_eq!(decode_frame(&frame[..frame.len() - 1]).unwrap(), None);
        }
    }

    /// The examples in PROTOCOL.md §12.3, byte for byte.
    #[test]
    fn spec_examples() {
        let hex =
            |m: &PairMessage| -> String { m.encode().iter().map(|b| format!("{b:02x}")).collect() };
        let examples = [
            hex(&PairMessage::Hello {
                mode: Mode::Qr,
                name: Some("Pixel 9".into()),
                proof: [0x11; 32],
            }),
            hex(&PairMessage::Nonce {
                nonce: [0x22; 32],
                name: Some("PC".into()),
            }),
            hex(&PairMessage::Reveal { nonce: [0x33; 32] }),
            hex(&PairMessage::Result {
                mac: [0x44; 32],
                name: None,
            }),
            hex(&PairMessage::Confirm { accepted: true }),
        ];
        assert_eq!(examples, SPEC_EXAMPLES.map(String::from));
    }

    const SPEC_EXAMPLES: [&str; 5] = [
        "318200a300000167506978656c20390258201111111111111111111111111111111111111111111111111111111111111111",
        "2a8201a2005820222222222222222222222222222222222222222222222222222222222222222201625043",
        "268202a10058203333333333333333333333333333333333333333333333333333333333333333",
        "268203a10058204444444444444444444444444444444444444444444444444444444444444444",
        "058204a100f5",
    ];

    #[test]
    fn frame_limits_are_checked_before_the_payload() {
        // Length 1025: refused from the length alone.
        let mut too_big = Vec::new();
        varint::encode(MAX_PAIR_FRAME as u64 + 1, &mut too_big);
        assert!(matches!(
            decode_frame(&too_big),
            Err(FrameError::FrameTooLarge { .. })
        ));
        assert!(decode_frame(&[0x00]).is_err(), "empty frame");
        // Non-minimal length encoding.
        assert!(decode_frame(&[0x40, 0x05, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn bad_fields_are_refused() {
        let frame = |ty: u64, body: Value| {
            let mut p = vec![0x82];
            cbor::encode(&int(ty), &mut p);
            cbor::encode(&body, &mut p);
            p
        };
        let cases = [
            frame(9, Value::Map(vec![])),
            frame(
                0,
                Value::Map(vec![(int(0), int(2)), (int(2), Value::Bytes(vec![0; 32]))]),
            ),
            frame(
                0,
                Value::Map(vec![(int(0), int(0)), (int(2), Value::Bytes(vec![0; 31]))]),
            ),
            frame(0, Value::Map(vec![(int(0), int(0))])),
            frame(
                0,
                Value::Map(vec![
                    (int(0), int(0)),
                    (int(1), Value::Text("x".repeat(65))),
                    (int(2), Value::Bytes(vec![0; 32])),
                ]),
            ),
            frame(4, Value::Map(vec![(int(0), int(1))])),
        ];
        for payload in cases {
            assert!(
                PairMessage::decode_payload(&payload).is_err(),
                "{payload:02x?}"
            );
        }
        // Unknown keys are ignored.
        let ok = frame(
            4,
            Value::Map(vec![(int(0), Value::Bool(true)), (int(9), Value::Null)]),
        );
        assert_eq!(
            PairMessage::decode_payload(&ok).unwrap(),
            PairMessage::Confirm { accepted: true }
        );
    }

    #[test]
    fn names_lose_control_and_direction_characters() {
        assert_eq!(clean_name("  Pixel 9 \n"), Some("Pixel 9".into()));
        assert_eq!(
            clean_name("Evil\u{202E}CP-NOMYZS\u{2066}x\u{200F}"),
            Some("EvilCP-NOMYZSx".into())
        );
        assert_eq!(clean_name("\u{0007}\u{202A}  "), None);
        let long = "ż".repeat(40); // 80 bytes // i18n: data
        let cut = clean_name(&long).unwrap();
        assert_eq!(cut.len(), 64);
        assert!(cut.chars().all(|c| c == 'ż')); // i18n: data
    }

    #[test]
    fn link_round_trips_and_hides_the_secret() {
        let key = DeviceIdentity::generate().unwrap().public_key();
        let offer = PairingOffer::new(
            key,
            [9; SECRET_LEN],
            vec![
                "192.168.1.20:47821".parse().unwrap(),
                "[fe80::1]:47821".parse().unwrap(),
            ],
            Some("SZYMON-PC & co".into()),
        );
        let link = offer.to_link();
        assert!(link.starts_with(LINK_PREFIX));
        let back = PairingOffer::parse(&link).unwrap();
        assert_eq!(back.key, key);
        assert_eq!(back.secret(), &[9; SECRET_LEN]);
        assert_eq!(back.addrs, offer.addrs);
        assert_eq!(back.name.as_deref(), Some("SZYMON-PC & co"));
        assert!(!format!("{offer:?}").contains("CQkJ"), "secret not printed");
    }

    #[test]
    fn bad_links_are_refused() {
        let key = DeviceIdentity::generate().unwrap().public_key();
        let good = PairingOffer::new(
            key,
            [1; SECRET_LEN],
            vec!["10.0.0.2:5000".parse().unwrap()],
            None,
        )
        .to_link()
        .to_string();
        assert!(PairingOffer::parse(&good).is_ok());
        let k = base64url_encode(key.as_bytes());
        let s = base64url_encode(&[1; SECRET_LEN]);
        let cases = [
            good.replace("v=1", "v=2"),
            good.replace("windconnect:pair?", "windconnect:other?"),
            format!("{good}&s={s}"),
            good.replace(&format!("&s={s}"), ""),
            good.replace("&a=10.0.0.2%3A5000", ""),
            good.replace(&s, &base64url_encode(&[1; 15])),
            good.replace(&k, &base64url_encode(&[7; 31])),
            format!("{good}&n={}", "x".repeat(65)),
            format!("{good}&n=%ZZ"),
            format!("{good}&a=nonsense"),
            format!("{good}&x"),
            format!("{good}&n={}", "a".repeat(MAX_LINK)),
        ];
        for link in cases {
            assert!(PairingOffer::parse(&link).is_err(), "{link}");
        }
        // Unknown parameters are fine.
        assert!(PairingOffer::parse(&format!("{good}&future=1")).is_ok());
    }

    #[test]
    fn base64url_is_strict() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", &[0xff; 32]] {
            let text = base64url_encode(data);
            assert_eq!(base64url_decode(text.as_bytes()).unwrap(), data);
        }
        assert!(base64url_decode(b"Zg==").is_err());
        assert!(base64url_decode(b"Zh").is_err(), "non-zero unused bits");
        assert!(base64url_decode(b"Z").is_err());
        assert!(base64url_decode(b"Zm9+").is_err(), "standard alphabet");
    }
}
