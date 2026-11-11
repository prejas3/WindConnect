//! CCS/1 M:Windows shared clipboard | IO:UI commands carry no clipboard payload. Automatic by default; settings contain one bool.
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
    clipboard::{Clipboard, Connection, Sink, Status},
    session::{ChannelMessage, Session},
};

#[derive(Clone, serde::Serialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub status: String,
    pub shortcut_error: bool,
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
    async fn send(&self, capture: crate::clipboard_native::Capture, manual: bool) {
        if capture.sensitive {
            if manual {
                self.status("Treść oznaczona jako wrażliwa — nie wysłano");
            }
            return;
        }
        let active = self.active.lock().unwrap().clone();
        let status = match active {
            Some(active) => active.send(capture.text, false, manual).await,
            None => Status::Unavailable,
        };
        if manual || !matches!(status, Status::Unchanged | Status::Disabled) {
            self.status(status_text(status));
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
    let (native, mut input) = crate::clipboard_native::Native::start()?;
    let hub = Arc::new(Clipboard::default());
    hub.set_enabled(enabled);
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
                Ok(Ok(None)) => shared.status("Brak obsługiwanego tekstu w schowku"),
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
        })
}
#[tauri::command]
pub fn clipboard_action(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, Runtime>,
    action: &str,
    enabled: Option<bool>,
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
