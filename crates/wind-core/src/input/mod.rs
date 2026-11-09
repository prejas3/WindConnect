//! The `input` channel (PROTOCOL.md §8, WC-13): pointer and scroll as
//! unreliable QUIC datagrams with cumulative state, button transitions on
//! the reliable `input` stream, and a receiver that merges both and drops
//! late or stale state.
//!
//! A [`crate::session::Session`] wires them up: `Session::input` gives an
//! [`InputSender`]; `Router::route_input` gives a queue of [`InputEvent`]s.

mod receiver;
mod sender;

pub use receiver::{Class, InputEvent, InputReceiver};
pub use sender::{InputAnnouncement, InputSender, SenderCore, REPEATS};
