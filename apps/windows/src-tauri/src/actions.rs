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
        builtin_label, is_custom_id, Action, ActionHost, Builtin, CustomAction, Executor, Policy,
        PolicySource, Status, LOCK, OPEN_URL, SLEEP,
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
        return Some(
            "Identyfikator musi zaczynać się od „x-” i mieć tylko małe litery, cyfry i „-”.",
        );
    }
    if earlier.iter().any(|e| e.id == entry.id) {
        return Some("Ten identyfikator już jest na liście.");
    }
    if entry.label.trim().is_empty() {
        return Some("Brak nazwy akcji.");
    }
    let program = Path::new(&entry.program);
    if !program.is_absolute()
        || !program
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return Some("Program musi być pełną ścieżką do pliku .exe.");
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
        use std::os::windows::ffi::OsStrExt;
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
            Action::OpenUrl(url) => {
                // Checked by wind-core: an absolute http(s) URL, so the shell hands it to the browser.
                let wide: Vec<u16> = std::ffi::OsStr::new(url)
                    .encode_wide()
                    .chain(Some(0))
                    .collect();
                let verb: Vec<u16> = "open\0".encode_utf16().collect();
                // SAFETY: NUL-terminated buffers live across the call.
                let result = unsafe {
                    windows_sys::Win32::UI::Shell::ShellExecuteW(
                        std::ptr::null_mut(),
                        verb.as_ptr(),
                        wide.as_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                    )
                };
                if (result as isize) <= 32 {
                    return Err(format!("ShellExecuteW: {}", result as isize));
                }
                Ok(())
            }
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
                LOCK => builtin_label(Builtin::Lock).to_string(),
                SLEEP => builtin_label(Builtin::Sleep).to_string(),
                OPEN_URL => builtin_label(Builtin::OpenUrl).to_string(),
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

fn snapshot_of(file: &Path, last: Option<LastAction>) -> Snapshot {
    let (doc, error) = match read(file) {
        Read::Missing => (Doc::default(), None),
        Read::Ok(doc) => (doc, None),
        Read::Broken(e) => (Doc::default(), Some(e)),
    };
    let broken = error.is_some();
    let builtin = |b: Builtin, id: &str, on: bool, detail: &str| Item {
        id: id.into(),
        label: builtin_label(b).into(),
        detail: detail.into(),
        enabled: on && !broken,
        problem: None,
    };
    Snapshot {
        builtins: vec![
            builtin(Builtin::Lock, LOCK, doc.lock, "Ekran blokady, jak Win+L"),
            builtin(Builtin::Sleep, SLEEP, doc.sleep, "Uśpienie komputera"),
            builtin(
                Builtin::OpenUrl,
                OPEN_URL,
                doc.open_url,
                "Tylko adresy http i https, w domyślnej przeglądarce",
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
        Read::Broken(_) => return Err("Plik akcji jest uszkodzony. Popraw go albo usuń.".into()),
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
