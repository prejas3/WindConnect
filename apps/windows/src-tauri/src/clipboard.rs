//! CCS/1 M:Windows shared clipboard, text and images (WN-36), history and pins (WN-37) | IO:UI commands carry no clipboard payload; the history list carries previews only. Automatic by default; settings: one bool, the image limit in MB.
use crate::clipboard_history::{Direction, History, PinError};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use wind_core::{
    clipboard::{Clipboard, Connection, IncomingImage, Sink, Status},
    session::{ChannelMessage, Session},
};

/// Image limit choices (MB, decimal), the default first in importance.
pub const IMAGE_LIMITS_MB: [u64; 3] = [5, 16, 50];
pub const DEFAULT_IMAGE_LIMIT_MB: u64 = 16;

#[derive(Clone, serde::Serialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub status: String,
    pub shortcut_error: bool,
    pub image_limit_mb: u64,
}
#[derive(Default)]
pub struct Runtime {
    shared: OnceLock<Arc<Shared>>,
    worker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    hotkey: Mutex<Option<crate::hotkey::Hotkey>>,
}
struct Shared {
    hub: Arc<Clipboard>,
    native: Arc<crate::clipboard_native::Native>,
    active: Mutex<Option<Arc<Connection>>>,
    state: Mutex<Snapshot>,
    path: PathBuf,
    app: tauri::AppHandle,
    baseline: AtomicU32,
    /// WN-37: what crossed between the devices; pins saved in `pins`.
    history: Mutex<History>,
    pins: PathBuf,
    pins_gate: Mutex<()>,
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
impl Shared {
    fn history_changed(&self) {
        let views = self.history.lock().unwrap().views();
        let _ = self.app.emit("clipboard-history", views);
    }
    /// Records an item, only while sync is on (a manual send with sync off leaves no trace).
    fn record(&self, change: impl FnOnce(&mut History) -> u64) -> Option<u64> {
        if !self.hub.enabled() {
            return None;
        }
        let id = change(&mut self.history.lock().unwrap());
        self.history_changed();
        Some(id)
    }
    /// Writes the pins as they are now (DPAPI, off the async runtime); the gate keeps saves in order.
    fn save_pins(self: &Arc<Self>) {
        let shared = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _gate = shared.pins_gate.lock().unwrap();
            let plain = shared.history.lock().unwrap().encode_pins();
            if crate::clipboard_history::save_pins(&shared.pins, &plain).is_err() {
                shared.status(crate::i18n::tr("clipboard.pins_save_failed"));
            }
        });
    }
    fn status(&self, text: &str) {
        let value = {
            let mut state = self.state.lock().unwrap();
            state.status = text.into();
            state.clone()
        };
        let _ = self.app.emit("clipboard-state", value);
    }
    async fn send(self: &Arc<Self>, capture: crate::clipboard_native::Capture, manual: bool) {
        if capture.sensitive {
            if manual {
                self.status(crate::i18n::tr("clipboard.sensitive"));
            }
            return;
        }
        let active = self.active.lock().unwrap().clone();
        let Some(active) = active else {
            self.status(status_text(Status::Unavailable));
            return;
        };
        let Some(payload) = capture.image else {
            let text = capture.text;
            let status = active.send(text.clone(), false, manual).await;
            if status == Status::Applied {
                self.record(|h| h.record_text(&text, Direction::ToPhone, now_ms()));
            }
            if manual || !matches!(status, Status::Unchanged | Status::Disabled) {
                self.status(status_text(status));
            }
            return;
        };
        if !manual && !self.hub.enabled() {
            return;
        }
        // CCS/1 RULE:image decoding/encoding runs off the window thread and the async runtime.
        let converted = tokio::task::spawn_blocking(move || match payload {
            crate::clipboard_native::ImagePayload::Png(png) => {
                crate::clipboard_image::outgoing_from_png(png.as_ref().clone())
            }
            crate::clipboard_native::ImagePayload::Dib(dib) => {
                crate::clipboard_image::outgoing_from_dib(&dib)
            }
        })
        .await;
        let image = match converted {
            Ok(Ok(image)) => image,
            Ok(Err(crate::clipboard_image::ImageError::TooBig)) => {
                self.status(crate::i18n::tr("clipboard.image_too_tall"));
                return;
            }
            _ => {
                if manual {
                    self.status(crate::i18n::tr("clipboard.image_format"));
                }
                return;
            }
        };
        let size = image.png.len() as u64;
        let kept = (
            image.thumbnail.clone(),
            Arc::new(image.png.clone()),
            image.width as u32,
            image.height as u32,
        );
        let sending = self.clone();
        let status = active
            .send_image(image, false, manual, move || {
                sending.status(crate::i18n::tr("clipboard.image_sending"))
            })
            .await;
        if status == Status::Applied {
            let (thumbnail, png, width, height) = kept;
            self.record(|h| {
                h.record_image(
                    thumbnail,
                    Some(png),
                    width,
                    height,
                    Direction::ToPhone,
                    now_ms(),
                )
            });
        }
        if manual || !matches!(status, Status::Unchanged | Status::Disabled) {
            self.status(&image_status_text(status, size, self.hub.image_limit()));
        }
    }
}
struct WindowsSink(std::sync::Weak<Shared>);
impl Sink for WindowsSink {
    fn apply(&self, text: &str) -> bool {
        let Some(shared) = self.0.upgrade() else {
            return false;
        };
        if !shared.hub.enabled() {
            return false;
        }
        let applied = shared.native.write_received(text);
        if applied {
            shared.record(|h| h.record_text(text, Direction::FromPhone, now_ms()));
            shared.status(crate::i18n::tr("clipboard.received"));
        }
        applied
    }
    fn apply_image(&self, image: Arc<IncomingImage>) -> bool {
        let Some(shared) = self.0.upgrade() else {
            return false;
        };
        if !shared.hub.enabled() {
            return false;
        }
        let applied = shared.native.write_received_image(image.clone());
        if applied {
            let recorded = shared.record(|h| {
                h.record_image(
                    image.thumbnail().to_vec(),
                    None,
                    image.width() as u32,
                    image.height() as u32,
                    Direction::FromPhone,
                    now_ms(),
                )
            });
            // The entry is copyable once the whole PNG is in; a failed transfer leaves no entry.
            if let Some(id) = recorded {
                let waiting = shared.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let shared = waiting;
                    match image.wait_complete(wind_core::clipboard::STALL) {
                        Ok(png) => shared
                            .history
                            .lock()
                            .unwrap()
                            .complete_image(id, Arc::new(png)),
                        Err(_) => {
                            shared.history.lock().unwrap().remove(id);
                        }
                    }
                    shared.history_changed();
                });
            }
            shared.status(crate::i18n::tr("clipboard.image_received"));
        }
        applied
    }
}
fn read_limit(path: &std::path::Path) -> u64 {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return DEFAULT_IMAGE_LIMIT_MB;
    };
    let mut bytes = Vec::new();
    if file.take(16).read_to_end(&mut bytes).is_err() {
        return DEFAULT_IMAGE_LIMIT_MB;
    }
    serde_json::from_slice::<u64>(&bytes)
        .ok()
        .filter(|mb| IMAGE_LIMITS_MB.contains(mb))
        .unwrap_or(DEFAULT_IMAGE_LIMIT_MB)
}
pub fn init(app: &tauri::AppHandle, directory: &std::path::Path) -> std::io::Result<()> {
    let runtime = app.state::<Runtime>();
    if runtime.shared.get().is_some() {
        return Ok(());
    }
    let path = directory.join("clipboard.json");
    let enabled = match std::fs::File::open(&path) {
        Ok(file) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(64).read_to_end(&mut bytes)?;
            serde_json::from_slice::<bool>(&bytes).unwrap_or(false)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => return Err(e),
    };
    let image_limit_mb = read_limit(&directory.join("clipboard-images.json"));
    // WN-37: pins from the last run; an unreadable file (another user's, corrupt, newer) is left as it is.
    let pins = directory.join("clipboard-pins.bin");
    let mut history = History::default();
    if let Ok(Some(plain)) = crate::clipboard_history::read_pins(&pins) {
        let _ = history.load_pins(&plain);
    }
    let (native, mut input) = crate::clipboard_native::Native::start()?;
    let hub = Arc::new(Clipboard::default());
    hub.set_enabled(enabled);
    hub.set_image_limit(image_limit_mb * 1_000_000);
    let shared = Arc::new(Shared {
        hub: hub.clone(),
        native: Arc::new(native),
        active: Mutex::new(None),
        state: Mutex::new(Snapshot {
            enabled,
            status: if enabled {
                crate::i18n::tr("clipboard.sync_auto")
            } else {
                crate::i18n::tr("clipboard.sync_off")
            }
            .into(),
            shortcut_error: false,
            image_limit_mb,
        }),
        path,
        app: app.clone(),
        baseline: AtomicU32::new(0),
        history: Mutex::new(history),
        pins,
        pins_gate: Mutex::new(()),
    });
    hub.set_sink(Arc::new(WindowsSink(Arc::downgrade(&shared))));
    let weak = Arc::downgrade(&shared);
    *runtime.hotkey.lock().unwrap() = match crate::hotkey::install_clipboard({
        let app = app.clone();
        move || send_manual(&app)
    }) {
        Ok(key) => Some(key),
        Err(_) => {
            shared.state.lock().unwrap().shortcut_error = true;
            None
        }
    };
    let ready = shared.clone();
    let _ = runtime.shared.set(shared);
    ready.status(if enabled {
        crate::i18n::tr("clipboard.sync_auto")
    } else {
        crate::i18n::tr("clipboard.sync_off")
    });
    *runtime.worker.lock().unwrap() = Some(tauri::async_runtime::spawn(async move {
        while input.changed().await.is_ok() {
            let value = input.borrow_and_update().clone();
            let Some(shared) = weak.upgrade() else { break };
            if !shared.hub.enabled() || shared.active.lock().unwrap().is_none() {
                continue;
            }
            if let Some(value) = value {
                let sequence = unsafe {
                    windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
                };
                if value.sequence == sequence
                    && value.sequence != shared.baseline.load(Ordering::SeqCst)
                {
                    shared.send(value, false).await;
                }
            }
        }
    }));
    Ok(())
}
pub fn attach(app: &tauri::AppHandle, session: Session, input: mpsc::Receiver<ChannelMessage>) {
    if let Some(shared) = app.state::<Runtime>().shared.get() {
        shared.baseline.store(
            unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() },
            Ordering::SeqCst,
        );
        let binding = shared.hub.attach(session, input);
        *shared.active.lock().unwrap() = Some(binding);
        shared.native.set_monitoring(shared.hub.enabled());
    }
}
pub fn detach(app: &tauri::AppHandle) {
    if let Some(shared) = app.state::<Runtime>().shared.get() {
        shared.native.set_monitoring(false);
        shared.active.lock().unwrap().take();
    }
}
pub fn send_manual(app: &tauri::AppHandle) {
    let shared = app.state::<Runtime>().shared.get().cloned();
    if let Some(shared) = shared {
        if shared.active.lock().unwrap().is_none() {
            shared.status(status_text(Status::Unavailable));
            return;
        }
        tauri::async_runtime::spawn(async move {
            match tokio::task::spawn_blocking(crate::clipboard_native::read).await {
                Ok(Ok(Some(value))) => shared.send(value, true).await,
                Ok(Ok(None)) => shared.status(crate::i18n::tr("clipboard.empty")),
                _ => shared.status(crate::i18n::tr("clipboard.busy")),
            }
        });
    }
}
#[tauri::command]
pub fn clipboard_snapshot(state: tauri::State<'_, Runtime>) -> Snapshot {
    state
        .shared
        .get()
        .map(|s| s.state.lock().unwrap().clone())
        .unwrap_or(Snapshot {
            enabled: false,
            status: crate::i18n::tr("clipboard.unavailable_short").into(),
            shortcut_error: true,
            image_limit_mb: DEFAULT_IMAGE_LIMIT_MB,
        })
}
/// WN-37: the history as the panel shows it (previews and thumbnails only).
#[tauri::command]
pub fn clipboard_history(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
) -> Result<Vec<crate::clipboard_history::View>, String> {
    if !matches!(window.label(), "main" | "panel") {
        return Err("unsupported window".into());
    }
    Ok(state
        .shared
        .get()
        .map(|s| s.history.lock().unwrap().views())
        .unwrap_or_default())
}
/// Puts a history entry back on the clipboard with the flags it had (from the
/// phone: kept out of Win+V history and the cloud); with sync on it then goes
/// to the phone (an equal one is `Unchanged` there).
fn copy_from_history(shared: &Arc<Shared>, id: u64) -> Result<(), String> {
    use crate::clipboard_history::Item;
    use crate::clipboard_native::{Capture, ImagePayload};
    let (content, direction) = shared
        .history
        .lock()
        .unwrap()
        .get(id)
        .ok_or("history entry unavailable")?;
    let protected = direction == Direction::FromPhone;
    let shared = shared.clone();
    tauri::async_runtime::spawn(async move {
        let native = shared.native.clone();
        // A protected text is not read back by the listener, nor our own image: both are sent here.
        let (written, capture) = match content {
            Item::Text(text) => {
                let copy = text.clone();
                let ok = tokio::task::spawn_blocking(move || {
                    native.write_history_text(&copy, protected)
                })
                .await
                .unwrap_or(false);
                (
                    ok,
                    Capture {
                        text: text.to_string(),
                        image: None,
                        sensitive: false,
                        sequence: 0,
                    },
                )
            }
            Item::Image {
                png,
                thumbnail,
                width,
                height,
            } => {
                let bytes = png.clone();
                let ok = tokio::task::spawn_blocking(move || {
                    native.write_history_image(&bytes, thumbnail, width, height, protected)
                })
                .await
                .unwrap_or(false);
                (
                    ok,
                    Capture {
                        text: String::new(),
                        image: Some(ImagePayload::Png(png)),
                        sensitive: false,
                        sequence: 0,
                    },
                )
            }
        };
        if !written {
            shared.status(crate::i18n::tr("clipboard.busy"));
            return;
        }
        shared.status(crate::i18n::tr("clipboard.copied_from_history"));
        if shared.hub.enabled() && shared.active.lock().unwrap().is_some() {
            shared.send(capture, false).await;
        }
    });
    Ok(())
}
#[allow(clippy::too_many_arguments)] // one IPC command, its optional arguments
#[tauri::command]
pub fn clipboard_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Runtime>,
    action: &str,
    enabled: Option<bool>,
    limit: Option<u64>,
    id: Option<u64>,
    pinned: Option<bool>,
) -> Result<(), String> {
    if !matches!(window.label(), "main" | "panel") {
        return Err("unsupported window".into());
    }
    let shared = state.shared.get().ok_or("clipboard unavailable")?;
    match action {
        "history-copy" => copy_from_history(shared, id.ok_or("id missing")?)?,
        "history-pin" => {
            let pinned = pinned.ok_or("pinned missing")?;
            let result = shared
                .history
                .lock()
                .unwrap()
                .pin(id.ok_or("id missing")?, pinned);
            match result {
                Ok(()) => {
                    shared.history_changed();
                    shared.save_pins();
                }
                Err(PinError::Missing) => return Err("history entry unavailable".into()),
                Err(error) => shared.status(match error {
                    PinError::Full => crate::i18n::tr("clipboard.pin_full"),
                    PinError::TooLarge => crate::i18n::tr("clipboard.pin_too_large"),
                    _ => crate::i18n::tr("clipboard.pin_arriving"),
                }),
            }
        }
        "history-delete" => {
            let was_pinned = shared
                .history
                .lock()
                .unwrap()
                .remove(id.ok_or("id missing")?)
                .ok_or("history entry unavailable")?;
            shared.history_changed();
            if was_pinned {
                shared.save_pins();
            }
        }
        "history-clear" => {
            shared.history.lock().unwrap().clear();
            shared.history_changed();
        }
        "send" => send_manual(&app),
        "enabled" => {
            let enabled = enabled.ok_or("enabled missing")?;
            let mut state = shared.state.lock().unwrap();
            let temporary = shared.path.with_extension("tmp");
            std::fs::write(
                &temporary,
                if enabled {
                    b"true".as_slice()
                } else {
                    b"false".as_slice()
                },
            )
            .map_err(|_| "cannot save clipboard setting")?;
            std::fs::rename(&temporary, &shared.path)
                .map_err(|_| "cannot save clipboard setting")?;
            shared.hub.set_enabled(enabled);
            shared
                .native
                .set_monitoring(enabled && shared.active.lock().unwrap().is_some());
            state.enabled = enabled;
            drop(state);
            // WN-37: sync off clears the unpinned history (memory); pins stay.
            if !enabled {
                shared.history.lock().unwrap().clear();
                shared.history_changed();
            }
            shared.status(if enabled {
                crate::i18n::tr("clipboard.sync_auto")
            } else {
                crate::i18n::tr("clipboard.sync_off")
            });
        }
        "image-limit" => {
            let limit = limit
                .filter(|mb| IMAGE_LIMITS_MB.contains(mb))
                .ok_or("image limit must be 5, 16 or 50 MB")?;
            let mut state = shared.state.lock().unwrap();
            let path = shared.path.with_file_name("clipboard-images.json");
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, limit.to_string())
                .map_err(|_| "cannot save clipboard setting")?;
            std::fs::rename(&temporary, &path).map_err(|_| "cannot save clipboard setting")?;
            shared.hub.set_image_limit(limit * 1_000_000);
            state.image_limit_mb = limit;
            let value = state.clone();
            drop(state);
            let _ = shared.app.emit("clipboard-state", value);
        }
        _ => return Err("unsupported clipboard action".into()),
    }
    Ok(())
}
fn status_text(status: Status) -> &'static str {
    match status {
        Status::Applied => crate::i18n::tr("clipboard.applied"),
        Status::Refused => crate::i18n::tr("clipboard.refused"),
        Status::Sensitive => crate::i18n::tr("clipboard.sensitive"),
        Status::Invalid => crate::i18n::tr("clipboard.invalid"),
        Status::Disabled => crate::i18n::tr("clipboard.sync_off"),
        Status::Unchanged => crate::i18n::tr("clipboard.unchanged"),
        Status::Busy => crate::i18n::tr("clipboard.busy"),
        Status::Failed => crate::i18n::tr("clipboard.write_failed"),
        Status::Unavailable => crate::i18n::tr("clipboard.unavailable"),
        Status::TooLarge => crate::i18n::tr("clipboard.over_setting"),
        Status::PeerLimit(_) => crate::i18n::tr("clipboard.peer_too_large"),
        Status::Unsupported => crate::i18n::tr("clipboard.images_unsupported"),
        Status::Replaced => crate::i18n::tr("clipboard.replaced"),
    }
}
fn megabytes(bytes: u64) -> String {
    let tenths = bytes.div_ceil(100_000);
    if tenths.is_multiple_of(10) {
        format!("{}", tenths / 10)
    } else {
        // The decimal separator of the UI language (WN-63).
        let separator = if crate::i18n::lang() == crate::i18n::Lang::Pl {
            ','
        } else {
            '.'
        };
        format!("{}{separator}{}", tenths / 10, tenths % 10)
    }
}
/// What the panel says after an image send.
fn image_status_text(status: Status, size: u64, limit: u64) -> String {
    match status {
        Status::Applied => crate::i18n::tr("clipboard.image_sent").into(),
        Status::TooLarge => crate::i18n::trf(
            "clipboard.image_over",
            &[("size", &megabytes(size)), ("limit", &megabytes(limit))],
        ),
        Status::PeerLimit(limit) => {
            crate::i18n::trf("clipboard.peer_limit", &[("limit", &megabytes(limit))])
        }
        Status::Failed => crate::i18n::tr("clipboard.image_failed").into(),
        Status::Invalid => crate::i18n::tr("clipboard.image_format").into(),
        other => status_text(other).into(),
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(task) = self.worker.get_mut().unwrap().take() {
            task.abort();
        }
        self.hotkey.get_mut().unwrap().take();
        if let Some(shared) = self.shared.get() {
            shared.native.set_monitoring(false);
            shared.active.lock().unwrap().take();
        }
    }
}
