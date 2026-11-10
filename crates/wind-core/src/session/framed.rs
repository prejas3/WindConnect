//! Frames over QUIC streams: reading with [`FrameReader`] and writing whole
//! frames.

use std::fmt;

use crate::protocol::{
    encode_raw_frame, Channel, ErrorCode, FrameError, FrameReader, Message, RawMessage,
};

use tokio::sync::watch;

use super::varint_code;

/// Reading a stream failed: its bytes broke the protocol, or the stream or
/// connection ended abnormally.
#[derive(Debug)]
pub enum RecvError {
    Frame(FrameError),
    Read(quinn::ReadError),
    /// This side withdrew the stream's channel (§6, rule 5) after the
    /// stream was taken; it has been stopped with `CANCELLED`.
    Withdrawn,
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvError::Frame(e) => write!(f, "{e:?}"),
            RecvError::Read(e) => write!(f, "{e}"),
            RecvError::Withdrawn => f.write_str("channel withdrawn by this side"),
        }
    }
}

impl std::error::Error for RecvError {}

impl From<FrameError> for RecvError {
    fn from(e: FrameError) -> Self {
        RecvError::Frame(e)
    }
}

impl From<quinn::ReadError> for RecvError {
    fn from(e: quinn::ReadError) -> Self {
        RecvError::Read(e)
    }
}

const READ_BUF: usize = 64 * 1024;

/// The receiving side of a stream, read as frames of one channel. The
/// stream header has already been read.
pub struct FramedRecv {
    recv: quinn::RecvStream,
    reader: FrameReader,
    buf: Box<[u8]>,
    start: usize,
    end: usize,
    /// For a stream of a switchable channel: its gate and the withdrawal
    /// count when it was taken (WN-86).
    gate: Option<(watch::Receiver<super::Gate>, u64)>,
}

impl fmt::Debug for FramedRecv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedRecv")
            .field("channel", &self.reader.channel())
            .field("stream", &self.recv.id())
            .finish_non_exhaustive()
    }
}

impl FramedRecv {
    pub fn new(recv: quinn::RecvStream, channel: Channel) -> Self {
        FramedRecv {
            recv,
            reader: FrameReader::new(channel),
            buf: vec![0; READ_BUF].into_boxed_slice(),
            start: 0,
            end: 0,
            gate: None,
        }
    }

    /// Ties the stream to its channel's gate: once the channel is withdrawn
    /// (its count moves past `at`), reading stops the stream with
    /// `CANCELLED` and fails with [`RecvError::Withdrawn`].
    pub(super) fn with_gate(mut self, gate: watch::Receiver<super::Gate>, at: u64) -> Self {
        self.gate = Some((gate, at));
        self
    }

    fn withdrawn(&mut self) -> RecvError {
        let _ = self.recv.stop(varint_code(ErrorCode::Cancelled));
        RecvError::Withdrawn
    }

    pub fn id(&self) -> quinn::StreamId {
        self.recv.id()
    }

    /// Payload length of the last frame read.
    pub fn last_frame_len(&self) -> usize {
        self.reader.last_frame_len()
    }

    /// The next typed message (`control`, `input`); `None` at FIN. An
    /// unknown type is a non-fatal `FrameError::UnknownMessage`.
    pub async fn next_message(&mut self) -> Result<Option<Message>, RecvError> {
        loop {
            if let Some(m) = self.reader.next_message()? {
                return Ok(Some(m));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// The next message as `[type, body]`; `None` at FIN.
    pub async fn next_raw(&mut self) -> Result<Option<RawMessage>, RecvError> {
        loop {
            if let Some(m) = self.reader.next_raw()? {
                return Ok(Some(m));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// Pushes buffered bytes into the reader, reading more when none are
    /// left. `false` at a clean FIN.
    async fn fill(&mut self) -> Result<bool, RecvError> {
        if let Some((gate, at)) = &self.gate {
            if gate.borrow().withdrawals > *at {
                return Err(self.withdrawn());
            }
        }
        if self.start == self.end {
            let read = match &mut self.gate {
                None => self.recv.read(&mut self.buf).await?,
                Some((gate, at)) => {
                    let at = *at;
                    // A gate whose session is gone never fires.
                    let withdrawn = async {
                        if gate.wait_for(|g| g.withdrawals > at).await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    };
                    let read = tokio::select! {
                        biased;
                        () = withdrawn => None,
                        r = self.recv.read(&mut self.buf) => Some(r),
                    };
                    match read {
                        None => return Err(self.withdrawn()),
                        Some(r) => r?,
                    }
                }
            };
            match read {
                None => {
                    self.reader.finish()?;
                    return Ok(false);
                }
                Some(n) => {
                    self.start = 0;
                    self.end = n;
                }
            }
        }
        let used = self.reader.push(&self.buf[self.start..self.end])?;
        self.start += used;
        Ok(true)
    }

    /// Asks the peer to stop sending (`STOP_SENDING`) with `code`.
    pub fn stop(&mut self, code: ErrorCode) {
        let _ = self.recv.stop(varint_code(code));
    }
}

/// Writes one raw message as a frame of `channel`.
pub async fn write_raw(
    send: &mut quinn::SendStream,
    channel: Channel,
    msg: &RawMessage,
) -> Result<(), super::SessionError> {
    let frame = encode_raw_frame(channel, msg).map_err(super::SessionError::Frame)?;
    send.write_all(&frame).await?;
    Ok(())
}
