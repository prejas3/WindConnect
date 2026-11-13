//! Frames (§4), their size limits (§4.2) and `input` datagrams (§8.1).

use super::message::{self, ControlMessage, InputMessage, Message};
use super::{cbor, varint, Channel, FrameError, PROTOCOL_MAJOR};

type Result<T> = std::result::Result<T, FrameError>;

/// Largest `Hello` / `HelloAck` frame length.
pub const MAX_HELLO_FRAME: usize = 4096;
/// Largest length of any other frame on `control`.
pub const MAX_CONTROL_FRAME: usize = 16 * 1024;
/// Largest frame length on every other stream.
pub const MAX_STREAM_FRAME: usize = 1024 * 1024;
/// Largest datagram, before the peer's `max_datagram_frame_size`.
pub const MAX_DATAGRAM: usize = 1200;

/// Largest frame length on `channel`. `first`: the stream's first frame,
/// which on `control` is `Hello` / `HelloAck`.
pub fn frame_limit(channel: Channel, first: bool) -> usize {
    match channel {
        Channel::Control if first => MAX_HELLO_FRAME,
        Channel::Control => MAX_CONTROL_FRAME,
        // One small request or reply per stream (§9.2).
        Channel::Actions => MAX_CONTROL_FRAME,
        // A few bytes per status (§9.4).
        Channel::Battery => MAX_CONTROL_FRAME,
        // Ring, stop and states (§9.5).
        Channel::Find => MAX_CONTROL_FRAME,
        // One small command per message (§9.6).
        Channel::MediaControl => MAX_CONTROL_FRAME,
        // Dismiss, or a reply of at most 1 KiB (§9.8).
        Channel::NotifyAction => MAX_CONTROL_FRAME,
        // One small state or level per message (§9.9, §9.10).
        Channel::Volume | Channel::Call | Channel::CallAction => MAX_CONTROL_FRAME,
        // Batches of threads and messages (§9.11) take the stream limit.
        _ => MAX_STREAM_FRAME,
    }
}

fn message_limit(msg: &Message) -> usize {
    match msg {
        Message::Control(ControlMessage::Hello(_) | ControlMessage::HelloAck(_)) => MAX_HELLO_FRAME,
        Message::Control(_) => MAX_CONTROL_FRAME,
        Message::Input(_) => MAX_STREAM_FRAME,
    }
}

/// The CBOR payload `[type, body]` of `msg`, without a length.
pub fn encode_payload(msg: &Message) -> Result<Vec<u8>> {
    Ok(cbor::to_vec(&message::encode(msg)?))
}

/// A whole frame: length, then payload. A message whose fields break
/// their limits is `Malformed`; one over its frame limit `FrameTooLarge`.
pub fn encode_frame(msg: &Message) -> Result<Vec<u8>> {
    let payload = encode_payload(msg)?;
    let max = message_limit(msg);
    if payload.len() > max {
        return Err(FrameError::FrameTooLarge {
            len: payload.len() as u64,
            max,
        });
    }
    let mut out = Vec::with_capacity(payload.len() + 4);
    varint::encode(payload.len() as u64, &mut out);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// A message as `[type, body]`, not decoded further: what channels whose
/// message types belong to feature modules carry (§4: the router picks the
/// module by `(channel, type)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    pub ty: u64,
    /// A map with natural-number keys; empty for a message without fields.
    pub body: cbor::Value,
}

impl RawMessage {
    pub fn new(ty: u64, body: cbor::Value) -> Self {
        Self { ty, body }
    }
}

/// Reads one frame's payload as `[type, body]`: canonical CBOR (§4.1), a
/// natural-number type and a map body with natural-number keys.
pub fn decode_raw_payload(payload: &[u8]) -> Result<RawMessage> {
    let value = cbor::decode(payload).map_err(FrameError::Malformed)?;
    let cbor::Value::Array(items) = value else {
        return Err(FrameError::Malformed("payload is not a [type, body] array"));
    };
    let [ty, body]: [cbor::Value; 2] = items
        .try_into()
        .map_err(|_| FrameError::Malformed("payload is not a [type, body] array"))?;
    let ty = match ty {
        cbor::Value::Int(n) if (0..=u64::MAX as i128).contains(&n) => n as u64,
        _ => {
            return Err(FrameError::Malformed(
                "message type is not a natural number",
            ))
        }
    };
    check_body(&body)?;
    Ok(RawMessage { ty, body })
}

fn check_body(body: &cbor::Value) -> Result<()> {
    let cbor::Value::Map(pairs) = body else {
        return Err(FrameError::Malformed("message body is not a map"));
    };
    if pairs
        .iter()
        .any(|(k, _)| !matches!(k, cbor::Value::Int(n) if (0..=i128::from(u64::MAX)).contains(n)))
    {
        return Err(FrameError::Malformed("body key is not a natural number"));
    }
    Ok(())
}

/// A whole frame of a raw message on `channel` (not `control`, whose
/// messages are typed). The whole message is checked first (§4.1: integer
/// range, nesting, unique map keys; natural-number body keys), so what is
/// sent is exactly what the peer reads, and a bad value is an error, never
/// a changed value or a panic.
pub fn encode_raw_frame(channel: Channel, msg: &RawMessage) -> Result<Vec<u8>> {
    if channel == Channel::Control {
        return Err(FrameError::Malformed("control messages are typed"));
    }
    check_body(&msg.body)?;
    // The body sits inside the [type, body] array: one level. Checked on
    // the borrowed value, before anything recursive (clone, encode) touches
    // it, so arbitrarily deep input is refused without deep recursion.
    cbor::check_inside(&msg.body, 1).map_err(FrameError::Malformed)?;
    // [type, body] written directly, without cloning the body.
    let mut payload = vec![0x82]; // array of 2
    cbor::encode(&cbor::Value::Int(msg.ty.into()), &mut payload);
    cbor::encode(&msg.body, &mut payload);
    let max = frame_limit(channel, false);
    if payload.len() > max {
        return Err(FrameError::FrameTooLarge {
            len: payload.len() as u64,
            max,
        });
    }
    let mut out = Vec::with_capacity(payload.len() + 4);
    varint::encode(payload.len() as u64, &mut out);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Reads one frame's payload as a message of `channel`.
pub fn decode_payload(channel: Channel, payload: &[u8]) -> Result<Message> {
    let value = cbor::decode(payload).map_err(FrameError::Malformed)?;
    message::decode(channel, value)
}

/// An `input` datagram: `01 04` and the payload, no length (§8.1). Only
/// `PointerMotion` and `Scroll` travel in datagrams.
pub fn encode_datagram(msg: &InputMessage) -> Result<Vec<u8>> {
    if !matches!(
        msg,
        InputMessage::PointerMotion(_) | InputMessage::Scroll(_)
    ) {
        return Err(FrameError::Malformed(
            "only PointerMotion and Scroll travel in datagrams",
        ));
    }
    let mut out = vec![PROTOCOL_MAJOR, Channel::Input.byte()];
    out.extend_from_slice(&encode_payload(&Message::Input(msg.clone()))?);
    if out.len() > MAX_DATAGRAM {
        return Err(FrameError::FrameTooLarge {
            len: out.len() as u64,
            max: MAX_DATAGRAM,
        });
    }
    Ok(out)
}

/// Reads an `input` datagram. `max_len`: the `max_datagram_frame_size` this
/// side announced (the limit is the smaller of it and 1200 B). The caller
/// drops a datagram that fails, without closing anything (§8.1).
pub fn decode_datagram(data: &[u8], max_len: usize) -> Result<InputMessage> {
    let max = max_len.min(MAX_DATAGRAM);
    if data.len() > max {
        return Err(FrameError::FrameTooLarge {
            len: data.len() as u64,
            max,
        });
    }
    let [version, channel, payload @ ..] = data else {
        return Err(FrameError::Malformed("datagram shorter than its header"));
    };
    if *version != PROTOCOL_MAJOR {
        return Err(FrameError::UnsupportedVersion(*version));
    }
    if *channel != Channel::Input.byte() {
        return Err(FrameError::UnknownChannel(*channel));
    }
    match decode_payload(Channel::Input, payload)? {
        Message::Input(m @ (InputMessage::PointerMotion(_) | InputMessage::Scroll(_))) => Ok(m),
        _ => Err(FrameError::Malformed(
            "only PointerMotion and Scroll travel in datagrams",
        )),
    }
}

/// Reads the frames of one stream (after its header) as they arrive, never
/// holding more than one frame.
///
/// [`push`](Self::push) takes bytes only up to the end of the current
/// frame and says how many it took; [`next_message`](Self::next_message)
/// then hands the frame over and frees room for the next one.
/// [`read`](Self::read) does both. Feed a received chunk like this:
///
/// ```
/// # use wind_core::protocol::{Channel, FrameError, FrameReader, Message};
/// fn on_chunk(reader: &mut FrameReader, chunk: &[u8]) -> Result<(), FrameError> {
///     let mut rest = chunk;
///     loop {
///         match reader.read(&mut rest) {
///             Ok(Some(msg)) => route(msg),
///             Ok(None) => return Ok(()), // all of `chunk` taken
///             Err(e) if !e.is_fatal() => {} // unknown type: skipped (§4.4)
///             Err(e) => return Err(e), // close with e.code()
///         }
///     }
/// }
/// # fn route(_: Message) {}
/// # let mut r = FrameReader::new(Channel::Control);
/// # on_chunk(&mut r, &[0x06, 0x82, 0x02, 0xa1, 0x00, 0x18, 0x2a]).unwrap();
/// ```
///
/// Each frame's length is checked against its limit (§4.2) as soon as its
/// length bytes are in, before any of its payload is kept, so memory stays
/// within one frame's limit however the bytes are chunked.
/// [`FrameError::UnknownMessage`] consumes its frame and is not fatal; any
/// other error ends the stream, and every later call returns it again. At
/// the stream's FIN, call [`finish`](Self::finish).
#[derive(Debug)]
pub struct FrameReader {
    channel: Channel,
    frames: u64,
    /// Length bytes of the current frame received so far.
    len_bytes: [u8; 8],
    len_count: usize,
    /// The current frame's checked length, once all its length bytes are in.
    payload_len: Option<usize>,
    payload: Vec<u8>,
    /// Payload length of the last frame read.
    last_len: usize,
    failed: Option<FrameError>,
}

impl FrameReader {
    pub fn new(channel: Channel) -> Self {
        Self {
            channel,
            frames: 0,
            len_bytes: [0; 8],
            len_count: 0,
            payload_len: None,
            payload: Vec::new(),
            last_len: 0,
            failed: None,
        }
    }

    /// Payload length of the last frame [`next_message`](Self::next_message)
    /// or [`next_raw`](Self::next_raw) returned.
    pub fn last_frame_len(&self) -> usize {
        self.last_len
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    /// Bytes of the current frame received so far.
    pub fn buffered(&self) -> usize {
        self.len_count + self.payload.len()
    }

    fn fail(&mut self, e: FrameError) -> FrameError {
        self.payload = Vec::new();
        self.payload_len = None;
        self.len_count = 0;
        self.failed = Some(e.clone());
        e
    }

    fn frame_complete(&self) -> bool {
        self.payload_len == Some(self.payload.len())
    }

    /// Takes bytes from the start of `data` up to the end of the current
    /// frame, and returns how many it took: fewer than `data.len()` once a
    /// whole frame is in, which [`next_message`](Self::next_message) must
    /// collect before the rest is pushed (until then, `push` takes 0).
    /// Fails as soon as the frame's length is invalid or over its limit.
    pub fn push(&mut self, data: &[u8]) -> Result<usize> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let mut used = 0;
        while used < data.len() && !self.frame_complete() {
            match self.payload_len {
                None => {
                    self.len_bytes[self.len_count] = data[used];
                    self.len_count += 1;
                    used += 1;
                    let decoded = varint::decode(&self.len_bytes[..self.len_count]);
                    if let Some(len) = self.check_len(decoded)? {
                        self.payload_len = Some(len);
                    }
                }
                Some(len) => {
                    let take = (len - self.payload.len()).min(data.len() - used);
                    let needed = self.payload.len() + take;
                    if needed > self.payload.capacity() {
                        // Grow geometrically, but never past the frame's length.
                        let target = needed.max(self.payload.capacity() * 2).min(len);
                        self.payload.reserve_exact(target - self.payload.len());
                    }
                    self.payload.extend_from_slice(&data[used..used + take]);
                    used += take;
                }
            }
        }
        Ok(used)
    }

    /// [`push`](Self::push)es from `data`, advancing it past what was
    /// taken, then returns [`next_message`](Self::next_message). `Ok(None)`
    /// means all of `data` was taken and no frame is complete yet.
    pub fn read(&mut self, data: &mut &[u8]) -> Result<Option<Message>> {
        let used = self.push(data)?;
        *data = &data[used..];
        self.next_message()
    }

    /// The checked length once all its bytes are in; `None` before.
    fn check_len(&mut self, decoded: Result<Option<(u64, usize)>>) -> Result<Option<usize>> {
        let max = frame_limit(self.channel, self.frames == 0);
        let len = match decoded {
            Ok(Some((len, _))) => len,
            Ok(None) => return Ok(None),
            Err(e) => return Err(self.fail(e)),
        };
        if len == 0 {
            return Err(self.fail(FrameError::Malformed("zero-length frame")));
        }
        if len > max as u64 {
            return Err(self.fail(FrameError::FrameTooLarge { len, max }));
        }
        Ok(Some(len as usize))
    }

    /// The current frame's message once all of it has been pushed, else
    /// `Ok(None)`.
    pub fn next_message(&mut self) -> Result<Option<Message>> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        if !self.frame_complete() {
            return Ok(None);
        }
        let payload = std::mem::take(&mut self.payload);
        self.payload_len = None;
        self.len_count = 0;
        self.frames += 1;
        self.last_len = payload.len();
        match decode_payload(self.channel, &payload) {
            Ok(msg) if payload.len() > message_limit(&msg) => {
                Err(self.fail(FrameError::FrameTooLarge {
                    len: payload.len() as u64,
                    max: message_limit(&msg),
                }))
            }
            Ok(msg) => Ok(Some(msg)),
            Err(e) if !e.is_fatal() => Err(e),
            Err(e) => Err(self.fail(e)),
        }
    }

    /// Like [`next_message`](Self::next_message), but the frame as a
    /// [`RawMessage`], for channels whose types belong to feature modules.
    pub fn next_raw(&mut self) -> Result<Option<RawMessage>> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        if !self.frame_complete() {
            return Ok(None);
        }
        let payload = std::mem::take(&mut self.payload);
        self.payload_len = None;
        self.len_count = 0;
        self.frames += 1;
        self.last_len = payload.len();
        decode_raw_payload(&payload)
            .map(Some)
            .map_err(|e| self.fail(e))
    }

    /// At the stream's FIN: a partly received frame is `MALFORMED_FRAME`.
    pub fn finish(&self) -> Result<()> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        if self.buffered() > 0 {
            return Err(FrameError::Malformed("stream ended inside a frame"));
        }
        Ok(())
    }
}
