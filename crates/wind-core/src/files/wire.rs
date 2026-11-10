//! `files/1` messages (PROTOCOL.md §9.1) and their `[type, body]` form.
//!
//! Type numbers are per direction: the sender of the file writes
//! [`ToReceiver`], the receiver answers with [`ToSender`] on the same
//! streams.

use crate::protocol::cbor::Value;
use crate::protocol::{FrameError, RawMessage};

type Result<T> = std::result::Result<T, FrameError>;

/// A transfer's id, chosen by the sender at random.
pub type Id = [u8; 16];

/// Smallest and largest chunk; chunk sizes are multiples of the smallest.
pub const MIN_CHUNK: u32 = 64 * 1024;
pub const MAX_CHUNK: u32 = 512 * 1024;
/// Most chunks in one transfer: the `have` bitmap stays at 512 KiB.
pub const MAX_CHUNKS: u64 = 1 << 22;
/// Most streams of one transfer.
pub const MAX_STREAMS: u64 = 8;
/// Longest `path`, in UTF-8 bytes.
pub const MAX_PATH: usize = 1024;

const WRONG: FrameError = FrameError::Malformed("a files field is missing or has the wrong type");

/// `Offer` (type 0): what the first stream of a transfer starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub id: Id,
    /// Relative path, `/`-separated (§9.1 rules, checked by the receiver).
    pub path: String,
    pub size: u64,
    pub chunk: u32,
    pub streams: u64,
    /// Last change of the source, ms since the Unix epoch.
    pub modified: Option<u64>,
}

impl Offer {
    /// Chunks of the file.
    pub fn chunks(&self) -> u64 {
        self.size.div_ceil(u64::from(self.chunk))
    }

    /// Bytes of chunk `index`.
    pub fn chunk_len(&self, index: u64) -> usize {
        let start = index * u64::from(self.chunk);
        (self.size - start).min(u64::from(self.chunk)) as usize
    }
}

/// From the sender of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToReceiver {
    Offer(Offer),
    /// First message of each further stream (type 1).
    Join {
        id: Id,
        stream: u64,
    },
    /// Type 2.
    Chunk {
        index: u64,
        data: Vec<u8>,
        sha256: [u8; 32],
    },
}

/// From the receiver of the file, on the way back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToSender {
    /// Type 0: the chunks the receiver already holds, as a bitmap.
    Accept { have: Vec<u8> },
    /// Type 1: chunk verified and written.
    Ack { index: u64 },
    /// Type 2: chunk failed its checksum; send it again.
    Bad { index: u64 },
    /// Type 3: the file is complete under its final name.
    Complete,
}

fn uint(n: u64) -> Value {
    Value::Int(n.into())
}

fn map(fields: Vec<(u64, Value)>) -> Value {
    Value::Map(fields.into_iter().map(|(k, v)| (uint(k), v)).collect())
}

impl ToReceiver {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::Offer(o) => {
                let mut f = vec![
                    (0, Value::Bytes(o.id.to_vec())),
                    (1, Value::Text(o.path.clone())),
                    (2, uint(o.size)),
                    (3, uint(o.chunk.into())),
                    (4, uint(o.streams)),
                ];
                if let Some(m) = o.modified {
                    f.push((5, uint(m)));
                }
                RawMessage::new(0, map(f))
            }
            Self::Join { id, stream } => RawMessage::new(
                1,
                map(vec![(0, Value::Bytes(id.to_vec())), (1, uint(*stream))]),
            ),
            Self::Chunk {
                index,
                data,
                sha256,
            } => RawMessage::new(
                2,
                map(vec![
                    (0, uint(*index)),
                    (1, Value::Bytes(data.clone())),
                    (2, Value::Bytes(sha256.to_vec())),
                ]),
            ),
        }
    }

    /// `Ok(None)` for an unknown type, which is skipped (§4.4).
    pub fn decode(msg: RawMessage) -> Result<Option<Self>> {
        let mut b = Body::new(msg.body)?;
        Ok(Some(match msg.ty {
            0 => {
                let offer = Offer {
                    id: b.id(0)?,
                    path: b.text(1, 1, MAX_PATH)?,
                    size: b.u64(2)?,
                    chunk: u32::try_from(b.u64(3)?).map_err(|_| WRONG)?,
                    streams: b.u64(4)?,
                    modified: b.opt_u64(5)?,
                };
                if offer.chunk < MIN_CHUNK
                    || offer.chunk > MAX_CHUNK
                    || !offer.chunk.is_multiple_of(MIN_CHUNK)
                    || !(1..=MAX_STREAMS).contains(&offer.streams)
                    || offer.chunks() > MAX_CHUNKS
                {
                    return Err(FrameError::Malformed("Offer out of range"));
                }
                Self::Offer(offer)
            }
            1 => {
                let (id, stream) = (b.id(0)?, b.u64(1)?);
                if !(1..MAX_STREAMS).contains(&stream) {
                    return Err(FrameError::Malformed("Join stream out of range"));
                }
                Self::Join { id, stream }
            }
            2 => Self::Chunk {
                index: b.u64(0)?,
                data: b.bytes(1)?,
                sha256: b.bytes(2)?.try_into().map_err(|_| WRONG)?,
            },
            _ => return Ok(None),
        }))
    }
}

impl ToSender {
    pub fn encode(&self) -> RawMessage {
        match self {
            Self::Accept { have } => RawMessage::new(0, map(vec![(0, Value::Bytes(have.clone()))])),
            Self::Ack { index } => RawMessage::new(1, map(vec![(0, uint(*index))])),
            Self::Bad { index } => RawMessage::new(2, map(vec![(0, uint(*index))])),
            Self::Complete => RawMessage::new(3, Value::Map(Vec::new())),
        }
    }

    /// `Ok(None)` for an unknown type, which is skipped (§4.4).
    pub fn decode(msg: RawMessage) -> Result<Option<Self>> {
        let mut b = Body::new(msg.body)?;
        Ok(Some(match msg.ty {
            0 => Self::Accept { have: b.bytes(0)? },
            1 => Self::Ack { index: b.u64(0)? },
            2 => Self::Bad { index: b.u64(0)? },
            3 => Self::Complete,
            _ => return Ok(None),
        }))
    }
}

/// A body's fields by key; unknown keys are left alone (§4.3).
struct Body(Vec<(Value, Value)>);

impl Body {
    fn new(body: Value) -> Result<Self> {
        match body {
            Value::Map(pairs) => Ok(Body(pairs)),
            _ => Err(WRONG),
        }
    }

    fn take(&mut self, key: u64) -> Option<Value> {
        let i = self
            .0
            .iter()
            .position(|(k, _)| matches!(k, Value::Int(n) if *n == i128::from(key)))?;
        Some(self.0.swap_remove(i).1)
    }

    fn opt_u64(&mut self, key: u64) -> Result<Option<u64>> {
        match self.take(key) {
            None => Ok(None),
            Some(Value::Int(n)) => u64::try_from(n).map(Some).map_err(|_| WRONG),
            Some(_) => Err(WRONG),
        }
    }

    fn u64(&mut self, key: u64) -> Result<u64> {
        self.opt_u64(key)?.ok_or(WRONG)
    }

    fn bytes(&mut self, key: u64) -> Result<Vec<u8>> {
        match self.take(key) {
            Some(Value::Bytes(b)) => Ok(b),
            _ => Err(WRONG),
        }
    }

    fn id(&mut self, key: u64) -> Result<Id> {
        self.bytes(key)?.try_into().map_err(|_| WRONG)
    }

    fn text(&mut self, key: u64, min: usize, max: usize) -> Result<String> {
        match self.take(key) {
            Some(Value::Text(t)) if (min..=max).contains(&t.len()) => Ok(t),
            _ => Err(WRONG),
        }
    }
}

/// Whether bit `index` of a `have` bitmap is set (chunk `i` is byte `i / 8`,
/// bit `0x80 >> (i % 8)`).
pub fn has(bitmap: &[u8], index: u64) -> bool {
    bitmap
        .get((index / 8) as usize)
        .is_some_and(|b| b & (0x80 >> (index % 8)) != 0)
}

/// Sets bit `index`, growing the bitmap as needed.
pub fn set(bitmap: &mut Vec<u8>, index: u64) {
    let byte = (index / 8) as usize;
    if bitmap.len() <= byte {
        bitmap.resize(byte + 1, 0);
    }
    bitmap[byte] |= 0x80 >> (index % 8);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{decode_raw_payload, encode_raw_frame, Channel};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn round_trip_receiver(m: ToReceiver) -> String {
        let frame = encode_raw_frame(Channel::Files, &m.encode()).unwrap();
        // One-byte length for these small examples.
        let back = decode_raw_payload(&frame[1..]).unwrap();
        assert_eq!(ToReceiver::decode(back).unwrap(), Some(m));
        hex(&frame)
    }

    fn round_trip_sender(m: ToSender) -> String {
        let frame = encode_raw_frame(Channel::Files, &m.encode()).unwrap();
        let back = decode_raw_payload(&frame[1..]).unwrap();
        assert_eq!(ToSender::decode(back).unwrap(), Some(m));
        hex(&frame)
    }

    /// The binary examples of PROTOCOL.md §9.1.
    #[test]
    fn spec_examples() {
        let offer = ToReceiver::Offer(Offer {
            id: [0x11; 16],
            path: "a/b.txt".into(),
            size: 300_000,
            chunk: 262_144,
            streams: 4,
            modified: None,
        });
        assert_eq!(
            round_trip_receiver(offer),
            "2c8200a5005011111111111111111111111111111111016761\
             2f622e747874021a000493e0031a00040000\
             0404"
        );
        assert_eq!(
            round_trip_receiver(ToReceiver::Join {
                id: [0x11; 16],
                stream: 1
            }),
            "178201a2005011111111111111111111111111111111\
             0101"
        );
        assert_eq!(
            round_trip_receiver(ToReceiver::Chunk {
                index: 1,
                data: b"hi".to_vec(),
                sha256: [0; 32],
            }),
            "2c8202a300010142686902582000000000000000000000\
             00000000000000000000000000000000000000000000"
        );
        assert_eq!(
            round_trip_sender(ToSender::Accept { have: vec![0x80] }),
            "068200a1004180"
        );
        assert_eq!(
            round_trip_sender(ToSender::Ack { index: 1 }),
            "058201a10001"
        );
        assert_eq!(
            round_trip_sender(ToSender::Bad { index: 1 }),
            "058202a10001"
        );
        assert_eq!(round_trip_sender(ToSender::Complete), "038203a0");
    }

    #[test]
    fn out_of_range_offers_are_malformed() {
        let base = Offer {
            id: [1; 16],
            path: "x".into(),
            size: 10,
            chunk: MIN_CHUNK,
            streams: 1,
            modified: Some(5),
        };
        let bad = [
            Offer {
                chunk: MIN_CHUNK - 1,
                ..base.clone()
            },
            Offer {
                chunk: MAX_CHUNK + MIN_CHUNK,
                ..base.clone()
            },
            Offer {
                chunk: MIN_CHUNK + 1,
                ..base.clone()
            },
            Offer {
                streams: 0,
                ..base.clone()
            },
            Offer {
                streams: MAX_STREAMS + 1,
                ..base.clone()
            },
            Offer {
                size: MAX_CHUNKS * u64::from(MAX_CHUNK) + 1,
                chunk: MAX_CHUNK,
                ..base.clone()
            },
        ];
        for o in bad {
            assert!(ToReceiver::decode(ToReceiver::Offer(o).encode()).is_err());
        }
        assert!(ToReceiver::decode(ToReceiver::Offer(base).encode()).is_ok());
        let join = ToReceiver::Join {
            id: [1; 16],
            stream: 0,
        };
        assert!(ToReceiver::decode(join.encode()).is_err());
        let unknown = RawMessage::new(9, Value::Map(Vec::new()));
        assert_eq!(ToReceiver::decode(unknown.clone()).unwrap(), None);
        assert_eq!(ToSender::decode(unknown).unwrap(), None);
    }

    #[test]
    fn bitmap_bits_are_msb_first() {
        let mut b = Vec::new();
        set(&mut b, 0);
        set(&mut b, 9);
        assert_eq!(b, vec![0x80, 0x40]);
        assert!(has(&b, 0) && has(&b, 9) && !has(&b, 1) && !has(&b, 100));
    }
}
