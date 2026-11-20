//! CCS/1 M:WN-58 the phone's newest photos and screenshots in the panel (gallery/1, PROTOCOL.md §9.13).
//! FLOW:session attach -> watch + `Recent` (when the phone offers gallery/1) -> thumbnails, at most [`FETCHES`] at a time
//! -> `gallery-state` to the panel; `Changed` -> `Recent` again, thumbnails only for new ids. A press on a photo starts
//! fetching its original; a drag hands the file to Windows (OLE DoDragDrop, CF_HDROP through the shell), a click opens it.
//! SEC:thumbnails only in memory; originals only under %TEMP%\WindConnect\gallery, removed when the session ends, another
//! phone connects or the app starts. SEC:PC chooses image-only names, checks signatures and writes MOTW before use.
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

/// The phone changed what it accepts (its "Photos on the computer" setting or photo access).
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
            .ok_or(crate::i18n::tr("gallery.unavailable"))?;
        if !state.items.iter().any(|i| i.id == id) {
            return Err(crate::i18n::tr("gallery.not_listed").into());
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
    let message = |e: AskError| match e {
        AskError::Status(Status::NotFound) => crate::i18n::tr("gallery.gone").to_string(),
        AskError::Status(Status::Denied) => crate::i18n::tr("gallery.unavailable").to_string(),
        _ => crate::i18n::tr("gallery.fetch_failed").to_string(),
    };
    let d = gallery::original(&session, id).await.map_err(message)?;
    let size = d.info.size;
    let peer = *session.peer().key.fingerprint().as_bytes();
    let progress = |received| {
        let _ = app.emit_to(PANEL, "gallery-fetch", Fetching { id, received, size });
    };
    let saved = save(&folder().join(id.to_string()), &peer, id, d, &progress).await;
    progress(if saved.is_ok() { size } else { 0 });
    saved
}

/// Writes an original into `dir` and hands back its path, only as a picture the PC itself named and checked: the
/// name from [`gallery::saved_name`] (never the phone's own extension unless it is an image one), the first bytes
/// matching that type, and Mark-of-the-Web (Internet zone, as received files, WN-38) before anyone can open or drag
/// it. Anything else is refused and nothing is left on disk. `progress` gets the bytes so far, at most every 100 ms.
async fn save(
    dir: &std::path::Path,
    peer: &[u8; 32],
    id: u64,
    mut d: gallery::Download,
    progress: &(dyn Fn(u64) + Sync),
) -> Result<PathBuf, String> {
    use tokio::io::AsyncWriteExt;
    let failed: &str = crate::i18n::tr("gallery.fetch_failed");
    let not_a_picture: &str = crate::i18n::tr("gallery.not_image");
    // Dropping `d` here stops the phone's stream.
    let name = gallery::saved_name(id, &d.info).ok_or(not_a_picture)?;
    let path = dir.join(&name);
    let part = dir.join(format!("{name}.part"));
    let write = async {
        tokio::fs::create_dir_all(dir).await.map_err(|_| failed)?;
        let mut file = tokio::fs::File::create(&part).await.map_err(|_| failed)?;
        let mut head = Vec::with_capacity(16);
        let mut checked = false;
        let mut last = std::time::Instant::now();
        while let Some(bytes) = d.next().await.map_err(|_| failed)? {
            if !checked {
                head.extend(bytes.iter().take(16 - head.len()));
                if head.len() == 16 || d.received() == d.info.size {
                    if !gallery::bytes_match(&name, &head) {
                        return Err(not_a_picture);
                    }
                    checked = true;
                }
            }
            file.write_all(&bytes).await.map_err(|_| failed)?;
            if last.elapsed() >= std::time::Duration::from_millis(100) {
                last = std::time::Instant::now();
                progress(d.received());
            }
        }
        if !checked {
            return Err(not_a_picture);
        }
        file.flush().await.map_err(|_| failed)?;
        drop(file);
        wind_core::files::store::mark_of_the_web(&part, peer).map_err(|_| failed)?;
        tokio::fs::rename(&part, &path).await.map_err(|_| failed)?;
        Ok(path)
    };
    let result = write.await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result.map_err(String::from)
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
        Err(_) => Err(crate::i18n::tr("gallery.drag_cancelled").into()),
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
            Err(crate::i18n::tr("gallery.no_viewer").into())
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
        Err(crate::i18n::tr("gallery.drag_windows").into())
    }
    pub fn open(_: &Path) -> Result<(), String> {
        Err(crate::i18n::tr("gallery.open_windows").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // CCS/1 SEC:Exercise the production save path over QUIC, without opening untrusted files in the test runner.
    #[tokio::test]
    async fn untrusted_originals_are_images_with_motw_or_are_refused() {
        use std::io::Cursor;
        use wind_core::gallery::{FileInfo, Opened, Server, Source};
        use wind_core::identity::DeviceIdentity;
        use wind_core::protocol::DeviceKind;
        use wind_core::session::{LocalInfo, Router};
        use wind_core::transport::{Endpoint, PinnedKeys};

        struct Untrusted;
        impl Source for Untrusted {
            fn recent(&self, _: u32) -> Result<Vec<Item>, Status> {
                Ok(vec![])
            }
            fn thumb(&self, _: u64, _: u32) -> Result<Vec<u8>, Status> {
                Err(Status::NotFound)
            }
            fn original(&self, id: u64) -> Result<Opened, Status> {
                let (name, mime, bytes) = match id {
                    1 => ("evil.exe", "image/jpeg", vec![0xff, 0xd8, 0xff, 0xe0]),
                    2 => (
                        "evil.exe",
                        "application/x-msdownload",
                        b"MZpayload".to_vec(),
                    ),
                    3 => ("evil.jpg", "image/jpeg", b"MZpayload".to_vec()),
                    4 => ("evil.jpg.exe", "image/jpeg", vec![0xff, 0xd8, 0xff, 0xe0]),
                    _ => return Err(Status::NotFound),
                };
                Ok(Opened {
                    info: FileInfo {
                        name: name.into(),
                        mime: mime.into(),
                        size: bytes.len() as u64,
                    },
                    bytes: Box::new(Cursor::new(bytes)),
                })
            }
        }

        let phone_id = DeviceIdentity::generate().unwrap();
        let pc_id = DeviceIdentity::generate().unwrap();
        let phone_pins = Arc::new(PinnedKeys::new());
        let pc_pins = Arc::new(PinnedKeys::new());
        phone_pins.pin(pc_id.public_key());
        pc_pins.pin(phone_id.public_key());
        let phone_endpoint =
            Endpoint::bind("127.0.0.1:0".parse().unwrap(), &phone_id, phone_pins).unwrap();
        let pc_endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &pc_id, pc_pins).unwrap();
        let accepting = pc_endpoint.clone();
        let accept = tokio::spawn(async move { accepting.accept().await.unwrap().unwrap() });
        let connection = phone_endpoint
            .connect(pc_endpoint.local_addr().unwrap(), pc_id.public_key())
            .await
            .unwrap();
        let local = || LocalInfo {
            name: "gallery test".into(),
            kind: DeviceKind::Phone,
            app: None,
        };
        let pc_task = tokio::spawn(Session::accept(
            accept.await.unwrap(),
            local(),
            Router::new(),
        ));
        let mut router = Router::new();
        let streams = router.route_gallery(8);
        let (_phone, _) = Session::connect(connection, local(), router).await.unwrap();
        let (pc, _) = pc_task.await.unwrap().unwrap();
        let server = tokio::spawn(Server::new(Arc::new(Untrusted)).serve(streams));
        let root = tempfile::tempdir().unwrap();
        let peer = *pc.peer().key.fingerprint().as_bytes();
        for id in 1..=4 {
            let dir = root.path().join(id.to_string());
            let download = gallery::original(&pc, id).await.unwrap();
            assert_eq!(
                download.info.name,
                if id == 3 {
                    "evil.jpg"
                } else if id == 4 {
                    "evil.jpg.exe"
                } else {
                    "evil.exe"
                }
            );
            let result = save(&dir, &peer, id, download, &|_| {}).await;
            if id == 1 || id == 4 {
                let path = result.unwrap();
                assert_eq!(path.file_name().unwrap(), format!("IMG_{id}.jpg").as_str());
                assert_eq!(std::fs::read(&path).unwrap(), [0xff, 0xd8, 0xff, 0xe0]);
                #[cfg(windows)]
                {
                    let zone =
                        std::fs::read_to_string(format!("{}:Zone.Identifier", path.display()))
                            .unwrap();
                    assert!(zone.contains("ZoneId=3"));
                }
                assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
            } else {
                assert!(result.is_err());
                assert!(!dir.exists() || std::fs::read_dir(&dir).unwrap().next().is_none());
            }
        }
        server.abort();
        let _ = server.await;
    }

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
