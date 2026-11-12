//! CCS/1 M:Windows shared clipboard, text and images (WN-36) | IO:UI commands carry no clipboard payload. Automatic by default; settings: one bool, the image limit in MB.
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
}
impl Shared {
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
                self.status("Treść oznaczona jako wrażliwa — nie wysłano");
            }
            return;
        }
        let active = self.active.lock().unwrap().clone();
        let Some(active) = active else {
            self.status(status_text(Status::Unavailable));
            return;
        };
        let Some(payload) = capture.image else {
            let status = active.send(capture.text, false, manual).await;
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
                self.status("Obraz jest za duży (ponad 16384 px)");
                return;
            }
            _ => {
                if manual {
                    self.status("Nieobsługiwany format obrazu");
                }
                return;
            }
        };
        let size = image.png.len() as u64;
        let sending = self.clone();
        let status = active
            .send_image(image, false, manual, move || {
                sending.status("Wysyłanie obrazu…")
            })
            .await;
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
            shared.status("Schowek odebrany z telefonu");
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
        let applied = shared.native.write_received_image(image);
        if applied {
            shared.status("Obraz odebrany z telefonu");
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
                "Synchronizacja automatyczna"
            } else {
                "Synchronizacja wyłączona"
            }
            .into(),
            shortcut_error: false,
            image_limit_mb,
        }),
        path,
        app: app.clone(),
        baseline: AtomicU32::new(0),
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
        "Synchronizacja automatyczna"
    } else {
        "Synchronizacja wyłączona"
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
                Ok(Ok(None)) => shared.status("Brak tekstu ani obrazu w schowku"),
                _ => shared.status("Schowek zajęty — spróbuj ponownie"),
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
            status: "Schowek niedostępny".into(),
            shortcut_error: true,
            image_limit_mb: DEFAULT_IMAGE_LIMIT_MB,
        })
}
#[tauri::command]
pub fn clipboard_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Runtime>,
    action: &str,
    enabled: Option<bool>,
    limit: Option<u64>,
) -> Result<(), String> {
    if !matches!(window.label(), "main" | "panel") {
        return Err("unsupported window".into());
    }
    let shared = state.shared.get().ok_or("clipboard unavailable")?;
    match action {
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
            shared.status(if enabled {
                "Synchronizacja automatyczna"
            } else {
                "Synchronizacja wyłączona"
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
        Status::Applied => "Schowek dostarczony do telefonu",
        Status::Refused => "Telefon odrzucił schowek lub ma nowszą zmianę",
        Status::Sensitive => "Treść oznaczona jako wrażliwa — nie wysłano",
        Status::Invalid => "Obsługiwany jest tekst do 256 KiB, bez NUL",
        Status::Disabled => "Synchronizacja wyłączona",
        Status::Unchanged => "Schowek jest już zsynchronizowany",
        Status::Busy => "Schowek zajęty — spróbuj ponownie",
        Status::Failed => "Nie udało się zapisać schowka na telefonie",
        Status::Unavailable => "Schowek niedostępny — sprawdź połączenie z telefonem",
        Status::TooLarge => "Obraz przekracza limit z ustawień",
        Status::PeerLimit(_) => "Telefon nie przyjmuje tak dużych obrazów",
        Status::Unsupported => "Telefon nie obsługuje obrazów — zaktualizuj aplikację",
        Status::Replaced => "Nowszy schowek zastąpił obraz",
    }
}
fn megabytes(bytes: u64) -> String {
    let tenths = bytes.div_ceil(100_000);
    if tenths.is_multiple_of(10) {
        format!("{}", tenths / 10)
    } else {
        format!("{},{}", tenths / 10, tenths % 10)
    }
}
/// What the panel says after an image send.
fn image_status_text(status: Status, size: u64, limit: u64) -> String {
    match status {
        Status::Applied => "Obraz wysłany na telefon".into(),
        Status::TooLarge => format!(
            "Obraz za duży ({} MB, limit {} MB)",
            megabytes(size),
            megabytes(limit)
        ),
        Status::PeerLimit(limit) => {
            format!("Telefon przyjmuje obrazy do {} MB", megabytes(limit))
        }
        Status::Failed => "Nie udało się przekazać obrazu na telefon".into(),
        Status::Invalid => "Nieobsługiwany format obrazu".into(),
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
