//! The router: which feature module receives which channel (§4: the module
//! is chosen by `(channel, type)`; a module owns a channel and dispatches
//! on `type` itself).
//!
//! A module registers its channel and the capability version it accepts,
//! and gets a queue of messages. The router's registrations are exactly
//! what the session announces in `Hello` / `HelloAck`: a capability means
//! "I accept" (§6), and nothing unregistered is accepted.

use std::collections::BTreeMap;

use tokio::sync::mpsc;

use crate::protocol::{Capability, Channel, RawMessage};

use super::framed::FramedRecv;

/// A message for a module, in order within its channel (§3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMessage {
    pub channel: Channel,
    pub message: RawMessage,
}

/// A `files` stream opened by the peer (one per transfer): its frames in
/// `recv`, the replies in `send`. The header has been read.
#[derive(Debug)]
pub struct FileStream {
    pub send: quinn::SendStream,
    pub recv: FramedRecv,
}

/// The capability name of a channel (§6); `control` has none.
pub fn capability_name(channel: Channel) -> Option<&'static str> {
    Some(match channel {
        Channel::Control => return None,
        Channel::Clipboard => "clipboard",
        Channel::Notify => "notify",
        Channel::Media => "media",
        Channel::Input => "input",
        Channel::Files => "files",
    })
}

/// Whether a channel's streams are uni (message queues, §3.1); `files` and
/// `control` are bidi.
pub fn is_uni(channel: Channel) -> bool {
    matches!(
        channel,
        Channel::Clipboard | Channel::Notify | Channel::Media | Channel::Input
    )
}

/// Which module receives each channel. Built before the session starts.
#[derive(Debug, Default)]
pub struct Router {
    pub(super) uni: BTreeMap<u8, (Channel, u64, mpsc::Sender<ChannelMessage>)>,
    pub(super) files: Option<(u64, mpsc::Sender<FileStream>)>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// Routes the messages of uni `channel` (capability version `version`)
    /// to the returned queue. A full queue holds back that channel's
    /// stream only (QUIC flow control), never other channels.
    ///
    /// Panics for `control` (the session's own) and `files` (use
    /// [`route_files`](Self::route_files)).
    pub fn route(
        &mut self,
        channel: Channel,
        version: u64,
        queue: usize,
    ) -> mpsc::Receiver<ChannelMessage> {
        assert!(is_uni(channel), "{channel:?} is not a uni channel");
        let (tx, rx) = mpsc::channel(queue.max(1));
        self.uni.insert(channel.byte(), (channel, version, tx));
        rx
    }

    /// Routes incoming `files` streams to the returned queue.
    pub fn route_files(&mut self, version: u64, queue: usize) -> mpsc::Receiver<FileStream> {
        let (tx, rx) = mpsc::channel(queue.max(1));
        self.files = Some((version, tx));
        rx
    }

    /// What the session announces: one capability per routed channel.
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut caps: Vec<Capability> = self
            .uni
            .values()
            .map(|(ch, v, _)| (*ch, *v))
            .chain(self.files.as_ref().map(|(v, _)| (Channel::Files, *v)))
            .filter_map(|(ch, v)| Capability::new(capability_name(ch)?, v))
            .collect();
        caps.sort_by_key(|c| c.to_string());
        caps
    }
}
