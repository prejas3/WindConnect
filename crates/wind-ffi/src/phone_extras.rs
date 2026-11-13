//! CCS/1 M:WN-48 the phone's call state for the PC (call/1, §9.10) and both sides' volume (volume/1, §9.9) on one session.
//! FLOW:Kotlin sets the call state / its STREAM_MUSIC level -> watch -> sent when it changes or when the PC starts accepting
//! (the state at every session start, idle too). The PC's `Level` and `Set` -> [`VolumeSink`]. RULE:no polling; a `Level` at
//! most every [`wind_core::volume::PERIOD`], `Set`s applied at most that often with the latest winning.
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wind_core::{
    call::{self, CallState},
    protocol::Capability,
    session::{ChannelMessage, Session},
    volume::{self, Message, PERIOD},
};

/// WN-48: the PC's volume and its requests for the phone's (called off the runtime's workers).
#[uniffi::export(callback_interface)]
pub trait VolumeSink: Send + Sync {
    /// The PC's output volume, 0..=100.
    fn peer_level(&self, level: u8, muted: bool);
    /// The PC asks to change STREAM_MUSIC: only what is given.
    fn set(&self, level: Option<u8>, muted: Option<bool>);
}

pub(crate) type VolumeRoute = (Arc<dyn VolumeSink>, mpsc::Receiver<ChannelMessage>);

fn accepts(caps: &[Capability], name: &str) -> bool {
    caps.iter().any(|c| c.name() == name && c.version() == 1)
}

/// The per-session tasks; they end with the session (or when this is dropped).
pub(crate) struct PhoneExtras {
    call: watch::Sender<CallState>,
    level: watch::Sender<Option<(u8, bool)>>,
    enabled: watch::Sender<bool>,
    sets: mpsc::Sender<(Option<u8>, Option<bool>)>,
    tasks: Vec<JoinHandle<()>>,
}

impl PhoneExtras {
    pub fn start(
        session: Session,
        caps: watch::Receiver<Vec<Capability>>,
        volume: Option<VolumeRoute>,
        volume_enabled: bool,
    ) -> Self {
        let (call_tx, call_rx) = watch::channel(CallState::Idle);
        let (level_tx, level_rx) = watch::channel(None);
        let (enabled_tx, enabled_rx) = watch::channel(volume_enabled);
        let (sets_tx, sets_rx) = mpsc::channel(32);
        let mut tasks = vec![
            tokio::spawn(send_call(session.clone(), caps.clone(), call_rx)),
            tokio::spawn(send_level(
                session.clone(),
                caps.clone(),
                level_rx,
                enabled_rx,
            )),
            tokio::spawn(send_sets(session.clone(), sets_rx)),
        ];
        if let Some((sink, queue)) = volume {
            tasks.push(tokio::spawn(serve_volume(sink, queue)));
        }
        Self {
            call: call_tx,
            level: level_tx,
            enabled: enabled_tx,
            sets: sets_tx,
            tasks,
        }
    }

    pub fn set_call(&self, state: CallState) {
        self.call.send_replace(state);
    }

    pub fn set_level(&self, level: u8, muted: bool) {
        self.level.send_replace(Some((level.min(100), muted)));
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.send_replace(enabled);
    }

    pub fn request(&self, level: Option<u8>, muted: Option<bool>) {
        let _ = self.sets.try_send((level.map(|l| l.min(100)), muted));
    }
}

impl Drop for PhoneExtras {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// CCS/1 RULE:the state goes out at the start (idle too: a PC that paused during a dropped session resumes), on every change,
/// and again whenever the PC starts accepting `call/1`.
async fn send_call(
    session: Session,
    mut caps: watch::Receiver<Vec<Capability>>,
    mut state: watch::Receiver<CallState>,
) {
    let mut accepted = accepts(&caps.borrow_and_update(), "call");
    let mut sent: Option<CallState> = None;
    loop {
        let now = *state.borrow_and_update();
        if accepted && sent != Some(now) {
            match call::send(&session, now).await {
                Ok(true) => sent = Some(now),
                Ok(false) => {}
                Err(_) => return,
            }
        }
        tokio::select! {
            changed = state.changed() => if changed.is_err() { return },
            changed = caps.changed() => {
                if changed.is_err() { return }
                let next = accepts(&caps.borrow_and_update(), "call");
                if next && !accepted { sent = None }
                accepted = next;
            }
        }
    }
}

/// The phone's own level, while both sides take `volume/1`; at most every [`PERIOD`].
async fn send_level(
    session: Session,
    mut caps: watch::Receiver<Vec<Capability>>,
    mut level: watch::Receiver<Option<(u8, bool)>>,
    mut enabled: watch::Receiver<bool>,
) {
    let mut accepted = accepts(&caps.borrow_and_update(), "volume");
    let mut sent: Option<(u8, bool)> = None;
    loop {
        let now = *level.borrow_and_update();
        let on = *enabled.borrow_and_update();
        if let (true, true, Some((l, m))) = (accepted, on, now) {
            if sent != now {
                match volume::send(&session, &Message::Level { level: l, muted: m }).await {
                    Ok(true) => sent = now,
                    Ok(false) => {}
                    Err(_) => return,
                }
                tokio::time::sleep(PERIOD).await;
                continue;
            }
        }
        tokio::select! {
            changed = level.changed() => if changed.is_err() { return },
            changed = enabled.changed() => {
                if changed.is_err() { return }
                if !*enabled.borrow() { sent = None }
            }
            changed = caps.changed() => {
                if changed.is_err() { return }
                let next = accepts(&caps.borrow_and_update(), "volume");
                if next && !accepted { sent = None }
                accepted = next;
            }
        }
    }
}

/// The person's changes to the PC's volume: the latest wins, at most every [`PERIOD`].
async fn send_sets(session: Session, mut sets: mpsc::Receiver<(Option<u8>, Option<bool>)>) {
    while let Some((level, muted)) = volume::latest(&mut sets).await {
        if volume::send(&session, &Message::Set { level, muted })
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(PERIOD).await;
    }
}

/// The PC's `Level`s and `Set`s: of what is queued, the latest of each kind; `Set`s at most every [`PERIOD`].
async fn serve_volume(sink: Arc<dyn VolumeSink>, mut queue: mpsc::Receiver<ChannelMessage>) {
    while let Some(first) = queue.recv().await {
        let (mut level, mut set) = (None, None);
        let mut take = |m: &ChannelMessage| match Message::decode(&m.message) {
            Some(Message::Level { level: l, muted }) => level = Some((l, muted)),
            Some(Message::Set { level: l, muted }) => set = Some((l, muted)),
            None => {}
        };
        take(&first);
        while let Ok(next) = queue.try_recv() {
            take(&next);
        }
        let sink = sink.clone();
        let applied = set.is_some();
        let _ = tokio::task::spawn_blocking(move || {
            if let Some((l, m)) = level {
                sink.peer_level(l, m);
            }
            if let Some((l, m)) = set {
                sink.set(l, m);
            }
        })
        .await;
        if applied {
            tokio::time::sleep(PERIOD).await;
        }
    }
}
