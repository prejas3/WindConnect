//! CCS/1 M:sending files from Windows (WN-39) on the WN-38 engine
//! FLOW:Explorer menu | drop on panel/main | "Wyślij" picker -> queue(paths) -> FileEngine::send(phone, path) -> attach(session)
//! RULE:the phone is the connected one, else the last paired; transfers wait in the engine's queue until it connects.
//! RULE:incoming files/1 uses the same engine, with local type folders (WN-40).
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Mutex, OnceLock},
};
use tauri::{Emitter, Manager};
use wind_core::{
    files::{Direction, FileEngine, FilesConfig, FilesError, TransferInfo, TransferState},
    session::Session,
};

/// Rows the panel gets (newest first).
pub use crate::transfer_model::{snapshot_of, Snapshot};

#[derive(Clone)]
struct Target {
    fingerprint: [u8; 32],
    name: Option<String>,
}

#[derive(Default)]
pub struct Runtime {
    engine: OnceLock<FileEngine>,
    inbox: OnceLock<PathBuf>,
    data_dir: OnceLock<PathBuf>,
    rates: Mutex<HashMap<String, crate::transfer_progress::Rate>>,
    target: Mutex<Option<Target>>,
    /// Paths that came before the engine or a paired phone.
    pending: Mutex<Vec<PathBuf>>,
    /// A drag is over the panel: focus loss must not hide it.
    pub dragging: AtomicBool,
}

#[derive(Clone, serde::Serialize)]
pub struct Notice {
    message: String,
    error: bool,
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
    let _ = runtime.inbox.set(inbox.clone());
    let _ = runtime.data_dir.set(data_dir.to_path_buf());
    let mut config = FilesConfig::new(inbox, data_dir.join("partials"));
    config.type_folders = true;
    let engine = FileEngine::new(config)?;
    let mut updates = engine.subscribe();
    let _ = runtime.engine.set(engine);
    let handle = app.clone();
    tokio::spawn(async move {
        let mut last = None;
        while let Some(transfers) =
            crate::transfer_model::next_snapshot(&mut updates, &mut last).await
        {
            let phone = phone_name(&handle);
            let mut snapshot = snapshot_of(phone, &transfers);
            apply_rates(&handle.state::<Runtime>(), &mut snapshot);
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

fn apply_rates(runtime: &Runtime, snapshot: &mut Snapshot) {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let now = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64;
    let mut rates = runtime.rates.lock().unwrap();
    rates.retain(|key, _| snapshot.rows.iter().any(|r| r.key == *key));
    for row in &mut snapshot.rows {
        row.rate =
            rates
                .entry(row.key.clone())
                .or_default()
                .update(now, row.done, row.state == "running");
    }
}

/// The phone files go to: the paired one (at start), then whichever connects.
pub fn set_target(app: &tauri::AppHandle, fingerprint: [u8; 32], name: Option<String>) {
    *app.state::<Runtime>().target.lock().unwrap() = Some(Target { fingerprint, name });
    flush(app);
}

/// A phone session started: its transfers start or resume.
pub fn receiving_enabled(app: &tauri::AppHandle) -> bool {
    app.state::<Runtime>().engine.get().is_some()
}

pub fn attach(
    app: &tauri::AppHandle,
    session: Session,
    incoming: Option<tokio::sync::mpsc::Receiver<wind_core::session::FileStream>>,
) {
    let peer = &session.peer();
    let name = wind_core::pairing::clean_name(&peer.hello.name);
    set_target(app, *peer.key.fingerprint().as_bytes(), name);
    if let Some(engine) = app.state::<Runtime>().engine.get() {
        engine.attach(session, incoming);
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
    let mut snapshot = snapshot_of(phone, &transfers);
    apply_rates(&state, &mut snapshot);
    Ok(snapshot)
}

fn parse_id(id: Option<&str>) -> Result<wind_core::files::TransferId, String> {
    let id = id.ok_or("missing id")?;
    if id.len() != 32 || !id.is_ascii() {
        return Err("bad id".into());
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&id[i * 2..i * 2 + 2], 16).map_err(|_| "bad id")?;
    }
    Ok(out)
}

#[cfg(windows)]
fn redirects_path(tag: u32) -> bool {
    use windows_sys::Win32::System::SystemServices::{
        IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK,
    };
    matches!(tag, IO_REPARSE_TAG_SYMLINK | IO_REPARSE_TAG_MOUNT_POINT)
}

#[cfg(windows)]
fn reparse_tag(path: &Path) -> std::io::Result<u32> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    // CCS/1 RULE:metadata access only, open the reparse point itself (including directories), never hydrate/read file content.
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: live owned file handle; correctly sized writable output, held until the call returns.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(info.ReparseTag)
}

pub fn checked_open_path(row: &TransferInfo, inbox: &Path) -> Result<PathBuf, String> {
    let missing = "Plik został przeniesiony lub usunięty";
    let source = match row.direction {
        Direction::Outgoing => row.source.as_ref(),
        Direction::Incoming if row.state == TransferState::Done => row.saved_as.as_ref(),
        _ => None,
    }
    .ok_or(missing)?;
    // CCS/1 SEC:outgoing only selects in Explorer, never executes; inbound also refuses path-redirection reparse points.
    if row.direction == Direction::Incoming {
        for ancestor in source.ancestors() {
            let meta = std::fs::symlink_metadata(ancestor).map_err(|_| missing)?;
            if meta.file_type().is_symlink() {
                return Err("Nie można otworzyć dowiązania".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0
                    && redirects_path(reparse_tag(ancestor).map_err(|_| missing)?)
                {
                    return Err("Nie można otworzyć punktu ponownej analizy".into());
                }
            }
        }
    }
    if !source.is_file() {
        return Err(missing.into());
    }
    let path = source.canonicalize().map_err(|_| missing)?;
    if row.direction == Direction::Incoming
        && !path.starts_with(inbox.canonicalize().map_err(|_| missing)?)
    {
        return Err("Plik poza folderem odbioru".into());
    }
    Ok(path)
}

#[cfg(windows)]
fn select_in_explorer(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{System::Com::*, UI::Shell::*};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: own COM apartment and PIDL; opens the parent with the item selected, never executes it.
    unsafe {
        let initialized = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let item = ILCreateFromPathW(wide.as_ptr());
        let result = if item.is_null() {
            -1
        } else {
            let result = SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0);
            ILFree(item);
            result
        };
        if initialized >= 0 {
            CoUninitialize();
        }
        if result < 0 {
            Err("Nie można otworzyć folderu w Eksploratorze".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(not(windows))]
fn select_in_explorer(_: &Path) -> Result<(), String> {
    Err("Eksplorator jest dostępny na Windows".into())
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
    if action != "clear" {
        let id = parse_id(id)?;
        let rows = engine.transfers();
        let matching: Vec<_> = rows.iter().filter(|t| t.id == id).collect();
        // CCS/1 SEC:wire IDs are peer-controlled inbound; ambiguity must never act on another transfer.
        if matching.len() != 1 {
            return Err("Nie można jednoznacznie wybrać transferu".into());
        }
        let row = matching[0];
        if action == "retry" {
            if row.direction != Direction::Outgoing
                || !matches!(
                    row.state,
                    TransferState::Failed(_) | TransferState::Cancelled { .. }
                )
            {
                return Err("Nie można ponowić tego transferu".into());
            }
            let directory = state.data_dir.get().ok_or("Brak listy urządzeń")?;
            let paired = wind_core::pairing::PairedDevicesFile::in_dir(directory)
                .load()
                .map_err(|e| e.to_string())?;
            if !paired
                .devices
                .iter()
                .any(|p| p.key.fingerprint().as_bytes() == &row.peer)
            {
                return Err("Urządzenie nie jest już sparowane".into());
            }
        }
        if action == "open" {
            let path = checked_open_path(row, state.inbox.get().ok_or("Brak folderu odbioru")?)?;
            return select_in_explorer(&path);
        }
    }
    match action {
        "cancel" => engine.cancel(&parse_id(id)?),
        "retry" => engine.retry(&parse_id(id)?),
        "clear" => {
            for t in engine.transfers() {
                if t.state.is_finished()
                    && engine.transfers().iter().filter(|r| r.id == t.id).count() == 1
                {
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
        files::FailReason,
        identity::DeviceIdentity,
        protocol::DeviceKind,
        session::{LocalInfo, Router},
        transport::{Endpoint, PinnedKeys},
    };

    fn incoming(path: PathBuf) -> TransferInfo {
        TransferInfo {
            id: [1; 16],
            peer: [2; 32],
            direction: Direction::Incoming,
            path: "safe.txt".into(),
            size: 4,
            done: 4,
            state: TransferState::Done,
            source: None,
            saved_as: Some(path),
        }
    }
    #[test]
    fn folder_selection_is_verified_local_and_contained() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = temp.path().join("inbox");
        std::fs::create_dir(&inbox).unwrap();
        let file = inbox.join("safe.txt");
        std::fs::write(&file, "safe").unwrap();
        let row = incoming(file.clone());
        assert_eq!(
            checked_open_path(&row, &inbox).unwrap(),
            file.canonicalize().unwrap()
        );
        let outside = temp.path().join("outside.exe");
        std::fs::write(&outside, "not executable").unwrap();
        assert!(checked_open_path(&incoming(outside), &inbox).is_err());
        let mut unfinished = row.clone();
        unfinished.state = TransferState::Running;
        assert!(checked_open_path(&unfinished, &inbox).is_err());
        std::fs::remove_file(file).unwrap();
        assert_eq!(
            checked_open_path(&row, &inbox).unwrap_err(),
            "Plik został przeniesiony lub usunięty"
        );
        assert!(parse_id(Some("ąąąąąąąąąąąąąąąą")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn folder_selection_allows_cloud_tags_but_not_links() {
        use windows_sys::Win32::System::SystemServices::*;
        assert!(redirects_path(IO_REPARSE_TAG_SYMLINK));
        assert!(redirects_path(IO_REPARSE_TAG_MOUNT_POINT));
        for tag in [
            0,
            IO_REPARSE_TAG_CLOUD,
            IO_REPARSE_TAG_CLOUD_1,
            IO_REPARSE_TAG_CLOUD_2,
            IO_REPARSE_TAG_CLOUD_3,
            IO_REPARSE_TAG_CLOUD_4,
            IO_REPARSE_TAG_CLOUD_5,
            IO_REPARSE_TAG_CLOUD_6,
            IO_REPARSE_TAG_CLOUD_7,
            IO_REPARSE_TAG_CLOUD_8,
            IO_REPARSE_TAG_CLOUD_9,
            IO_REPARSE_TAG_CLOUD_A,
            IO_REPARSE_TAG_CLOUD_B,
            IO_REPARSE_TAG_CLOUD_C,
            IO_REPARSE_TAG_CLOUD_D,
            IO_REPARSE_TAG_CLOUD_E,
            IO_REPARSE_TAG_CLOUD_F,
            IO_REPARSE_TAG_ONEDRIVE,
        ] {
            assert!(
                !redirects_path(tag),
                "cloud metadata tag {tag:#x} does not redirect the path"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn folder_selection_refuses_inbox_junction_but_allows_outgoing() {
        let temp = tempfile::tempdir().unwrap();
        let inbox = temp.path().join("inbox");
        let target = inbox.join("real");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("safe.txt"), "safe").unwrap();
        let junction = inbox.join("junction");
        let made = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        assert!(redirects_path(reparse_tag(&junction).unwrap()));
        let source = junction.join("safe.txt");
        assert!(checked_open_path(&incoming(source.clone()), &inbox).is_err());
        let mut row = incoming(source);
        row.direction = Direction::Outgoing;
        row.source = row.saved_as.take();
        assert_eq!(
            checked_open_path(&row, &inbox).unwrap(),
            target.join("safe.txt").canonicalize().unwrap()
        );
        std::fs::remove_dir(junction).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows Developer Mode or SeCreateSymbolicLinkPrivilege"]
    fn folder_selection_refuses_inbox_symlink_but_allows_outgoing() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("safe.txt");
        std::fs::write(&target, "safe").unwrap();
        let link = temp.path().join("linked.txt");
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        assert!(redirects_path(reparse_tag(&link).unwrap()));
        assert!(checked_open_path(&incoming(link.clone()), temp.path()).is_err());
        let mut row = incoming(link);
        row.direction = Direction::Outgoing;
        row.source = row.saved_as.take();
        assert_eq!(
            checked_open_path(&row, temp.path()).unwrap(),
            target.canonicalize().unwrap()
        );
        row.source = Some(temp.path().to_path_buf());
        assert!(
            checked_open_path(&row, temp.path()).is_err(),
            "outgoing still requires a file"
        );
    }
    #[test]
    fn incoming_rows_are_not_hidden_or_offered_outgoing_retry() {
        let mut row = incoming(PathBuf::from("local/safe.txt"));
        let done = snapshot_of(None, &[row.clone()]);
        assert_eq!(done.rows.len(), 1);
        assert_eq!(done.rows[0].direction, "incoming");
        assert!(!done.rows[0].can_retry);
        assert!(done.rows[0].can_open);
        row.state = TransferState::Failed(FailReason::NoSpace);
        let failed = snapshot_of(None, &[row]);
        assert!(!failed.rows[0].can_retry);
        assert!(!failed.rows[0].can_open);
    }

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
        for transfer in pc_engine.transfers() {
            assert!(
                pc_engine.retry(&transfer.id).is_err(),
                "receiver-confirmed files must never be requeued"
            );
        }
    }
}
