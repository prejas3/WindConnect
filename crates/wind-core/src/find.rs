//! CCS/1 M:find/1, find my phone (§9.5, WN-50). The asker opens one bidi `find` stream per ring:
//! `Ring` [0,{}], `Stop` [1,{}]; the ringer answers `State` [0,{0:state}]: 0 ringing, then one final 1..=4 and FIN.
//! RULE:a ring outlives its stream and session (the person is still searching) until stopped here, by `Stop`
//! or after [`RING_LIMIT`]; a second `Ring` takes the ring over (the old stream gets 2). Nothing persisted.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    protocol::{cbor::Value, Channel, ErrorCode, RawMessage},
    session::{write_raw, FindStream, Session, SessionError},
};

/// The longest ring.
pub const RING_LIMIT: Duration = Duration::from_secs(60);
/// `Ring` must arrive this soon after the stream opens.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// From the asker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Ring,
    Stop,
}

impl Request {
    pub fn encode(self) -> RawMessage {
        RawMessage::new(
            match self {
                Self::Ring => 0,
                Self::Stop => 1,
            },
            Value::Map(vec![]),
        )
    }
    /// `None` for an unknown type or a body that isn't a map.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        if !matches!(m.body, Value::Map(_)) {
            return None;
        }
        match m.ty {
            0 => Some(Self::Ring),
            1 => Some(Self::Stop),
            _ => None,
        }
    }
}

/// From the ringer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The sound is playing.
    Ringing,
    /// Stopped on the ringing device, by its user.
    StoppedHere,
    /// Stopped on request (`Stop`), or taken over by a newer `Ring`.
    StoppedByRequest,
    /// [`RING_LIMIT`] passed.
    TimedOut,
    /// Can't ring (no sound, or turned off).
    Unavailable,
}

impl State {
    pub fn code(self) -> u64 {
        match self {
            Self::Ringing => 0,
            Self::StoppedHere => 1,
            Self::StoppedByRequest => 2,
            Self::TimedOut => 3,
            Self::Unavailable => 4,
        }
    }
    pub fn is_final(self) -> bool {
        self != Self::Ringing
    }
    pub fn encode(self) -> RawMessage {
        RawMessage::new(
            0,
            Value::Map(vec![(Value::Int(0), Value::Int(self.code().into()))]),
        )
    }
    /// An unknown state is final and taken as `Unavailable`; a malformed
    /// message is `None`.
    pub fn decode(m: &RawMessage) -> Option<Self> {
        let Value::Map(fields) = &m.body else {
            return None;
        };
        if m.ty != 0 {
            return None;
        }
        let mut code = None;
        for (k, v) in fields {
            match (k, v) {
                (Value::Int(0), Value::Int(n)) if code.is_none() => code = Some(*n),
                (Value::Int(0), _) => return None,
                (Value::Int(_), _) => {}
                _ => return None,
            }
        }
        Some(match code? {
            0 => Self::Ringing,
            1 => Self::StoppedHere,
            2 => Self::StoppedByRequest,
            3 => Self::TimedOut,
            _ => Self::Unavailable,
        })
    }
}

/// The ringing device's sound.
pub trait Sink: Send + Sync {
    /// Starts the sound; true once it is playing. Called off the async runtime.
    fn ring(&self) -> bool;
    /// Stops it (also when it has already stopped).
    fn stop(&self);
}

struct Episode {
    /// The final state, once there is one.
    end: watch::Sender<Option<State>>,
    /// The stream that reports it.
    owner: watch::Sender<u64>,
}

/// Serves `find` streams on the ringing device.
pub struct Ringer {
    sink: Arc<dyn Sink>,
    limit: Duration,
    episode: Mutex<Option<Arc<Episode>>>,
    next_stream: std::sync::atomic::AtomicU64,
    changes: watch::Sender<bool>,
}

impl Ringer {
    pub fn new(sink: Arc<dyn Sink>) -> Arc<Self> {
        Self::with_limit(sink, RING_LIMIT)
    }

    pub fn with_limit(sink: Arc<dyn Sink>, limit: Duration) -> Arc<Self> {
        Arc::new(Self {
            sink,
            limit,
            episode: Mutex::new(None),
            next_stream: 0.into(),
            changes: watch::channel(false).0,
        })
    }

    /// Ringing now; changes as `subscribe` reports.
    pub fn is_ringing(&self) -> bool {
        self.episode.lock().unwrap().is_some()
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.changes.subscribe()
    }

    /// The person stopped it here (state 1 to the asker).
    pub fn stop_here(&self) {
        self.finish(State::StoppedHere);
    }

    fn finish(&self, state: State) {
        let Some(episode) = self.episode.lock().unwrap().take() else {
            return;
        };
        episode.end.send_replace(Some(state));
        self.sink.stop();
        self.changes.send_replace(false);
    }

    /// Serves the streams of one session until it ends; a ring outlives it.
    pub async fn serve(self: Arc<Self>, mut streams: mpsc::Receiver<FindStream>) {
        while let Some(stream) = streams.recv().await {
            tokio::spawn(self.clone().handle(stream));
        }
    }

    async fn handle(self: Arc<Self>, mut stream: FindStream) {
        let id = self
            .next_stream
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        match tokio::time::timeout(REQUEST_TIMEOUT, stream.recv.next_raw()).await {
            Ok(Ok(Some(m))) if Request::decode(&m) == Some(Request::Ring) => {}
            Ok(Ok(Some(m))) => {
                let code = if Request::decode(&m).is_some() {
                    ErrorCode::ProtocolViolation
                } else {
                    ErrorCode::UnknownMessage
                };
                return abort(&mut stream, code);
            }
            Ok(Err(_)) => return abort(&mut stream, ErrorCode::MalformedFrame),
            Ok(Ok(None)) => return,
            Err(_) => return abort(&mut stream, ErrorCode::Cancelled),
        }
        let existing = self.episode.lock().unwrap().clone();
        let episode = match existing {
            Some(e) => {
                e.owner.send_replace(id);
                e
            }
            None => {
                let sink = self.sink.clone();
                let playing = tokio::task::spawn_blocking(move || sink.ring())
                    .await
                    .unwrap_or(false);
                if !playing {
                    let _ = write_raw(
                        &mut stream.send,
                        Channel::Find,
                        &State::Unavailable.encode(),
                    )
                    .await;
                    let _ = stream.send.finish();
                    return;
                }
                let mut slot = self.episode.lock().unwrap();
                match slot.as_ref() {
                    // Another Ring started it meanwhile: take it over.
                    Some(e) => {
                        e.owner.send_replace(id);
                        e.clone()
                    }
                    None => {
                        let e = Arc::new(Episode {
                            end: watch::channel(None).0,
                            owner: watch::channel(id).0,
                        });
                        *slot = Some(e.clone());
                        drop(slot);
                        self.changes.send_replace(true);
                        let (ringer, ended) = (self.clone(), e.end.subscribe());
                        tokio::spawn(async move {
                            let mut ended = ended;
                            let timed_out =
                                tokio::time::timeout(ringer.limit, ended.wait_for(|s| s.is_some()))
                                    .await
                                    .is_err();
                            if timed_out {
                                ringer.finish(State::TimedOut);
                            }
                        });
                        e
                    }
                }
            }
        };
        if write_raw(&mut stream.send, Channel::Find, &State::Ringing.encode())
            .await
            .is_err()
        {
            return; // the asker is gone; the ring goes on
        }
        let mut end = episode.end.subscribe();
        let mut owner = episode.owner.subscribe();
        let state = loop {
            tokio::select! {
                m = stream.recv.next_raw() => match m {
                    Ok(Some(m)) if Request::decode(&m) == Some(Request::Stop) => {
                        if *episode.owner.borrow() == id { self.finish(State::StoppedByRequest); }
                    }
                    Ok(Some(_)) => {}
                    // FIN or a broken stream: the asker left; keep ringing.
                    _ => return,
                },
                r = end.wait_for(|s| s.is_some()) => match r {
                    Ok(s) => break s.expect("waited for Some"),
                    Err(_) => return,
                },
                r = owner.wait_for(|o| *o != id) => {
                    if r.is_err() { return; }
                    break State::StoppedByRequest;
                },
            }
        };
        let _ = write_raw(&mut stream.send, Channel::Find, &state.encode()).await;
        let _ = stream.send.finish();
    }
}

fn abort(stream: &mut FindStream, code: ErrorCode) {
    stream.recv.stop(code);
    let _ = stream.send.reset(crate::session::varint_code(code));
}

/// The asker's view of one ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// Sent; the sound hasn't started yet.
    Starting,
    Ringing,
    /// The final state; `None` when the stream or session broke first
    /// (the phone may still be ringing).
    Ended(Option<State>),
}

/// One ring asked of the peer: its state, and a way to stop it.
pub struct Ring {
    pub state: watch::Receiver<Asked>,
    stop: Mutex<Option<oneshot::Sender<()>>>,
}

impl Ring {
    /// Asks the peer to stop (state 2 follows).
    pub fn stop(&self) {
        if let Some(stop) = self.stop.lock().unwrap().take() {
            let _ = stop.send(());
        }
    }
}

/// Asks the peer to ring (§9.5). `CapabilityMissing` when it doesn't take `find/1`.
pub async fn ring(session: &Session) -> Result<Ring, SessionError> {
    let mut stream = session.open_find().await?;
    write_raw(&mut stream.send, Channel::Find, &Request::Ring.encode()).await?;
    let (state_tx, state) = watch::channel(Asked::Starting);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let mut stopping = false;
        let end = loop {
            tokio::select! {
                m = stream.recv.next_raw() => match m {
                    Ok(Some(m)) => match State::decode(&m) {
                        Some(State::Ringing) => { state_tx.send_replace(Asked::Ringing); }
                        Some(s) => break Some(s),
                        None => {}
                    },
                    _ => break None,
                },
                r = &mut stop_rx, if !stopping => {
                    stopping = true;
                    if r.is_ok() && write_raw(&mut stream.send, Channel::Find, &Request::Stop.encode()).await.is_err() {
                        break None;
                    }
                },
            }
        };
        let _ = stream.send.finish();
        state_tx.send_replace(Asked::Ended(end));
    });
    Ok(Ring {
        state,
        stop: Mutex::new(Some(stop_tx)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::encode_raw_frame;

    #[test]
    fn codec_and_bytes() {
        for r in [Request::Ring, Request::Stop] {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
        for s in [
            State::Ringing,
            State::StoppedHere,
            State::StoppedByRequest,
            State::TimedOut,
            State::Unavailable,
        ] {
            assert_eq!(State::decode(&s.encode()), Some(s));
        }
        assert_eq!(
            encode_raw_frame(Channel::Find, &Request::Ring.encode()).unwrap(),
            [0x03, 0x82, 0x00, 0xa0]
        );
        assert_eq!(
            encode_raw_frame(Channel::Find, &Request::Stop.encode()).unwrap(),
            [0x03, 0x82, 0x01, 0xa0]
        );
        assert_eq!(
            encode_raw_frame(Channel::Find, &State::Ringing.encode()).unwrap(),
            [0x05, 0x82, 0x00, 0xa1, 0x00, 0x00]
        );
        assert_eq!(
            encode_raw_frame(Channel::Find, &State::StoppedHere.encode()).unwrap(),
            [0x05, 0x82, 0x00, 0xa1, 0x00, 0x01]
        );
    }

    #[test]
    fn odd_messages() {
        let state = |n: i128| RawMessage::new(0, Value::Map(vec![(Value::Int(0), Value::Int(n))]));
        assert_eq!(
            State::decode(&state(9)),
            Some(State::Unavailable),
            "unknown state is final"
        );
        assert_eq!(State::decode(&RawMessage::new(0, Value::Map(vec![]))), None);
        assert_eq!(State::decode(&RawMessage::new(1, Value::Map(vec![]))), None);
        assert_eq!(
            Request::decode(&RawMessage::new(2, Value::Map(vec![]))),
            None
        );
        assert_eq!(Request::decode(&RawMessage::new(0, Value::Int(0))), None);
    }
}
