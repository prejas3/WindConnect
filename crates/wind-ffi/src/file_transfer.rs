//! CCS/1 SEC:only staged regular files below the configured private outbox cross the boundary.
use crate::{public_key, CoreClient, CoreConnection, CoreError};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tokio::sync::watch;
use wind_core::{
    files::{id_hex, FileEngine, FilesConfig, TransferInfo, TransferState},
    session::{Session, SessionEvent},
    transport::PeerTrust,
};

#[derive(Clone, Debug, uniffi::Record)]
pub struct FileTransfer {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub state: String,
}

fn invalid(reason: impl ToString) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

struct Runtime {
    engine: FileEngine,
    outbox: PathBuf,
}
#[derive(Default)]
pub(crate) struct FileBridge {
    runtime: Mutex<Option<Runtime>>,
    updates: tokio::sync::Mutex<Option<watch::Receiver<Vec<TransferInfo>>>>,
}
impl FileBridge {
    fn engine(&self) -> Result<FileEngine, CoreError> {
        self.runtime
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.engine.clone())
            .ok_or_else(|| invalid("files are not configured"))
    }
    fn rows(infos: &[TransferInfo]) -> Vec<FileTransfer> {
        infos
            .iter()
            .rev()
            .take(1000)
            .map(|t| FileTransfer {
                id: id_hex(&t.id),
                name: t.path.clone(),
                size: t.size,
                done: t.done,
                state: match t.state {
                    TransferState::Queued => "queued",
                    TransferState::Running => "running",
                    TransferState::Waiting => "waiting",
                    TransferState::Done => "done",
                    TransferState::Failed(_) => "failed",
                    TransferState::Cancelled { .. } => "cancelled",
                }
                .into(),
            })
            .collect()
    }
}

/// CCS/1 FLOW:session capability events -> watch; standby probes never attach the file engine.
pub(crate) struct FileCapabilities {
    value: watch::Sender<bool>,
    /// WN-48: everything the peer accepts, as it changes (call/1, volume/1 follow it).
    caps: watch::Sender<Vec<wind_core::protocol::Capability>>,
    updates: tokio::sync::Mutex<watch::Receiver<bool>>,
    task: tokio::task::JoinHandle<()>,
}
impl FileCapabilities {
    pub fn new(
        session: &Session,
        mut events: tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
    ) -> Self {
        let accepted = session
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "files" && c.version() == 1);
        let (value, updates) = watch::channel(accepted);
        let sender = value.clone();
        let (caps_tx, _) = watch::channel(session.peer_capabilities());
        let all = caps_tx.clone();
        let task = tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if let SessionEvent::Capabilities(caps) = event {
                    all.send_replace(caps.clone());
                    sender.send_if_modified(|old| {
                        let new = caps.iter().any(|c| c.name() == "files" && c.version() == 1);
                        if *old == new {
                            false
                        } else {
                            *old = new;
                            true
                        }
                    });
                }
            }
            sender.send_replace(false);
            all.send_replace(Vec::new());
        });
        Self {
            value,
            caps: caps_tx,
            updates: tokio::sync::Mutex::new(updates),
            task,
        }
    }
}
impl FileCapabilities {
    /// What the peer accepts, now and as it changes (empty once the session ended).
    pub fn peer_caps(&self) -> watch::Receiver<Vec<wind_core::protocol::Capability>> {
        self.caps.subscribe()
    }
}
impl Drop for FileCapabilities {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreClient {
    pub async fn configure_files(
        &self,
        inbox: String,
        partials: String,
        outbox: String,
    ) -> Result<(), CoreError> {
        let mut updates = self.files.updates.lock().await;
        let mut runtime = self.files.runtime.lock().unwrap();
        if runtime.is_some() {
            return Err(invalid("files already configured"));
        }
        std::fs::create_dir_all(&outbox).map_err(invalid)?;
        let root = std::fs::canonicalize(outbox).map_err(invalid)?;
        let engine = FileEngine::new(FilesConfig::new(inbox, partials)).map_err(invalid)?;
        *updates = Some(engine.subscribe());
        *runtime = Some(Runtime {
            engine,
            outbox: root,
        });
        Ok(())
    }
    pub async fn next_file_transfers(&self) -> Result<Vec<FileTransfer>, CoreError> {
        let mut updates = self.files.updates.lock().await;
        let rx = updates
            .as_mut()
            .ok_or_else(|| invalid("files are not configured"))?;
        rx.changed().await.map_err(invalid)?;
        let rows = FileBridge::rows(&rx.borrow_and_update());
        Ok(rows)
    }
    pub fn file_transfers(&self) -> Result<Vec<FileTransfer>, CoreError> {
        Ok(FileBridge::rows(&self.files.engine()?.transfers()))
    }
    pub fn cancel_file(&self, id: String) -> Result<(), CoreError> {
        let id = parse_id(&id)?;
        self.files.engine()?.cancel(&id).map_err(invalid)
    }
}

fn parse_id(id: &str) -> Result<[u8; 16], CoreError> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("bad transfer id"));
    }
    let mut bytes = [0; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).map_err(invalid)?;
    }
    Ok(bytes)
}

fn staged_path(root: &Path, source: &Path) -> Result<PathBuf, CoreError> {
    let meta = std::fs::symlink_metadata(source).map_err(invalid)?;
    let canonical = source.canonicalize().map_err(invalid)?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || canonical != source
        || !canonical.starts_with(root)
    {
        return Err(invalid(
            "source must be a regular file in the configured outbox",
        ));
    }
    Ok(canonical)
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreConnection {
    pub fn files_accepted(&self) -> bool {
        *self.file_caps.value.borrow()
    }
    pub async fn next_files_accepted(&self) -> Result<bool, CoreError> {
        let mut rx = self.file_caps.updates.lock().await;
        rx.changed().await.map_err(invalid)?;
        let value = *rx.borrow_and_update();
        Ok(value)
    }
    pub async fn activate_files(&self) {
        if let Ok(engine) = self.files.engine() {
            if !self
                .files_active
                .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                engine.attach(self.session.clone(), None);
            }
        }
    }
    pub async fn queue_files(&self, paths: Vec<String>) -> Result<Vec<String>, CoreError> {
        if paths.is_empty() || paths.len() > 100 {
            return Err(invalid("expected 1–100 staged files"));
        }
        let peer = public_key(&self.peer().public_key)?;
        if !self.files_active.load(std::sync::atomic::Ordering::SeqCst)
            || !self.trust.is_trusted(&peer)
            || !self.files_accepted()
        {
            return Err(invalid("adopted paired peer does not accept files"));
        }
        let (engine, checked) = {
            let runtime = self.files.runtime.lock().unwrap();
            let runtime = runtime
                .as_ref()
                .ok_or_else(|| invalid("files are not configured"))?;
            let checked = paths
                .iter()
                .map(|p| staged_path(&runtime.outbox, Path::new(p)))
                .collect::<Result<Vec<_>, _>>()?;
            (runtime.engine.clone(), checked)
        };
        tokio::task::spawn_blocking(move || {
            let mut ids = Vec::new();
            for path in checked {
                ids.extend(
                    engine
                        .send(*peer.fingerprint().as_bytes(), &path)
                        .map_err(invalid)?
                        .iter()
                        .map(id_hex),
                );
            }
            Ok(ids)
        })
        .await
        .map_err(invalid)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_bounded_and_ascii() {
        assert!(parse_id("../file").is_err());
        assert!(parse_id(&"é".repeat(16)).is_err());
        assert_eq!(parse_id(&"0a".repeat(16)).unwrap(), [10; 16]);
    }
}
