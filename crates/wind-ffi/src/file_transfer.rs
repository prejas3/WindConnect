//! CCS/1 SEC:only staged regular files below the configured private outbox cross the boundary.
use crate::{public_key, session_error, CoreClient, CoreConnection, CoreError};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};
use tokio::sync::watch;
use wind_core::{
    files::{
        id_hex, Direction, FileEngine, FilesConfig, IncomingAdmission, IncomingCommit,
        IncomingPermit, TransferInfo, TransferState, VerifiedIncoming,
    },
    session::{Session, SessionEvent},
    transport::PeerTrust,
};

#[derive(Clone, Debug, uniffi::Record)]
pub struct FileTransfer {
    pub peer: String,
    pub incoming: bool,
    pub id: String,
    pub name: String,
    pub size: u64,
    pub done: u64,
    pub state: String,
    pub failure: Option<String>,
}

fn invalid(reason: impl ToString) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_string(),
    }
}

struct Runtime {
    engine: FileEngine,
    outbox: PathBuf,
    inbox: PathBuf,
}
#[derive(Default)]
pub(crate) struct FileBridge {
    runtime: Mutex<Option<Runtime>>,
    updates: tokio::sync::Mutex<Option<watch::Receiver<Vec<TransferInfo>>>>,
    destination: Mutex<Option<Arc<dyn FileDestination>>>,
    admission: Mutex<Option<Arc<dyn FileAdmission>>>,
}

#[uniffi::export(callback_interface)]
pub trait FileAdmission: Send + Sync {
    fn begin(&self, attempt: u64, peer: String, id: String) -> bool;
    /// Must enqueue cleanup without blocking or querying the native engine synchronously.
    fn end(&self, attempt: u64);
}
struct AdmissionBridge(Weak<FileBridge>);
impl std::fmt::Debug for AdmissionBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AndroidIncomingAdmission")
    }
}
struct AdmissionPermit(Option<(Arc<dyn FileAdmission>, u64)>);
impl IncomingPermit for AdmissionPermit {}
impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        if let Some((sink, attempt)) = self.0.take() {
            sink.end(attempt);
        }
    }
}
impl IncomingAdmission for AdmissionBridge {
    fn admit(
        &self,
        peer: [u8; 32],
        id: wind_core::files::TransferId,
    ) -> std::io::Result<Box<dyn IncomingPermit>> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let bridge = self
            .0
            .upgrade()
            .ok_or_else(|| std::io::Error::other("file client closed"))?;
        let sink = bridge.admission.lock().unwrap().clone();
        let Some(sink) = sink else {
            return Ok(Box::new(AdmissionPermit(None)));
        };
        let attempt = NEXT.fetch_add(1, Ordering::Relaxed);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sink.begin(attempt, hex_peer(&peer), id_hex(&id))
        })) {
            Ok(true) => Ok(Box::new(AdmissionPermit(Some((sink, attempt))))),
            Ok(false) => Err(std::io::Error::other("incoming ownership refused")),
            Err(_) => {
                sink.end(attempt);
                Err(std::io::Error::other("incoming ownership callback failed"))
            }
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct IncomingFile {
    pub peer: String,
    pub id: String,
    pub name: String,
    pub source: String,
    pub size: u64,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum FileCommitStatus {
    Published,
    Cancelled,
    NoSpace,
    Failed,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FileCommitResult {
    pub status: FileCommitStatus,
    pub error: Option<String>,
}
#[derive(uniffi::Object)]
pub struct IncomingControl {
    cancelled: Arc<AtomicBool>,
}
#[uniffi::export]
impl IncomingControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
#[uniffi::export(callback_interface)]
pub trait FileDestination: Send + Sync {
    fn commit(&self, file: IncomingFile, control: Arc<IncomingControl>) -> FileCommitResult;
}
struct CommitBridge(Weak<FileBridge>);
impl std::fmt::Debug for CommitBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AndroidFileDestination")
    }
}
impl IncomingCommit for CommitBridge {
    fn commit(&self, file: &VerifiedIncoming, cancelled: Arc<AtomicBool>) -> std::io::Result<()> {
        let bridge = self
            .0
            .upgrade()
            .ok_or_else(|| std::io::Error::other("file destination closed"))?;
        let inbox = bridge
            .runtime
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.inbox.clone())
            .ok_or_else(|| std::io::Error::other("files not configured"))?;
        let path = file.path.canonicalize()?;
        if !path.starts_with(&inbox)
            || !path.is_file()
            || std::fs::symlink_metadata(&file.path)?
                .file_type()
                .is_symlink()
        {
            return Err(std::io::Error::other("invalid verified private file"));
        }
        let sink = bridge
            .destination
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| std::io::Error::other("file destination unavailable"))?;
        let result = sink.commit(
            IncomingFile {
                peer: hex_peer(&file.peer),
                id: id_hex(&file.id),
                name: file.relative_path.clone(),
                source: path.to_string_lossy().into(),
                size: file.size,
            },
            Arc::new(IncomingControl { cancelled }),
        );
        match result.status {
            FileCommitStatus::Published => Ok(()),
            FileCommitStatus::NoSpace => Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "public destination has no space",
            )),
            FileCommitStatus::Cancelled => Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "publication cancelled",
            )),
            FileCommitStatus::Failed => Err(std::io::Error::other(
                result
                    .error
                    .unwrap_or_else(|| "public publication failed".into())
                    .chars()
                    .take(256)
                    .collect::<String>(),
            )),
        }
    }
}
fn hex_peer(peer: &[u8; 32]) -> String {
    peer.iter().map(|b| format!("{b:02x}")).collect()
}
fn parse_peer(peer: &str) -> Result<[u8; 32], CoreError> {
    if peer.len() != 64 || !peer.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("bad peer fingerprint"));
    }
    let mut value = [0; 32];
    for (i, b) in value.iter_mut().enumerate() {
        *b = u8::from_str_radix(&peer[i * 2..i * 2 + 2], 16).map_err(invalid)?;
    }
    Ok(value)
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
                peer: hex_peer(&t.peer),
                incoming: t.direction == Direction::Incoming,
                id: id_hex(&t.id),
                name: t.path.clone(),
                size: t.size,
                done: t.done,
                failure: match &t.state {
                    TransferState::Failed(wind_core::files::FailReason::NoSpace) => {
                        Some("no_space".into())
                    }
                    TransferState::Failed(_) => Some("failed".into()),
                    _ => None,
                },
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
        let mut config = FilesConfig::new(&inbox, partials);
        config.incoming_admission = Some(Arc::new(AdmissionBridge(Arc::downgrade(&self.files))));
        config.incoming_commit = Some(Arc::new(CommitBridge(Arc::downgrade(&self.files))));
        config.max_incoming_file = 2 * 1024 * 1024 * 1024;
        config.max_incoming_active = 4 * 1024 * 1024 * 1024;
        config.incoming_copies = 2;
        let engine = FileEngine::new(config).map_err(invalid)?;
        *updates = Some(engine.subscribe());
        *runtime = Some(Runtime {
            engine,
            outbox: root,
            inbox: Path::new(&inbox).canonicalize().map_err(invalid)?,
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
        self.files.engine()?.cancel_outgoing(&id).map_err(invalid)
    }
    pub fn set_file_destination(&self, destination: Box<dyn FileDestination>) {
        *self.files.destination.lock().unwrap() = Some(Arc::from(destination));
    }
    pub fn set_file_admission(&self, admission: Box<dyn FileAdmission>) {
        *self.files.admission.lock().unwrap() = Some(Arc::from(admission));
    }
    pub fn cancel_incoming_file(&self, peer: String, id: String) -> Result<(), CoreError> {
        self.files
            .engine()?
            .cancel_incoming(&parse_peer(&peer)?, &parse_id(&id)?)
            .map_err(invalid)
    }
    pub fn release_incoming_file(&self, peer: String, id: String) -> Result<(), CoreError> {
        let peer = parse_peer(&peer)?;
        let id = parse_id(&id)?;
        let (engine, root) = {
            let runtime = self.files.runtime.lock().unwrap();
            let r = runtime
                .as_ref()
                .ok_or_else(|| invalid("files not configured"))?;
            (r.engine.clone(), r.inbox.clone())
        };
        let row = engine
            .transfers()
            .into_iter()
            .find(|r| r.direction == Direction::Incoming && r.peer == peer && r.id == id)
            .ok_or_else(|| invalid("unknown incoming transfer"))?;
        if !row.state.is_finished() {
            return Err(invalid("incoming transfer is active"));
        }
        if let Some(path) = row.saved_as {
            if path.exists() {
                let canonical = path.canonicalize().map_err(invalid)?;
                if !canonical.starts_with(&root)
                    || std::fs::symlink_metadata(&path)
                        .map_err(invalid)?
                        .file_type()
                        .is_symlink()
                {
                    return Err(invalid("unsafe private receipt path"));
                }
                std::fs::remove_file(canonical).map_err(invalid)?;
            }
        }
        engine.forget_incoming(&peer, &id).map_err(invalid)
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
                engine.attach(self.session.clone(), self.file_input.lock().unwrap().take());
            }
        }
    }
    pub async fn set_file_receiving(&self, enabled: bool) -> Result<(), CoreError> {
        if enabled
            && (!self.files_active.load(Ordering::SeqCst)
                || self.files.destination.lock().unwrap().is_none()
                || !self.trust.is_trusted(&public_key(&self.peer().public_key)?))
        {
            return Err(invalid("adopted paired receiver not ready"));
        }
        self.session
            .set_accepted(wind_core::protocol::Channel::Files, enabled)
            .await
            .map_err(session_error)?;
        Ok(())
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
