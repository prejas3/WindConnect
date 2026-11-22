//! CCS/1 M:remote actions on this PC (WN-54, PROTOCOL.md §9.2)
//! SEC:the allowlist is actions.json beside the identity, edited only here; a phone sends an id, never a command.
//! RULE:unreadable file -> nothing runs (fail closed); missing file -> built-ins on, no custom actions.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use wind_core::{
    actions::{
        is_custom_id, Action, ActionHost, Builtin, CustomAction, Executor, Policy, PolicySource,
        Status, LOCK, OPEN_URL, SLEEP,
    },
    session::{ActionStream, Session},
    transport::PinnedKeys,
};

pub const FILE: &str = "actions.json";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Doc {
    #[serde(default = "yes")]
    lock: bool,
    #[serde(default = "yes")]
    sleep: bool,
    #[serde(default = "yes")]
    open_url: bool,
    #[serde(default)]
    custom: Vec<DocCustom>,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocCustom {
    id: String,
    label: String,
    #[serde(default = "yes")]
    enabled: bool,
    program: String,
    #[serde(default)]
    args: Vec<String>,
}
fn yes() -> bool {
    true
}
impl Default for Doc {
    fn default() -> Self {
        Doc {
            lock: true,
            sleep: true,
            open_url: true,
            custom: Vec::new(),
        }
    }
}

/// Why a custom entry is left out, or None when it can run.
fn invalid(entry: &DocCustom, earlier: &[DocCustom]) -> Option<&'static str> {
    if !is_custom_id(&entry.id) {
        return Some(crate::i18n::tr("actions.bad_id"));
    }
    if earlier.iter().any(|e| e.id == entry.id) {
        return Some(crate::i18n::tr("actions.duplicate_id"));
    }
    if entry.label.trim().is_empty() {
        return Some(crate::i18n::tr("actions.no_name"));
    }
    let program = Path::new(&entry.program);
    if !program.is_absolute()
        || !program
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Some(crate::i18n::tr("actions.bad_program"));
    }
    None
}

enum Read {
    Missing,
    Ok(Doc),
    Broken(String),
}
fn read(path: &Path) -> Read {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(e) => Read::Broken(e.to_string()),
        Ok(bytes) => match serde_json::from_slice::<Doc>(&bytes) {
            Ok(doc) => Read::Ok(doc),
            Err(e) => Read::Broken(e.to_string()),
        },
    }
}
fn policy_of(doc: &Doc) -> Policy {
    let custom = doc
        .custom
        .iter()
        .enumerate()
        .filter(|(i, c)| invalid(c, &doc.custom[..*i]).is_none())
        .map(|(_, c)| CustomAction {
            id: c.id.clone(),
            label: c.label.clone(),
            enabled: c.enabled,
            program: PathBuf::from(&c.program),
            args: c.args.clone(),
        })
        .collect();
    Policy {
        lock: doc.lock,
        sleep: doc.sleep,
        open_url: doc.open_url,
        custom,
    }
}
fn write(path: &Path, doc: &Doc) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(doc).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The allowlist in actions.json, read for every request.
pub struct FilePolicy(PathBuf);
impl PolicySource for FilePolicy {
    fn policy(&self) -> Policy {
        match read(&self.0) {
            Read::Missing => Policy::default(),
            Read::Ok(doc) => policy_of(&doc),
            Read::Broken(_) => Policy {
                lock: false,
                sleep: false,
                open_url: false,
                custom: Vec::new(),
            },
        }
    }
}

pub struct WindowsExecutor;
impl Executor for WindowsExecutor {
    fn supports(&self, _: Builtin) -> bool {
        cfg!(windows)
    }
    #[cfg(windows)]
    fn run(&self, action: &Action) -> Result<(), String> {
        match action {
            Action::Lock => {
                // SAFETY: no arguments; locks the interactive session.
                if unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() } == 0 {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                Ok(())
            }
            Action::Sleep => {
                // After the reply has left.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    // SAFETY: plain flags: sleep (not hibernate), no forced suspend, wake events on.
                    unsafe {
                        windows_sys::Win32::System::Power::SetSuspendState(false, false, false)
                    };
                });
                Ok(())
            }
            // Checked by wind-core: an absolute http(s) URL, so the shell hands it to the browser.
            Action::OpenUrl(url) => open_in_browser(url),
            Action::Custom(custom) => {
                // CCS/1 SEC:no shell: the .exe itself with the arguments set here; never cmd.exe or a script.
                let program = &custom.program;
                if !program.is_absolute()
                    || !program
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                    || !program.is_file()
                {
                    return Err("program missing or not an .exe".into());
                }
                let mut command = std::process::Command::new(program);
                command.args(&custom.args);
                if let Some(dir) = program.parent() {
                    command.current_dir(dir);
                }
                command.spawn().map(drop).map_err(|e| e.to_string())
            }
        }
    }
    #[cfg(not(windows))]
    fn run(&self, _: &Action) -> Result<(), String> {
        Err("unsupported".into())
    }
}

/// CCS/1 M:WN-173 cause (one of two): the link reached the PC but `ShellExecuteW` ran on a tokio blocking-pool thread with
/// no COM apartment and returned at once. Microsoft requires COM initialised (STA) before ShellExecute, because the
/// http(s) association (the default browser chosen in Settings) is resolved through shell COM handlers; without it the
/// call could report success with no browser started, or fail, and the phone only saw its clipboard carried over.
/// Now: an STA on this thread for the call (`with_sta`), `ShellExecuteExW` with SEE_MASK_NOASYNC (it returns only once
/// the browser was handed the link, so the apartment isn't torn down under it) and SEE_MASK_FLAG_NO_UI (no error box on
/// a tray process); any failure is the action's status, "failed", which the phone shows as "couldn't open the link".
#[cfg(windows)]
fn open_in_browser(url: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
    };
    let file: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    with_sta(|| {
        // SAFETY: a zeroed SHELLEXECUTEINFOW is valid (all null/0); the NUL-terminated buffers outlive the call.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        // SAFETY: `info` is filled as above and lives across the call.
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            return Err(format!(
                "ShellExecuteExW: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    })?
}

/// Runs `f` on this thread inside a single-threaded COM apartment (WN-173), released after; `Err` when COM can't start.
#[cfg(windows)]
fn with_sta<T: Send>(f: impl FnOnce() -> T + Send) -> Result<T, String> {
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    // SAFETY: plain flags, no reserved pointer; paired with CoUninitialize below when it succeeded (S_OK or S_FALSE).
    let hr = unsafe {
        CoInitializeEx(
            std::ptr::null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        )
    };
    if hr < 0 {
        // RPC_E_CHANGED_MODE: this thread is already MTA; ShellExecute needs an STA, so it runs on a fresh thread.
        return std::thread::scope(|s| s.spawn(|| with_sta(f)).join())
            .unwrap_or_else(|_| Err(format!("CoInitializeEx: {hr:#x}, COM thread panicked")));
    }
    let out = f();
    // SAFETY: balances the successful CoInitializeEx above on the same thread.
    unsafe { CoUninitialize() };
    Ok(out)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastAction {
    peer: String,
    label: String,
    status: &'static str,
    at_ms: u64,
}
#[derive(Default)]
pub struct Runtime {
    last: Mutex<Option<LastAction>>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    id: String,
    label: String,
    detail: String,
    enabled: bool,
    /// Why it can't run, for a custom entry left out.
    problem: Option<String>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    builtins: Vec<Item>,
    custom: Vec<Item>,
    /// The file can't be read: nothing runs until it is fixed.
    error: Option<String>,
    last: Option<LastAction>,
}

fn path() -> Result<PathBuf, String> {
    crate::pairing::data_dir()
        .map(|d| d.join(FILE))
        .map_err(|e| e.to_string())
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Done => "done",
        Status::Refused => "refused",
        Status::BadArgument => "bad-argument",
        Status::Failed => "failed",
        Status::Busy => "busy",
    }
}

/// Serves one phone's action requests for as long as its session runs.
pub fn serve(
    app: &tauri::AppHandle,
    trust: Arc<PinnedKeys>,
    session: Session,
    streams: mpsc::Receiver<ActionStream>,
) {
    let Ok(file) = path() else {
        return;
    };
    let (events, mut handled) = mpsc::unbounded_channel();
    let host = ActionHost::new(
        Arc::new(FilePolicy(file.clone())),
        Arc::new(WindowsExecutor),
        trust,
    )
    .with_events(events);
    tokio::spawn(host.serve(session, streams));
    let app = app.clone();
    tokio::spawn(async move {
        while let Some(h) = handled.recv().await {
            let label = match h.action.as_str() {
                LOCK => local_label(Builtin::Lock).to_string(),
                SLEEP => local_label(Builtin::Sleep).to_string(),
                OPEN_URL => local_label(Builtin::OpenUrl).to_string(),
                id => match read(&file) {
                    Read::Ok(doc) => doc
                        .custom
                        .iter()
                        .find(|c| c.id == id)
                        .and_then(|c| wind_core::pairing::clean_name(&c.label))
                        .unwrap_or_else(|| id.to_string()),
                    _ => id.to_string(),
                },
            };
            let last = LastAction {
                peer: h.peer,
                label,
                status: status_name(h.status),
                at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            };
            *app.state::<Runtime>().last.lock().unwrap() = Some(last.clone());
            let _ = app.emit_to("main", "actions-handled", last);
        }
    });
}

/// A built-in's name in this PC's language (WN-165); the phone gets `builtin_label`'s English fallback with the id.
fn local_label(builtin: Builtin) -> &'static str {
    crate::i18n::tr(match builtin {
        Builtin::Lock => "actions.lock",
        Builtin::Sleep => "actions.sleep",
        Builtin::OpenUrl => "actions.open_url",
    })
}

fn snapshot_of(file: &Path, last: Option<LastAction>) -> Snapshot {
    let (doc, error) = match read(file) {
        Read::Missing => (Doc::default(), None),
        Read::Ok(doc) => (doc, None),
        Read::Broken(e) => (Doc::default(), Some(e)),
    };
    let broken = error.is_some();
    let builtin = |b: Builtin, id: &str, on: bool, detail: &str| Item {
        id: id.into(),
        label: local_label(b).into(),
        detail: detail.into(),
        enabled: on && !broken,
        problem: None,
    };
    Snapshot {
        builtins: vec![
            builtin(
                Builtin::Lock,
                LOCK,
                doc.lock,
                crate::i18n::tr("actions.lock_about"),
            ),
            builtin(
                Builtin::Sleep,
                SLEEP,
                doc.sleep,
                crate::i18n::tr("actions.sleep_about"),
            ),
            builtin(
                Builtin::OpenUrl,
                OPEN_URL,
                doc.open_url,
                crate::i18n::tr("actions.open_url_about"),
            ),
        ],
        custom: doc
            .custom
            .iter()
            .enumerate()
            .map(|(i, c)| Item {
                id: c.id.clone(),
                label: wind_core::pairing::clean_name(&c.label).unwrap_or_else(|| c.id.clone()),
                detail: std::iter::once(c.program.as_str())
                    .chain(c.args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" "),
                enabled: c.enabled && !broken,
                problem: invalid(c, &doc.custom[..i]).map(String::from),
            })
            .collect(),
        error,
        last,
    }
}

fn main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("unsupported window".into())
    }
}

#[tauri::command]
pub fn actions_snapshot(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
) -> Result<Snapshot, String> {
    main_window(&window)?;
    Ok(snapshot_of(&path()?, state.last.lock().unwrap().clone()))
}

#[tauri::command]
pub fn set_action_enabled(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    id: &str,
    enabled: bool,
) -> Result<Snapshot, String> {
    main_window(&window)?;
    let file = path()?;
    let mut doc = match read(&file) {
        Read::Missing => Doc::default(),
        Read::Ok(doc) => doc,
        Read::Broken(_) => return Err(crate::i18n::tr("actions.file_broken").into()),
    };
    match id {
        LOCK => doc.lock = enabled,
        SLEEP => doc.sleep = enabled,
        OPEN_URL => doc.open_url = enabled,
        id => {
            doc.custom
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or("unknown action")?
                .enabled = enabled
        }
    }
    write(&file, &doc).map_err(|e| e.to_string())?;
    Ok(snapshot_of(&file, state.last.lock().unwrap().clone()))
}

/// Opens actions.json in Notepad (made first with the defaults).
#[tauri::command]
pub fn edit_custom_actions(window: tauri::WebviewWindow) -> Result<(), String> {
    main_window(&window)?;
    let file = path()?;
    if matches!(read(&file), Read::Missing) {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        write(&file, &Doc::default()).map_err(|e| e.to_string())?;
    }
    std::process::Command::new("notepad.exe")
        .arg(&file)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WN-173: the browser is opened from inside an STA (Microsoft's rule for ShellExecute); before, it ran on a tokio
    /// blocking thread with no apartment at all. Also from a thread already in the MTA (it moves to a fresh thread).
    #[cfg(windows)]
    #[test]
    fn opening_a_link_runs_in_a_single_threaded_apartment() {
        use windows_sys::Win32::System::Com::{
            CoGetApartmentType, CoInitializeEx, CoUninitialize, APTTYPE, APTTYPEQUALIFIER,
            APTTYPE_MAINSTA, APTTYPE_STA, COINIT_MULTITHREADED,
        };
        let sta = |kind: Result<APTTYPE, String>| matches!(kind, Ok(APTTYPE_STA | APTTYPE_MAINSTA));
        fn apartment() -> APTTYPE {
            let (mut kind, mut qualifier): (APTTYPE, APTTYPEQUALIFIER) = (-1, 0);
            // SAFETY: two out-pointers to locals.
            let hr = unsafe { CoGetApartmentType(&mut kind, &mut qualifier) };
            if hr < 0 {
                -1
            } else {
                kind
            }
        }
        let plain = std::thread::spawn(|| (apartment(), with_sta(apartment)))
            .join()
            .unwrap();
        assert!(!sta(Ok(plain.0)), "a plain thread is in no STA");
        assert!(sta(plain.1.clone()), "{:?}", plain.1);
        let mta = std::thread::spawn(|| {
            // SAFETY: plain flags; balanced below.
            unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
            let inside = with_sta(apartment);
            // SAFETY: balances the call above.
            unsafe { CoUninitialize() };
            inside
        })
        .join()
        .unwrap();
        assert!(sta(mta.clone()), "{mta:?}");
    }

    fn doc(json: &str) -> Doc {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn file_policy_fails_closed_and_skips_bad_entries() {
        let dir = std::env::temp_dir().join(format!("wc-actions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE);
        let _ = std::fs::remove_file(&file);
        assert_eq!(FilePolicy(file.clone()).policy(), Policy::default());
        std::fs::write(&file, b"{ not json").unwrap();
        let p = FilePolicy(file.clone()).policy();
        assert!(!p.lock && !p.sleep && !p.open_url && p.custom.is_empty());
        assert!(snapshot_of(&file, None).error.is_some());
        std::fs::write(
            &file,
            br#"{"lock":false,"custom":[
              {"id":"x-ok","label":"Kopia","program":"C:\\Tools\\b.exe","args":["--now"]},
              {"id":"x-ok","label":"Drugi","program":"C:\\Tools\\c.exe"},
              {"id":"backup","label":"Bez x","program":"C:\\Tools\\b.exe"},
              {"id":"x-bat","label":"Skrypt","program":"C:\\Tools\\b.bat"},
              {"id":"x-rel","label":"Wzgledna","program":"b.exe"},
              {"id":"x-cmd","label":"Powloka","program":"cmd /c calc"}
            ]}"#,
        )
        .unwrap();
        let p = FilePolicy(file.clone()).policy();
        assert!(!p.lock && p.sleep && p.open_url);
        assert_eq!(p.custom.len(), 1);
        assert_eq!(p.custom[0].program, PathBuf::from("C:\\Tools\\b.exe"));
        let s = snapshot_of(&file, None);
        assert_eq!(s.custom.iter().filter(|c| c.problem.is_some()).count(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn defaults_fill_missing_fields() {
        let d = doc(r#"{"custom":[{"id":"x-a","label":"A","program":"C:\\a.exe"}]}"#);
        assert!(d.lock && d.sleep && d.open_url && d.custom[0].enabled);
        assert!(d.custom[0].args.is_empty());
    }
}

/// WN-100: "Open on the phone" in the panel: asks the phone to open `url` (§9.2 `open-url`). Checked here first with the
/// same rule the phone applies; the answer is a status for the panel: "done", "refused", "bad-argument", "failed", "busy",
/// or "unavailable" (no phone, or it doesn't take links).
#[tauri::command]
pub async fn open_on_phone(app: tauri::AppHandle, url: String) -> Result<&'static str, String> {
    let url = url.trim().to_owned();
    if !wind_core::actions::check_url(&url) {
        return Ok("bad-argument");
    }
    let Some(session) = crate::sms::phone_session(&app) else {
        return Ok("unavailable");
    };
    match wind_core::actions::run(&session, OPEN_URL, Some(&url)).await {
        Ok(status) => Ok(status_name(status)),
        Err(wind_core::actions::RequestError::Session(
            wind_core::session::SessionError::CapabilityMissing(_),
        )) => Ok("unavailable"),
        Err(_) => Ok("failed"),
    }
}
