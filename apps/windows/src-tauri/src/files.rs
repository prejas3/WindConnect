//! CCS/1 M:sending files from Windows (WN-39) on the WN-38 engine
//! FLOW:Explorer menu | drop on panel/main | "Wyślij" picker -> queue(paths) -> FileEngine::send(phone, path) -> attach(session)
//! RULE:the phone is the connected one, else the last paired; transfers wait in the engine's queue until it connects.
//! RULE:this PC routes no files/1 yet (receiving is WN-40), so attach takes no incoming queue.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};
use tauri::{Emitter, Manager};
use wind_core::{
    files::{
        id_hex, Direction, FailReason, FileEngine, FilesConfig, FilesError, TransferInfo,
        TransferState,
    },
    protocol::ErrorCode,
    session::Session,
};

/// Rows the panel gets (newest first).
pub const MAX_ROWS: usize = 50;

#[derive(Clone)]
struct Target {
    fingerprint: [u8; 32],
    name: Option<String>,
}

#[derive(Default)]
pub struct Runtime {
    engine: OnceLock<FileEngine>,
    target: Mutex<Option<Target>>,
    /// Paths that came before the engine or a paired phone.
    pending: Mutex<Vec<PathBuf>>,
    /// A drag is over the panel: focus loss must not hide it.
    pub dragging: AtomicBool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    id: String,
    name: String,
    size: u64,
    done: u64,
    state: &'static str,
    reason: Option<&'static str>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    phone: Option<String>,
    rows: Vec<Row>,
    /// Over unfinished transfers.
    active: usize,
    active_size: u64,
    active_done: u64,
}

#[derive(Clone, serde::Serialize)]
pub struct Notice {
    message: String,
    error: bool,
}

fn state_name(state: &TransferState) -> (&'static str, Option<&'static str>) {
    match state {
        TransferState::Queued => ("queued", None),
        TransferState::Running => ("running", None),
        TransferState::Waiting => ("waiting", Some("Czeka na połączenie z telefonem")),
        TransferState::Done => ("done", None),
        TransferState::Cancelled { by_peer } => (
            "cancelled",
            Some(if *by_peer {
                "Anulowano na telefonie"
            } else {
                "Anulowano"
            }),
        ),
        TransferState::Failed(reason) => (
            "failed",
            Some(match reason {
                FailReason::NoSpace => "Brak miejsca na telefonie",
                FailReason::SourceChanged => "Plik zmienił się podczas wysyłania",
                FailReason::Refused(ErrorCode::CapabilityMissing) | FailReason::Withdrawn => {
                    "Telefon nie przyjmuje teraz plików"
                }
                FailReason::Refused(_) => "Telefon odrzucił plik",
                FailReason::Protocol(_) => "Błąd połączenia",
                FailReason::Io(_) => "Nie można odczytać pliku",
            }),
        ),
    }
}

/// What the panel shows: outgoing transfers, newest first.
pub fn snapshot_of(phone: Option<String>, transfers: &[TransferInfo]) -> Snapshot {
    let outgoing: Vec<&TransferInfo> = transfers
        .iter()
        .filter(|t| t.direction == Direction::Outgoing)
        .collect();
    let active: Vec<&&TransferInfo> = outgoing.iter().filter(|t| !t.state.is_finished()).collect();
    Snapshot {
        phone,
        active: active.len(),
        active_size: active.iter().map(|t| t.size).sum(),
        active_done: active.iter().map(|t| t.done).sum(),
        rows: outgoing
            .iter()
            .rev()
            .take(MAX_ROWS)
            .map(|t| {
                let (state, reason) = state_name(&t.state);
                Row {
                    id: id_hex(&t.id),
                    name: t.path.clone(),
                    size: t.size,
                    done: t.done,
                    state,
                    reason,
                }
            })
            .collect(),
    }
}

/// Queues each path for `peer`. Returns how many files were queued and
/// what could not be (shown to the user).
pub fn send_paths(engine: &FileEngine, peer: [u8; 32], paths: &[PathBuf]) -> (usize, Vec<String>) {
    let mut seen = HashSet::new();
    let (mut queued, mut problems) = (0, Vec::new());
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        let shown = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if !path.is_absolute() {
            problems.push(format!("{shown}: nieprawidłowa ścieżka"));
            continue;
        }
        match engine.send(peer, path) {
            Ok(ids) => queued += ids.len(),
            Err(FilesError::BadPath(name, _)) => {
                problems.push(format!("{shown}: nazwy „{name}” telefon nie przyjmie"))
            }
            Err(FilesError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                problems.push(format!("{shown}: nie istnieje"))
            }
            Err(_) => problems.push(format!("{shown}: nie można odczytać")),
        }
    }
    (queued, problems)
}

fn notice(app: &tauri::AppHandle, message: String, error: bool) {
    let n = Notice { message, error };
    let _ = app.emit_to("panel", "files-notice", n.clone());
    let _ = app.emit_to("main", "files-notice", n);
}

fn downloads() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{System::Com::CoTaskMemFree, UI::Shell::*};
        let mut raw = std::ptr::null_mut();
        // SAFETY: out pointer freed with CoTaskMemFree below, success or not.
        let hr =
            unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut raw) };
        let path = (hr >= 0 && !raw.is_null()).then(|| {
            // SAFETY: a NUL-terminated wide string from the shell.
            let len = (0..).take_while(|&i| unsafe { *raw.add(i) } != 0).count();
            let slice = unsafe { std::slice::from_raw_parts(raw, len) };
            PathBuf::from(String::from_utf16_lossy(slice))
        });
        unsafe { CoTaskMemFree(raw.cast()) };
        path
    }
    #[cfg(not(windows))]
    None
}

/// Starts the engine (in the app's Tokio runtime) once the data folder is known.
pub fn init(app: &tauri::AppHandle, data_dir: &Path) -> std::io::Result<()> {
    let runtime = app.state::<Runtime>();
    let inbox = downloads()
        .unwrap_or_else(|| data_dir.join("Downloads"))
        .join("WindConnect");
    let engine = FileEngine::new(FilesConfig::new(inbox, data_dir.join("partials")))?;
    let mut updates = engine.subscribe();
    let _ = runtime.engine.set(engine);
    let handle = app.clone();
    tokio::spawn(async move {
        while updates.changed().await.is_ok() {
            let transfers = updates.borrow_and_update().clone();
            let phone = phone_name(&handle);
            let snapshot = snapshot_of(phone, &transfers);
            let _ = handle.emit_to("panel", "files-state", snapshot.clone());
            let _ = handle.emit_to("main", "files-state", snapshot);
        }
    });
    flush(app);
    Ok(())
}

fn phone_name(app: &tauri::AppHandle) -> Option<String> {
    app.state::<Runtime>()
        .target
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|t| t.name.clone())
}

/// The phone files go to: the paired one (at start), then whichever connects.
pub fn set_target(app: &tauri::AppHandle, fingerprint: [u8; 32], name: Option<String>) {
    *app.state::<Runtime>().target.lock().unwrap() = Some(Target { fingerprint, name });
    flush(app);
}

/// A phone session started: its transfers start or resume.
pub fn attach(app: &tauri::AppHandle, session: Session) {
    let peer = &session.peer();
    let name = wind_core::pairing::clean_name(&peer.hello.name);
    set_target(app, *peer.key.fingerprint().as_bytes(), name);
    if let Some(engine) = app.state::<Runtime>().engine.get() {
        engine.attach(session, None);
    }
}

fn flush(app: &tauri::AppHandle) {
    let runtime = app.state::<Runtime>();
    if runtime.engine.get().is_none() || runtime.target.lock().unwrap().is_none() {
        return;
    }
    let paths = std::mem::take(&mut *runtime.pending.lock().unwrap());
    if !paths.is_empty() {
        queue(app, paths);
    }
}

/// Sends `paths` to the phone, from any entry point.
pub fn queue(app: &tauri::AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let runtime = app.state::<Runtime>();
    let (Some(engine), Some(target)) = (
        runtime.engine.get().cloned(),
        runtime.target.lock().unwrap().clone(),
    ) else {
        let waiting = {
            let mut pending = runtime.pending.lock().unwrap();
            pending.extend(paths);
            pending.len()
        };
        let ready = runtime.engine.get().is_some();
        notice(
            app,
            if ready {
                "Najpierw sparuj telefon, a pliki zostaną wysłane.".into()
            } else {
                format!("Przygotowywanie wysyłania ({waiting})…")
            },
            ready,
        );
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let peer = target.fingerprint;
        let result = tokio::task::spawn_blocking(move || send_paths(&engine, peer, &paths)).await;
        let Ok((queued, problems)) = result else {
            return;
        };
        let to = target.name.unwrap_or_else(|| "telefon".into());
        let message = match (queued, problems.first()) {
            (0, Some(first)) => format!("Nie wysłano: {first}"),
            (n, None) => format!("Wysyłanie na {to}: {}", files_word(n)),
            (n, Some(first)) => format!(
                "Wysyłanie na {to}: {}. Pominięto {}: {first}",
                files_word(n),
                problems.len()
            ),
        };
        notice(&app, message, queued == 0);
    });
}

fn files_word(n: usize) -> String {
    let word = if n == 1 {
        "plik"
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        "pliki"
    } else {
        "plików"
    };
    format!("{n} {word}")
}

/// The system file picker (several files); folders go by the Explorer menu or a drop.
#[cfg(windows)]
fn pick() -> Vec<PathBuf> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut buffer = vec![0u16; 64 * 1024];
    let title: Vec<u16> = "Wyślij na telefon\0".encode_utf16().collect();
    // SAFETY: zeroed is a valid OPENFILENAMEW; the buffers outlive the call.
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFile = buffer.as_mut_ptr();
    ofn.nMaxFile = buffer.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return Vec::new();
    }
    parse_picked(&buffer)
}
#[cfg(not(windows))]
fn pick() -> Vec<PathBuf> {
    Vec::new()
}

/// `folder\0name\0name\0\0` for several files, `full path\0\0` for one.
pub fn parse_picked(buffer: &[u16]) -> Vec<PathBuf> {
    let parts: Vec<String> = buffer
        .split(|&c| c == 0)
        .take_while(|p| !p.is_empty())
        .map(String::from_utf16_lossy)
        .collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [one] => vec![PathBuf::from(one)],
        [folder, names @ ..] => names.iter().map(|n| Path::new(folder).join(n)).collect(),
    }
}

fn panel_or_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    if matches!(window.label(), "panel" | "main") {
        Ok(())
    } else {
        Err("unsupported window".into())
    }
}

#[tauri::command]
pub fn files_snapshot(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
) -> Result<Snapshot, String> {
    panel_or_main(&window)?;
    let phone = state
        .target
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|t| t.name.clone());
    let transfers = state
        .engine
        .get()
        .map(|e| e.transfers())
        .unwrap_or_default();
    Ok(snapshot_of(phone, &transfers))
}

fn parse_id(id: Option<&str>) -> Result<wind_core::files::TransferId, String> {
    let id = id.ok_or("missing id")?;
    if id.len() != 32 {
        return Err("bad id".into());
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).map_err(|_| "bad id")?;
    }
    Ok(out)
}

#[tauri::command]
pub fn files_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    action: &str,
    id: Option<&str>,
) -> Result<(), String> {
    panel_or_main(&window)?;
    if action == "pick" {
        tauri::async_runtime::spawn(async move {
            if let Ok(paths) = tokio::task::spawn_blocking(pick).await {
                queue(&app, paths);
            }
        });
        return Ok(());
    }
    let engine = state.engine.get().ok_or("files unavailable")?;
    match action {
        "cancel" => engine.cancel(&parse_id(id)?),
        "retry" => engine.retry(&parse_id(id)?),
        "clear" => {
            for t in engine.transfers() {
                if t.direction == Direction::Outgoing && t.state.is_finished() {
                    let _ = engine.forget(&t.id);
                }
            }
            Ok(())
        }
        _ => return Err("unsupported files action".into()),
    }
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use wind_core::{
        identity::DeviceIdentity,
        protocol::DeviceKind,
        session::{LocalInfo, Router},
        transport::{Endpoint, PinnedKeys},
    };

    #[test]
    fn picker_results() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(
            parse_picked(&w("C:\\a\\b.txt\0\0")),
            vec![PathBuf::from("C:\\a\\b.txt")]
        );
        assert_eq!(
            parse_picked(&w("C:\\a\0b.txt\0c d.jpg\0\0")),
            vec![
                PathBuf::from("C:\\a\\b.txt"),
                PathBuf::from("C:\\a\\c d.jpg")
            ]
        );
        assert!(parse_picked(&w("\0\0")).is_empty());
    }

    #[test]
    fn polish_counts() {
        assert_eq!(files_word(1), "1 plik");
        assert_eq!(files_word(3), "3 pliki");
        assert_eq!(files_word(5), "5 plików");
        assert_eq!(files_word(12), "12 plików");
        assert_eq!(files_word(22), "22 pliki");
    }

    /// Files and a folder picked on the PC arrive on a receiving peer.
    #[tokio::test(flavor = "multi_thread")]
    async fn files_and_folders_reach_the_phone() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("Album/sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("Album/1.jpg"), vec![7u8; 300_000]).unwrap();
        std::fs::write(src.join("Album/sub/2.jpg"), b"two").unwrap();

        let (ipc, iphone) = (
            DeviceIdentity::generate().unwrap(),
            DeviceIdentity::generate().unwrap(),
        );
        let (ppc, pphone) = (Arc::new(PinnedKeys::new()), Arc::new(PinnedKeys::new()));
        ppc.pin(iphone.public_key());
        pphone.pin(ipc.public_key());
        let local: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let pc = Endpoint::bind(local, &ipc, ppc).unwrap();
        let phone = Endpoint::bind(local, &iphone, pphone).unwrap();
        let addr = pc.local_addr().unwrap();
        let info = |n: &str| LocalInfo {
            name: n.into(),
            kind: DeviceKind::Computer,
            app: None,
        };
        let accepting = pc.clone();
        let server = tokio::spawn(async move {
            let c = accepting.accept().await.unwrap().unwrap();
            Session::accept(c, info("PC"), Router::new()).await.unwrap()
        });
        let mut router = Router::new();
        let incoming = router.route_files(1, 16);
        let c = phone.connect(addr, ipc.public_key()).await.unwrap();
        let (phone_session, _) = Session::connect(c, info("Pixel"), router).await.unwrap();
        let (pc_session, _) = server.await.unwrap();

        let pc_engine = FileEngine::new(FilesConfig::new(
            dir.path().join("pc-in"),
            dir.path().join("pc-parts"),
        ))
        .unwrap();
        let inbox = dir.path().join("phone-in");
        let phone_engine =
            FileEngine::new(FilesConfig::new(&inbox, dir.path().join("phone-parts"))).unwrap();
        phone_engine.attach(phone_session, Some(incoming));
        let peer = *iphone.fingerprint().as_bytes();
        let (queued, problems) = send_paths(
            &pc_engine,
            peer,
            &[
                src.join("a.txt"),
                src.join("Album"),
                src.join("a.txt"),
                src.join("missing.txt"),
                PathBuf::from("relative.txt"),
            ],
        );
        assert_eq!(queued, 3);
        assert_eq!(problems.len(), 2, "{problems:?}");
        pc_engine.attach(pc_session, None);
        let mut updates = pc_engine.subscribe();
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let all = updates.borrow_and_update().clone();
                if all.len() == 3 && all.iter().all(|t| t.state == TransferState::Done) {
                    break;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(inbox.join("a.txt")).unwrap(), b"hello");
        assert_eq!(
            std::fs::read(inbox.join("Album/sub/2.jpg")).unwrap(),
            b"two"
        );
        assert_eq!(
            std::fs::read(inbox.join("Album/1.jpg")).unwrap().len(),
            300_000
        );
        let s = snapshot_of(Some("Pixel".into()), &pc_engine.transfers());
        assert_eq!((s.active, s.rows.len()), (0, 3));
        assert!(s.rows.iter().all(|r| r.state == "done"));
    }
}
