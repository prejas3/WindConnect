//! CCS/1 IO:session lifecycle -> event-driven tray state | SEC:never pins a peer or trusts discovery.
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;
use wind_core::session::Session;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Connected,
    Searching,
    Unavailable,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => crate::i18n::tr("tray.connected"),
            Self::Searching => crate::i18n::tr("tray.searching"),
            Self::Unavailable => crate::i18n::tr("tray.unavailable"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Update {
    pub sequence: u64,
    pub phase: Phase,
    pub changed: Instant,
}
struct Inner {
    generation: u64,
    update: Update,
}
#[derive(Clone)]
pub struct Indicator {
    inner: Arc<Mutex<Inner>>,
    updates: watch::Sender<Update>,
}
impl Default for Indicator {
    fn default() -> Self {
        let update = Update {
            sequence: 0,
            phase: Phase::Unavailable,
            changed: Instant::now(),
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                generation: 0,
                update: update.clone(),
            })),
            updates: watch::channel(update).0,
        }
    }
}
impl Indicator {
    pub fn subscribe(&self) -> watch::Receiver<Update> {
        self.updates.subscribe()
    }
    pub fn snapshot(&self) -> Update {
        self.updates.borrow().clone()
    }
    fn publish(&self, inner: &mut Inner, phase: Phase) {
        inner.update = Update {
            sequence: inner.update.sequence.wrapping_add(1),
            phase,
            changed: Instant::now(),
        };
        self.updates.send_replace(inner.update.clone());
    }
    pub fn searching(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.generation = inner.generation.wrapping_add(1);
        self.publish(&mut inner, Phase::Searching);
        inner.generation
    }
    pub fn unavailable(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.generation = inner.generation.wrapping_add(1);
        self.publish(&mut inner, Phase::Unavailable);
    }
    pub fn attempt_failed(&self, generation: u64) {
        self.transition(generation, Phase::Unavailable);
    }
    pub fn connected(&self, generation: u64) {
        self.transition(generation, Phase::Connected);
    }
    fn transition(&self, generation: u64, phase: Phase) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if generation == inner.generation {
            self.publish(&mut inner, phase);
        }
    }
    /// Caller owns this future; dropping it cancels the session observation.
    pub async fn track_session(&self, generation: u64, session: &Session) {
        self.transition(generation, Phase::Connected);
        session.closed().await;
        self.transition(generation, Phase::Unavailable);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_old_session_cannot_overwrite_a_new_attempt() {
        let state = Indicator::default();
        let old = state.searching();
        let current = state.searching();
        state.transition(current, Phase::Connected);
        state.transition(old, Phase::Unavailable);
        assert_eq!(state.snapshot().phase, Phase::Connected);
        state.unavailable();
        state.transition(current, Phase::Connected);
        assert_eq!(state.snapshot().phase, Phase::Unavailable);
    }
}
