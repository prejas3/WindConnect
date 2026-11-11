//! Removing a pairing on both sides (WC-28, PROTOCOL.md §12.10).
//!
//! CCS/1 FLOW:unpair -> `-<key>` (notice owed) + revoke + forget tickets -> Goodbye UNPAIRED
//! (now in the session, or later on a notice-mode connection) -> acked -> `=<key>` (settled).
//! RULE:one path online and offline; settled only on the peer's acknowledgement.
//! SEC:only the authenticated peer's key (TLS) is ever removed; no message carries a key.

use std::io;
use std::sync::Arc;

use super::store::PairedDevicesFile;
use crate::identity::PublicKey;
use crate::protocol::ErrorCode;
use crate::session::{Session, SessionError};
use crate::transport::{Endpoint, PinnedKeys};

/// The paired-device list, the pinned keys and the endpoint, kept in step.
#[derive(Clone)]
pub struct Unpairing {
    store: PairedDevicesFile,
    pins: Arc<PinnedKeys>,
    endpoint: Endpoint,
}

impl Unpairing {
    pub fn new(store: PairedDevicesFile, pins: Arc<PinnedKeys>, endpoint: Endpoint) -> Self {
        Unpairing {
            store,
            pins,
            endpoint,
        }
    }

    /// Removes `key` here and tells its device. With `session` to that very
    /// device the notice goes at once; otherwise (or if it isn't
    /// acknowledged) it is owed until the device's next connection, which
    /// is then in notice mode. True when the notice was delivered now.
    pub async fn unpair(&self, key: PublicKey, session: Option<&Session>) -> io::Result<bool> {
        self.store.remove(&key)?;
        self.pins.revoke(key);
        self.endpoint.forget(&key);
        if let Some(session) = session.filter(|s| s.peer().key == key) {
            if session.goodbye(ErrorCode::Unpaired, None).await {
                self.settle(&key)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `key`'s notice is delivered: nothing is owed any more.
    pub fn settle(&self, key: &PublicKey) -> io::Result<()> {
        self.store.settle(key)?;
        self.pins.settle(key);
        Ok(())
    }

    /// The peer of a session sent `Goodbye` `UNPAIRED`: drop its key, the
    /// one from the TLS handshake (`session.peer().key`), never anything
    /// named in a message.
    pub fn unpaired_by(&self, session: &Session) -> io::Result<()> {
        let key = session.peer().key;
        self.store.settle(&key)?;
        self.pins.settle(&key);
        self.endpoint.forget(&key);
        Ok(())
    }

    /// Settles a delivered notice from a notice-mode connection's result;
    /// true if `error` was such a notice (delivered or not).
    pub fn notice_result(&self, error: &SessionError) -> io::Result<bool> {
        match error {
            SessionError::Unpaired {
                peer,
                delivered: true,
            } => self.settle(peer).map(|()| true),
            SessionError::Unpaired { .. } => Ok(true),
            _ => Ok(false),
        }
    }

    pub fn store(&self) -> &PairedDevicesFile {
        &self.store
    }
}
