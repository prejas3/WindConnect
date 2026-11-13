//! CCS/1 M:WN-58 the phone's newest photos and screenshots in the panel (gallery/1, PROTOCOL.md §9.13).
//! FLOW:session attach -> watch + `Recent` (when the phone offers gallery/1) -> thumbnails, at most [`FETCHES`] at a time
//! -> `gallery-state` to the panel; `Changed` -> `Recent` again, thumbnails only for new ids. A press on a photo starts
//! fetching its original; a drag hands the file to Windows (OLE DoDragDrop, CF_HDROP through the shell), a click opens it.
//! SEC:thumbnails only in memory; originals only under %TEMP%\WindConnect\gallery, removed when the session ends, another
//! phone connects or the app starts. Names are checked by the core (one part, nothing Windows refuses) before use.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{Emitter, Manager};
use wind_core::{
    gallery::{self, AskError, Item, Status},
    identity::PublicKey,
    session::Session,
};

/// Photos in the panel: two rows of five.
pub const SHOWN: u32 = 10;
/// Thumbnail size asked for (longer side, px): sharp in a ~64 px tile at 200% scaling, cropped square.
const THUMB_PX: u32 = 256;
/// Thumbnail requests at once (the phone serves 4 per session).
const FETCHES: usize = 3;
/// Originals kept on disk for repeated drags.
const KEEP: usize = 8;
const PANEL: &str = "panel";

/// A drag of ours is in progress: the panel must not take it as files to send, nor hide.
pub static DRAGGING_OUT: AtomicBool = AtomicBool::new(false);

type Fetch = Arc<tokio::sync::OnceCell<Result<PathBuf, String>>>;

#[derive(Default)]
pub struct Gallery(Mutex<State>);

#[derive(Default)]
struct State {
    session: Option<Session>,
    phone: Option<PublicKey>,
    items: Vec<Item>,
    /// By id: the thumbnail as a data: URL.
    thumbs: HashMap<u64, String>,
    /// Ids whose thumbnail is on its way or failed this session (not asked again).
    asked: std::collections::HashSet<u64>,
    watch: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Bumped on every attach/detach: a late refresh of an older session changes nothing.
    generation: u64,
    originals: HashMap<u64, Fetch>,
    kept: VecDeque<u64>,
    cleared: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PhotoView {
    id: u64,
    name: String,
    screenshot: bool,
    date_ms: u64,
    /// `data:image/jpeg;base64,…`, or none while it loads.
    thumb: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GalleryState {
    /// A phone session that offers gallery/1 (its setting on and photo access granted).
    available: bool,
    photos: Vec<PhotoView>,
}

#[derive(Serialize, Clone, Debug)]
struct Fetching {
    id: u64,
    received: u64,
    size: u64,
}

fn offers(session: &Session) -> bool {
    session
        .peer_capabilities()
        .iter()
        .any(|c| c.name() == "gallery" && c.version() == 1)
}

fn view(state: &State) -> GalleryState {
    let available = state.session.as_ref().is_some_and(offers);
    GalleryState {
        available,
        photos: if available {
            state
                .items
                .iter()
                .map(|i| PhotoView {
                    id: i.id,
                    name: i.name.clone(),
                    screenshot: i.screenshot,
                    date_ms: i.date_ms,
                    thumb: state.thumbs.get(&i.id).cloned(),
                })
                .collect()
        } else {
            vec![]
        },
    }
}

fn emit(app: &tauri::AppHandle) {
    let v = view(&app.state::<Gallery>().inner().0.lock().unwrap());
    let _ = app.emit_to(PANEL, "gallery-state", v);
}

/// Where originals go; created on use. A test instance keeps them in its own data folder.
fn folder() -> PathBuf {
    match std::env::var_os("WINDCONNECT_TEST_DATA_DIR") {
        Some(dir) => PathBuf::from(dir).join("gallery-temp"),
        None => std::env::temp_dir().join("WindConnect").join("gallery"),
    }
}

fn clear_folder() {
    let _ = std::fs::remove_dir_all(folder());
}

/// A phone session started. Another phone's photos are dropped first.
pub fn attach(app: &tauri::AppHandle, session: Session) {
    {
        let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
        if !state.cleared {
            // Left over from a previous run.
            clear_folder();
            state.cleared = true;
        }
        if state.phone.as_ref() != Some(&session.peer().key) {
            reset(&mut state);
            state.phone = Some(session.peer().key);
        }
        state.session = Some(session);
        state.generation += 1;
    }
    restart(app);
}

fn reset(state: &mut State) {
    if let Some(w) = state.watch.take() {
        w.abort();
    }
    let generation = state.generation + 1;
    let cleared = state.cleared;
    *state = State {
        generation,
        cleared,
        ..State::default()
    };
    clear_folder();
}

/// The session ended: its originals are removed; thumbnails stay for this phone until another one connects.
pub fn detach(app: &tauri::AppHandle) {
    {
        let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
        state.session = None;
        state.generation += 1;
        if let Some(w) = state.watch.take() {
            w.abort();
        }
        state.originals.clear();
        state.kept.clear();
        clear_folder();
    }
    emit(app);
}

/// The phone was unpaired: forget its photos.
pub fn forget(app: &tauri::AppHandle, key: &PublicKey) {
    let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
    if state.phone.as_ref() == Some(key) {
        reset(&mut state);
    }
}

/// The phone changed what it accepts (its "Zdjęcia na komputerze" setting or photo access).
pub fn capabilities_changed(app: &tauri::AppHandle) {
    restart(app);
}

fn restart(app: &tauri::AppHandle) {
    {
        let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
        if let Some(w) = state.watch.take() {
            w.abort();
        }
        if let Some(session) = state.session.clone().filter(offers) {
            let generation = state.generation;
            let app = app.clone();
            state.watch = Some(tauri::async_runtime::spawn(async move {
                // The watch first, so a photo taken during the first `Recent` isn't missed.
                let changes = gallery::watch(&session).await;
                refresh(&app, &session, generation).await;
                let Ok(mut changes) = changes else { return };
                while changes.recv().await.is_some() {
                    refresh(&app, &session, generation).await;
                }
            }));
        }
    }
    emit(app);
}

/// Asks for the newest photos, shows them at once, then fills in the missing thumbnails.
async fn refresh(app: &tauri::AppHandle, session: &Session, generation: u64) {
    let Ok(items) = gallery::recent(session, SHOWN).await else {
        return;
    };
    let wanted: Vec<u64> = {
        let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
        if state.generation != generation {
            return;
        }
        state
            .thumbs
            .retain(|id, _| items.iter().any(|i| i.id == *id));
        state.asked.retain(|id| items.iter().any(|i| i.id == *id));
        let wanted: Vec<u64> = items
            .iter()
            .map(|i| i.id)
            .filter(|id| !state.thumbs.contains_key(id) && !state.asked.contains(id))
            .collect();
        state.asked.extend(wanted.iter().copied());
        state.items = items;
        wanted
    };
    emit(app);
    // Newest first, a few at a time; each shows as soon as it comes.
    for_each_limited(wanted, FETCHES, |id| {
        let app = app.clone();
        let session = session.clone();
        async move {
            let thumb = gallery::thumb(&session, id, THUMB_PX).await;
            let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
            if state.generation != generation {
                return;
            }
            match thumb {
                Ok(jpeg) => {
                    state.thumbs.insert(
                        id,
                        format!(
                            "data:image/jpeg;base64,{}",
                            crate::clipboard_history::base64(&jpeg)
                        ),
                    );
                }
                // Gone from the phone: its tile stays blank until the next list drops it.
                Err(AskError::Status(Status::NotFound)) => {}
                // Busy, broken or slow: ask again on the next change.
                Err(_) => {
                    state.asked.remove(&id);
                }
            }
            drop(state);
            emit(&app);
        }
    })
    .await;
}

/// Runs `f` for each id, at most `limit` at a time, in order of start (a bounded `for_each_concurrent`).
async fn for_each_limited<F, Fut>(ids: Vec<u64>, limit: usize, f: F)
where
    F: Fn(u64) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let permits = Arc::new(tokio::sync::Semaphore::new(limit.max(1)));
    let mut tasks = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(permit) = permits.clone().acquire_owned().await else {
            return;
        };
        let job = f(id);
        tasks.push(tauri::async_runtime::spawn(async move {
            job.await;
            drop(permit);
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
}

/// The original of `id` on disk, fetched once per session (later callers wait for the same fetch).
async fn original(app: &tauri::AppHandle, id: u64) -> Result<PathBuf, String> {
    let (fetch, session, generation) = {
        let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
        let session = state
            .session
            .clone()
            .filter(offers)
            .ok_or("Telefon nie udostępnia teraz zdjęć.")?;
        if !state.items.iter().any(|i| i.id == id) {
            return Err("Tego zdjęcia nie ma już na liście.".into());
        }
        let fetch = state.originals.entry(id).or_default().clone();
        (fetch, session, state.generation)
    };
    let result = fetch
        .get_or_init(|| download(app.clone(), session, id))
        .await
        .clone();
    let mut state = app.state::<Gallery>().inner().0.lock().unwrap();
    match &result {
        // A failed fetch may be tried again.
        Err(_) => {
            state.originals.remove(&id);
        }
        Ok(_) if state.generation == generation && !state.kept.contains(&id) => {
            state.kept.push_back(id);
            while state.kept.len() > KEEP {
                if let Some(old) = state.kept.pop_front() {
                    state.originals.remove(&old);
                    let _ = std::fs::remove_dir_all(folder().join(old.to_string()));
                }
            }
        }
        Ok(_) => {}
    }
    result
}

async fn download(app: tauri::AppHandle, session: Session, id: u64) -> Result<PathBuf, String> {
    use tokio::io::AsyncWriteExt;
    let message = |e: AskError| match e {
        AskError::Status(Status::NotFound) => "Tego zdjęcia nie ma już na telefonie.".to_string(),
        AskError::Status(Status::Denied) => "Telefon nie udostępnia teraz zdjęć.".to_string(),
        _ => "Nie udało się pobrać zdjęcia z telefonu.".to_string(),
    };
    let mut d = gallery::original(&session, id).await.map_err(message)?;
    let dir = folder().join(id.to_string());
    let path = dir.join(&d.info.name);
    let part = dir.join(format!("{}.part", d.info.name));
    let size = d.info.size;
    let write = async {
        tokio::fs::create_dir_all(&dir).await.ok()?;
        let mut file = tokio::fs::File::create(&part).await.ok()?;
        let mut last = std::time::Instant::now();
        while let Some(bytes) = d.next().await.ok()? {
            file.write_all(&bytes).await.ok()?;
            if last.elapsed() >= std::time::Duration::from_millis(100) {
                last = std::time::Instant::now();
                let _ = app.emit_to(
                    PANEL,
                    "gallery-fetch",
                    Fetching {
                        id,
                        received: d.received(),
                        size,
                    },
                );
            }
        }
        file.flush().await.ok()?;
        drop(file);
        tokio::fs::rename(&part, &path).await.ok()?;
        Some(())
    };
    let done = write.await;
    let _ = app.emit_to(
        PANEL,
        "gallery-fetch",
        Fetching {
            id,
            received: if done.is_some() { size } else { 0 },
            size,
        },
    );
    match done {
        Some(()) => Ok(path),
        None => {
            let _ = tokio::fs::remove_file(&part).await;
            Err("Nie udało się pobrać zdjęcia z telefonu.".into())
        }
    }
}

#[tauri::command]
pub fn gallery_state(app: tauri::AppHandle) -> GalleryState {
    view(&app.state::<Gallery>().inner().0.lock().unwrap())
}

/// A press on a photo: start fetching its original, so a drag that follows has it sooner.
#[tauri::command]
pub async fn gallery_prefetch(app: tauri::AppHandle, id: u64) -> Result<(), String> {
    original(&app, id).await.map(|_| ())
}

/// A drag of a photo out of the panel: "dropped", "cancelled", or "released" when the button came up before the
/// original arrived (it is kept, so the next drag starts at once).
#[tauri::command]
pub async fn gallery_drag(app: tauri::AppHandle, id: u64) -> Result<&'static str, String> {
    let path = original(&app, id).await?;
    if !platform::button_down() {
        return Ok("released");
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        DRAGGING_OUT.store(true, Ordering::SeqCst);
        let result = platform::drag(&path);
        DRAGGING_OUT.store(false, Ordering::SeqCst);
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    match rx.await {
        Ok(Ok(true)) => Ok("dropped"),
        Ok(Ok(false)) => Ok("cancelled"),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Przeciąganie przerwane.".into()),
    }
}

/// A click (or Enter) on a photo: open the original in the default viewer.
#[tauri::command]
pub async fn gallery_open(app: tauri::AppHandle, id: u64) -> Result<(), String> {
    let path = original(&app, id).await?;
    platform::open(&path)
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::{implement, BOOL, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{
        DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK,
    };
    use windows::Win32::System::Com::IDataObject;
    use windows::Win32::System::Ole::{
        DoDragDrop, IDropSource, IDropSource_Impl, DROPEFFECT, DROPEFFECT_COPY,
    };
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// The primary button is still held (the physical right one when the buttons are swapped).
    pub fn button_down() -> bool {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};
        // SAFETY: plain queries of the input state.
        unsafe {
            let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
                VK_RBUTTON
            } else {
                VK_LBUTTON
            };
            GetAsyncKeyState(key as i32) < 0
        }
    }

    #[implement(IDropSource)]
    struct Source;

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP
            } else {
                S_OK
            }
        }
        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// Drags `path` as a file (copy only), on the UI thread (OLE is initialised there for the window's drops).
    /// `Ok(true)` when it was dropped somewhere.
    pub fn drag(path: &Path) -> Result<bool, String> {
        let name = wide(path);
        // SAFETY: COM calls on the UI thread with valid, NUL-terminated input.
        unsafe {
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None)
                .map_err(|e| e.message())?;
            let data: IDataObject = item
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|e| e.message())?;
            let source: IDropSource = Source.into();
            let mut effect = DROPEFFECT(0);
            let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
            Ok(hr == DRAGDROP_S_DROP && effect.0 != 0)
        }
    }

    pub fn open(path: &Path) -> Result<(), String> {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let file = wide(path);
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: NUL-terminated strings that outlive the call.
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if code > 32 {
            Ok(())
        } else {
            Err("Windows nie ma programu do otwarcia tego zdjęcia.".into())
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;
    pub fn button_down() -> bool {
        false
    }
    pub fn drag(_: &Path) -> Result<bool, String> {
        Err("Przeciąganie działa tylko w Windows.".into())
    }
    pub fn open(_: &Path) -> Result<(), String> {
        Err("Otwieranie działa tylko w Windows.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64) -> Item {
        Item {
            id,
            date_ms: id,
            name: format!("PXL_{id}.jpg"),
            mime: "image/jpeg".into(),
            width: 1,
            height: 1,
            size: 1,
            screenshot: false,
        }
    }

    #[test]
    fn nothing_shows_without_a_session_that_offers_photos() {
        let state = State {
            items: vec![item(1), item(2)],
            thumbs: HashMap::from([(1, "data:image/jpeg;base64,AA==".to_string())]),
            ..State::default()
        };
        assert_eq!(
            view(&state),
            GalleryState {
                available: false,
                photos: vec![]
            }
        );
    }
}
