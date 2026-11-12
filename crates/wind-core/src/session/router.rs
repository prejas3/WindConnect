//! The router: which feature module receives which channel (§4: the module
//! is chosen by `(channel, type)`; a module owns a channel and dispatches
//! on `type` itself).
//!
//! A module registers its channel and the capability version it accepts,
//! and gets a queue of messages. The router's registrations are exactly
//! what the session announces in `Hello` / `HelloAck`: a capability means
//! "I accept" (§6), and nothing unregistered is accepted.

use std::collections::{BTreeMap, BTreeSet};

use tokio::sync::mpsc;

use crate::protocol::{Capability, Channel, RawMessage};

use super::framed::FramedRecv;
use crate::input::InputEvent;

/// A message for a module, in order within its channel (§3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMessage {
    pub channel: Channel,
    pub message: RawMessage,
}

/// A bidi stream of a feature channel, its header read (or written, for
/// one this side opened): the peer's frames in `recv`, ours in `send`.
/// Withdrawing its channel (`Session::set_accepted`) makes the next read of
/// a stream the peer opened fail with `RecvError::Withdrawn`, the stream
/// stopped with `CANCELLED`.
#[derive(Debug)]
pub struct BidiStream {
    pub send: quinn::SendStream,
    pub recv: FramedRecv,
}

/// A `files` stream (1–8 per transfer, §9.1).
pub type FileStream = BidiStream;

/// An `actions` stream: one request and its reply (§9.2, WN-54).
pub type ActionStream = BidiStream;

/// A `find` stream: one ring and its states (§9.5, WN-50).
pub type FindStream = BidiStream;

/// The capability name of a channel (§6); `control` has none.
pub fn capability_name(channel: Channel) -> Option<&'static str> {
    Some(match channel {
        Channel::Control => return None,
        Channel::Clipboard => "clipboard",
        Channel::Notify => "notify",
        Channel::Media => "media",
        Channel::Input => "input",
        Channel::Files => "files",
        Channel::Actions => "actions",
        Channel::Battery => "battery",
        Channel::Find => "find",
    })
}

/// Whether a channel's streams are uni (message queues, §3.1); `files`,
/// `actions` and `control` are bidi.
pub fn is_uni(channel: Channel) -> bool {
    matches!(
        channel,
        Channel::Clipboard | Channel::Notify | Channel::Media | Channel::Input | Channel::Battery
    )
}

/// Which module receives each channel. Built before the session starts.
#[derive(Debug, Default)]
pub struct Router {
    pub(super) uni: BTreeMap<u8, (Channel, u64, mpsc::Sender<ChannelMessage>)>,
    pub(super) files: Option<(u64, mpsc::Sender<FileStream>)>,
    pub(super) actions: Option<(u64, mpsc::Sender<ActionStream>)>,
    pub(super) find: Option<mpsc::Sender<FindStream>>,
    /// Routed but not announced at the start (WN-50): `Session::set_accepted` turns them on.
    pub(super) disabled: BTreeSet<u8>,
    pub(super) input: Option<mpsc::Sender<InputEvent>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// Routes the messages of uni `channel` (capability versions 1 to
    /// `version`) to the returned queue. A full queue holds back that channel's
    /// stream only (QUIC flow control), never other channels.
    ///
    /// Panics for `control` (the session's own), `files` (use
    /// [`route_files`](Self::route_files)) and `input` (use
    /// [`route_input`](Self::route_input)).
    pub fn route(
        &mut self,
        channel: Channel,
        version: u64,
        queue: usize,
    ) -> mpsc::Receiver<ChannelMessage> {
        assert!(is_uni(channel), "{channel:?} is not a uni channel");
        assert!(
            channel != Channel::Input,
            "input is routed with route_input"
        );
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

    /// Routes incoming `actions` streams (requests to run an action here,
    /// §9.2) to the returned queue; see [`crate::actions::ActionHost`].
    pub fn route_actions(&mut self, queue: usize) -> mpsc::Receiver<ActionStream> {
        let (tx, rx) = mpsc::channel(queue.max(1));
        self.actions = Some((1, tx));
        rx
    }

    /// Routes incoming `find` streams (asks to ring here, §9.5) to the
    /// returned queue; see [`crate::find::Ringer`].
    pub fn route_find(&mut self, queue: usize) -> mpsc::Receiver<FindStream> {
        let (tx, rx) = mpsc::channel(queue.max(1));
        self.find = Some(tx);
        rx
    }

    /// Starts the session with routed `channel` withdrawn: not announced in
    /// `Hello`, its streams refused, until `Session::set_accepted(channel, true)`.
    pub fn start_disabled(&mut self, channel: Channel) {
        self.disabled.insert(channel.byte());
    }

    /// Accepts input (`input/1`, §8): the session runs the receiver state
    /// machine over the `input` stream and datagrams, and the returned queue
    /// gets what should reach the system, late and stale state already
    /// dropped.
    pub fn route_input(&mut self, queue: usize) -> mpsc::Receiver<InputEvent> {
        let (tx, rx) = mpsc::channel(queue.max(1));
        self.input = Some(tx);
        rx
    }

    /// What the session announces: per routed channel, every version up to
    /// the routed one (a module that takes version N takes 1..=N).
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut caps: Vec<Capability> = self
            .uni
            .values()
            .map(|(ch, v, _)| (*ch, *v))
            .chain(self.files.as_ref().map(|(v, _)| (Channel::Files, *v)))
            .chain(self.actions.as_ref().map(|(v, _)| (Channel::Actions, *v)))
            .chain(self.find.as_ref().map(|_| (Channel::Find, 1)))
            .chain(self.input.as_ref().map(|_| (Channel::Input, 1)))
            .filter(|(ch, _)| !self.disabled.contains(&ch.byte()))
            .flat_map(|(ch, v)| (1..=v).map(move |v| (ch, v)))
            .filter_map(|(ch, v)| Capability::new(capability_name(ch)?, v))
            .collect();
        caps.sort_by_key(|c| c.to_string());
        caps
    }
}
