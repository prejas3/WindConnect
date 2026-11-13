//! Frame codec for WindConnect protocol v1 (PROTOCOL.md).
//!
//! This module turns bytes into messages and back: stream headers (§3),
//! frames and their size limits (§4), the CBOR subset (§4.1), the
//! `control` (§7) and `input` (§8) messages, and `input` datagrams (§8.1).
//! Connection behaviour (handshake order, sessions, epochs) is not here.

pub mod cbor;
mod frame;
mod message;
pub mod varint;

use std::fmt;

pub use frame::{
    decode_datagram, decode_payload, decode_raw_payload, encode_datagram, encode_frame,
    encode_payload, encode_raw_frame, frame_limit, FrameReader, RawMessage, MAX_CONTROL_FRAME,
    MAX_DATAGRAM, MAX_HELLO_FRAME, MAX_STREAM_FRAME,
};
pub use message::{
    Button, Capability, ControlMessage, DeviceKind, ErrorMessage, Goodbye, Hello, InputMessage,
    InputSession, Message, Motion, PointerButton, Version, MAX_CAPS, MAX_MESSAGE_TEXT,
};

/// Major protocol version: the `version` byte of stream headers and
/// datagrams, and the number in the ALPN.
pub const PROTOCOL_MAJOR: u8 = 1;
/// Minor protocol version this implementation speaks.
pub const PROTOCOL_MINOR: u64 = 0;
/// ALPN of protocol v1.
pub const ALPN: &[u8] = b"windconnect/1";
/// ALPN of the pairing protocol (§12), on its own endpoint only.
pub const PAIR_ALPN: &[u8] = b"windconnect-pair/1";
/// Default UDP port of a server.
pub const DEFAULT_PORT: u16 = 47820;

/// A channel, chosen by the stream header (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Channel {
    Control = 0,
    Clipboard = 1,
    Notify = 2,
    Media = 3,
    Input = 4,
    Files = 5,
    /// Remote actions (§9.2, WN-54).
    Actions = 6,
    /// The phone's battery level and charging (§9.4, WN-49).
    Battery = 7,
    /// Find my phone: ring and stop (§9.5, WN-50).
    Find = 8,
    /// Commands for this side's media players (§9.6, WN-46).
    MediaControl = 9,
    /// Dismiss or reply to the other side's notifications (§9.8, WN-44).
    NotifyAction = 10,
    /// Each side's output volume, shown and set by the other (§9.9, WN-48).
    Volume = 11,
    /// The phone's call state, for pausing the PC's players (§9.10, WN-48).
    Call = 12,
    /// The phone's SMS threads and messages, read from the PC (§9.11, WN-55).
    Sms = 13,
    /// The PC rejects or silences the phone's ringing call (§9.12, WN-57).
    CallAction = 14,
    /// The phone's newest photos and screenshots in the PC's panel (§9.13, WN-58).
    Gallery = 15,
}

impl Channel {
    /// Its number on the wire.
    pub fn byte(self) -> u8 {
        self as u8
    }

    /// The channel numbered `b`; reserved and disallowed numbers are
    /// `UNKNOWN_CHANNEL`.
    pub fn from_byte(b: u8) -> Result<Self, FrameError> {
        Ok(match b {
            0 => Self::Control,
            1 => Self::Clipboard,
            2 => Self::Notify,
            3 => Self::Media,
            4 => Self::Input,
            5 => Self::Files,
            6 => Self::Actions,
            7 => Self::Battery,
            8 => Self::Find,
            9 => Self::MediaControl,
            10 => Self::NotifyAction,
            11 => Self::Volume,
            12 => Self::Call,
            13 => Self::Sms,
            14 => Self::CallAction,
            15 => Self::Gallery,
            _ => return Err(FrameError::UnknownChannel(b)),
        })
    }
}

/// The two-byte header the opener writes at the start of a stream (§3).
pub fn stream_header(channel: Channel) -> [u8; 2] {
    [PROTOCOL_MAJOR, channel.byte()]
}

/// Reads a stream header: an unknown `version` is `UNSUPPORTED_VERSION`
/// (checked first), an unknown channel `UNKNOWN_CHANNEL`.
pub fn decode_stream_header(header: [u8; 2]) -> Result<Channel, FrameError> {
    if header[0] != PROTOCOL_MAJOR {
        return Err(FrameError::UnsupportedVersion(header[0]));
    }
    Channel::from_byte(header[1])
}

/// Error codes of §10, used in `Error` / `Goodbye` and as QUIC
/// application error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    NoError,
    ProtocolViolation,
    UnsupportedVersion,
    FrameTooLarge,
    MalformedFrame,
    UnknownChannel,
    UnknownMessage,
    CapabilityMissing,
    UntrustedPeer,
    HelloTimeout,
    InternalError,
    RateLimited,
    Cancelled,
    GoingAway,
    /// No room for an incoming file (§9.1, WC-38).
    NoSpace,
    /// The sender removed the pairing (`Goodbye`, WC-28): the receiver
    /// drops the sender's key, the one from the TLS handshake.
    Unpaired,
    /// The sender has a session with another device and keeps it (WC-28).
    Busy,
}

impl ErrorCode {
    /// Its number on the wire.
    pub fn code(self) -> u64 {
        match self {
            Self::NoError => 0x00,
            Self::ProtocolViolation => 0x01,
            Self::UnsupportedVersion => 0x02,
            Self::FrameTooLarge => 0x03,
            Self::MalformedFrame => 0x04,
            Self::UnknownChannel => 0x05,
            Self::UnknownMessage => 0x06,
            Self::CapabilityMissing => 0x07,
            Self::UntrustedPeer => 0x08,
            Self::HelloTimeout => 0x09,
            Self::InternalError => 0x0a,
            Self::RateLimited => 0x0b,
            Self::Cancelled => 0x0c,
            Self::GoingAway => 0x0d,
            Self::NoSpace => 0x0e,
            Self::Unpaired => 0x0f,
            Self::Busy => 0x10,
        }
    }

    /// The code numbered `code`; an unknown one reads as `INTERNAL_ERROR`.
    pub fn from_code(code: u64) -> Self {
        match code {
            0x00 => Self::NoError,
            0x01 => Self::ProtocolViolation,
            0x02 => Self::UnsupportedVersion,
            0x03 => Self::FrameTooLarge,
            0x04 => Self::MalformedFrame,
            0x05 => Self::UnknownChannel,
            0x06 => Self::UnknownMessage,
            0x07 => Self::CapabilityMissing,
            0x08 => Self::UntrustedPeer,
            0x09 => Self::HelloTimeout,
            0x0b => Self::RateLimited,
            0x0c => Self::Cancelled,
            0x0d => Self::GoingAway,
            0x0e => Self::NoSpace,
            0x0f => Self::Unpaired,
            0x10 => Self::Busy,
            _ => Self::InternalError,
        }
    }
}

/// Why bytes could not be read as (or a message written as) protocol v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// A stream header or datagram with another major version.
    UnsupportedVersion(u8),
    /// A stream header or datagram with an unknown or disallowed channel.
    UnknownChannel(u8),
    /// A frame or datagram longer than its limit (§4.2).
    FrameTooLarge { len: u64, max: usize },
    /// Not valid v1: bad CBOR, a missing or mistyped field, a bad length.
    Malformed(&'static str),
    /// A well-formed frame of a type this side does not know (§4.4). Not
    /// fatal: the frame is consumed and the stream goes on.
    UnknownMessage { channel: Channel, ty: u64 },
}

impl FrameError {
    /// The §10 code to close the stream or connection with (or, for
    /// [`FrameError::UnknownMessage`], to put in an `Error`).
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::UnsupportedVersion(_) => ErrorCode::UnsupportedVersion,
            Self::UnknownChannel(_) => ErrorCode::UnknownChannel,
            Self::FrameTooLarge { .. } => ErrorCode::FrameTooLarge,
            Self::Malformed(_) => ErrorCode::MalformedFrame,
            Self::UnknownMessage { .. } => ErrorCode::UnknownMessage,
        }
    }

    /// Whether the stream can't go on after it.
    pub fn is_fatal(&self) -> bool {
        !matches!(self, Self::UnknownMessage { .. })
    }
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "unsupported protocol version {v}"),
            Self::UnknownChannel(c) => write!(f, "unknown channel {c:#04x}"),
            Self::FrameTooLarge { len, max } => {
                write!(f, "frame of {len} B exceeds the {max} B limit")
            }
            Self::Malformed(why) => write!(f, "malformed frame: {why}"),
            Self::UnknownMessage { channel, ty } => {
                write!(f, "unknown message type {ty} on channel {channel:?}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_headers() {
        assert_eq!(stream_header(Channel::Control), [0x01, 0x00]);
        assert_eq!(stream_header(Channel::Input), [0x01, 0x04]);
        for b in 0..=6 {
            let c = Channel::from_byte(b).unwrap();
            assert_eq!(decode_stream_header(stream_header(c)), Ok(c));
        }
    }

    #[test]
    fn rejects_unknown_versions_before_channels() {
        assert_eq!(
            decode_stream_header([0x02, 0x00]),
            Err(FrameError::UnsupportedVersion(2))
        );
        assert_eq!(
            decode_stream_header([0x00, 0xff]),
            Err(FrameError::UnsupportedVersion(0))
        );
        for b in [0x10, 0x3f, 0x40, 0xff] {
            assert_eq!(
                decode_stream_header([0x01, b]),
                Err(FrameError::UnknownChannel(b))
            );
        }
    }

    #[test]
    fn error_codes_round_trip_and_unknown_is_internal() {
        for c in 0..=0x10 {
            assert_eq!(ErrorCode::from_code(c).code(), c);
        }
        assert_eq!(ErrorCode::from_code(0x11), ErrorCode::InternalError);
        assert_eq!(ErrorCode::from_code(u64::MAX), ErrorCode::InternalError);
    }
}
