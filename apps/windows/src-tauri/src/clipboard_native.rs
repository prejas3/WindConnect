//! CCS/1 IO:real CF_UNICODETEXT, images (the "PNG" format, CF_DIBV5/CF_DIB; WN-36) / WM_CLIPBOARDUPDATE, delayed rendering of received images | SEC:privacy formats respected, bounded copies, no payload logs.
#![cfg(windows)]
use std::{
    cell::Cell,
    io,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        mpsc, Arc, OnceLock,
    },
    thread,
    time::Duration,
};
use tokio::sync::watch;
use windows_sys::Win32::{
    Foundation::*,
    System::{
        DataExchange::*, LibraryLoader::GetModuleHandleW, Memory::*, Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::*,
};
const TEXT: u32 = 13; // CF_UNICODETEXT, not an OLE dependency.
const DIB: u32 = 8; // CF_DIB
const DIBV5: u32 = 17; // CF_DIBV5
/// Largest raw DIB copied out of the clipboard (8K × 4K × 4 B, with room).
const MAX_DIB: usize = 160 * 1024 * 1024;
/// The received image this window renders on request (WN-36).
static RENDER: std::sync::Mutex<Option<std::sync::Arc<wind_core::clipboard::IncomingImage>>> =
    std::sync::Mutex::new(None);
// CCS/1 RULE:serialize this process's clipboard opens; the window procedure never waits on an active writer (Win32 sends owner messages synchronously).
static CLIPBOARD_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
static DEFERRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Access(Option<std::sync::MutexGuard<'static, ()>>);
fn access(wait: bool) -> Option<Access> {
    use std::sync::atomic::Ordering::SeqCst;
    if wait {
        return Some(Access(Some(CLIPBOARD_GATE.lock().unwrap())));
    }
    DEFERRED.store(true, SeqCst);
    match CLIPBOARD_GATE.try_lock() {
        Ok(guard) => {
            DEFERRED.store(false, SeqCst);
            Some(Access(Some(guard)))
        }
        Err(_) => None,
    }
}
impl Drop for Access {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::SeqCst;
        self.0.take();
        if DEFERRED.swap(false, SeqCst) {
            let hwnd = WINDOW.load(SeqCst);
            if hwnd != 0 {
                unsafe {
                    PostMessageW(hwnd as HWND, WM_CLIPBOARDUPDATE, 0, 0);
                }
            }
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
struct Formats {
    exclude: u32,
    history: u32,
    cloud: u32,
    png: u32,
}
fn formats() -> io::Result<&'static Formats> {
    static IDS: OnceLock<Formats> = OnceLock::new();
    let f = IDS.get_or_init(|| unsafe {
        Formats {
            exclude: RegisterClipboardFormatW(
                wide("ExcludeClipboardContentFromMonitorProcessing").as_ptr(),
            ),
            history: RegisterClipboardFormatW(wide("CanIncludeInClipboardHistory").as_ptr()),
            cloud: RegisterClipboardFormatW(wide("CanUploadToCloudClipboard").as_ptr()),
            png: RegisterClipboardFormatW(wide("PNG").as_ptr()),
        }
    });
    if f.exclude == 0 || f.history == 0 || f.cloud == 0 || f.png == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(f)
    }
}
struct Open;
impl Open {
    fn acquire(hwnd: HWND) -> io::Result<Self> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(hwnd) } != 0 {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(3));
        }
        Err(io::Error::last_os_error())
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
fn private_flag(format: u32) -> bool {
    unsafe {
        if IsClipboardFormatAvailable(format) == 0 {
            return false;
        }
        let handle = GetClipboardData(format);
        if handle.is_null() || GlobalSize(handle) < 4 {
            return true;
        }
        let p = GlobalLock(handle);
        if p.is_null() {
            return true;
        }
        let excluded = std::ptr::read_unaligned(p.cast::<u32>()) == 0;
        GlobalUnlock(handle);
        excluded
    }
}
/// An image as the clipboard holds it, copied out (WN-36).
#[derive(Clone)]
pub enum ImagePayload {
    /// The registered "PNG" format: sent as it is.
    Png(Arc<Vec<u8>>),
    /// CF_DIBV5 / CF_DIB: encoded as PNG off the window thread.
    Dib(Arc<Vec<u8>>),
}
#[derive(Clone)]
pub struct Capture {
    /// Empty for an image.
    pub text: String,
    pub image: Option<ImagePayload>,
    pub sensitive: bool,
    pub sequence: u32,
}
/// Copies a clipboard format's bytes (at most `max`); `None` if absent or larger.
unsafe fn copy_format(format: u32, max: usize) -> io::Result<Option<Vec<u8>>> {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let size = unsafe { GlobalSize(handle) };
    if size == 0 || size > max {
        return Ok(None);
    }
    let p = unsafe { GlobalLock(handle) };
    if p.is_null() {
        return Err(io::Error::last_os_error());
    }
    let bytes = unsafe { std::slice::from_raw_parts(p.cast::<u8>(), size) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    Ok(Some(bytes))
}
pub fn read() -> io::Result<Option<Capture>> {
    read_inner(false, true)
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub fn read_test() -> io::Result<Option<Capture>> {
    read_inner(true, true)
}
fn read_inner(include_sensitive: bool, wait: bool) -> io::Result<Option<Capture>> {
    let Some(_gate) = access(wait) else {
        return Ok(None);
    };
    unsafe {
        // CCS/1 RULE:another process holding the clipboard is WouldBlock, the only error the listener retries.
        let _open = Open::acquire(std::ptr::null_mut())
            .map_err(|e| io::Error::new(io::ErrorKind::WouldBlock, e))?;
        let f = formats()?;
        let text = IsClipboardFormatAvailable(TEXT) != 0;
        let image = [f.png, DIBV5, DIB]
            .into_iter()
            .find(|format| IsClipboardFormatAvailable(*format) != 0);
        if !text && image.is_none() {
            return Ok(None);
        }
        // CCS/1 RULE:an image this window put there (a received one, delayed rendering) is never read back: no echo, no self-render wait.
        if !text && GetClipboardOwner() as usize == WINDOW.load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(None);
        }
        let sensitive = IsClipboardFormatAvailable(f.exclude) != 0
            || private_flag(f.history)
            || private_flag(f.cloud);
        // CCS/1 SEC:never even copy a password payload out of the OS when marked private.
        if sensitive && !include_sensitive {
            return Ok(Some(Capture {
                text: String::new(),
                image: None,
                sensitive: true,
                sequence: GetClipboardSequenceNumber(),
            }));
        }
        if let (false, Some(format)) = (text, image) {
            // Text wins when both are there (as before WN-36).
            type Wrap = fn(Arc<Vec<u8>>) -> ImagePayload;
            let (max, wrap): (usize, Wrap) = if format == f.png {
                (wind_core::clipboard::MAX_IMAGE as usize, ImagePayload::Png)
            } else {
                (MAX_DIB, ImagePayload::Dib)
            };
            return Ok(copy_format(format, max)?.map(|bytes| Capture {
                text: String::new(),
                image: Some(wrap(Arc::new(bytes))),
                sensitive,
                sequence: GetClipboardSequenceNumber(),
            }));
        }
        let handle = GetClipboardData(TEXT);
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let size = GlobalSize(handle);
        if !(2..=(wind_core::clipboard::MAX_TEXT + 1) * 2).contains(&size) {
            return Ok(None);
        }
        let p = GlobalLock(handle);
        if p.is_null() {
            return Err(io::Error::last_os_error());
        }
        let units = std::slice::from_raw_parts(p.cast::<u16>(), size / 2);
        let text = units
            .iter()
            .position(|c| *c == 0)
            .and_then(|end| String::from_utf16(&units[..end]).ok());
        GlobalUnlock(handle);
        Ok(text
            .filter(|s| s.len() <= wind_core::clipboard::MAX_TEXT)
            .map(|text| Capture {
                text,
                image: None,
                sensitive,
                sequence: GetClipboardSequenceNumber(),
            }))
    }
}
// CCS/1 RULE:ownership of a successful SetClipboardData allocation passes to Windows; every failed allocation is freed.
unsafe fn put(format: u32, data: &[u8]) -> io::Result<()> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len()) };
    if memory.is_null() {
        return Err(io::Error::last_os_error());
    }
    let p = unsafe { GlobalLock(memory) };
    if p.is_null() {
        unsafe { GlobalFree(memory) };
        return Err(io::Error::last_os_error());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), p.cast::<u8>(), data.len());
        GlobalUnlock(memory);
    }
    if unsafe { SetClipboardData(format, memory) }.is_null() {
        unsafe { GlobalFree(memory) };
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn write(hwnd: HWND, text: &str, protected: bool) -> io::Result<()> {
    let _gate = access(true).unwrap();
    unsafe {
        if text.len() > wind_core::clipboard::MAX_TEXT || text.contains('\0') {
            return Err(io::Error::other("invalid clipboard text"));
        }
        let f = formats()?;
        let _open = Open::acquire(hwnd)?;
        if EmptyClipboard() == 0 {
            return Err(io::Error::last_os_error());
        }
        // Metadata precedes text: failure can never leave a usable remote text without the opt-out markers.
        if protected {
            put(f.history, &0u32.to_ne_bytes())?;
            put(f.cloud, &0u32.to_ne_bytes())?;
        }
        let data: Vec<u8> = wide(text).iter().flat_map(|u| u.to_ne_bytes()).collect();
        RENDER.lock().unwrap().take();
        put(TEXT, &data)
    }
}
/// Puts a received image on the clipboard with delayed rendering (WN-36):
/// pasteable now, its "PNG" and CF_DIB rendered by this window when a paste
/// asks (WM_RENDERFORMAT), once the bytes are in.
fn write_image(
    hwnd: HWND,
    image: Arc<wind_core::clipboard::IncomingImage>,
    protected: bool,
) -> io::Result<()> {
    let _gate = access(true).unwrap();
    unsafe {
        let f = formats()?;
        let _open = Open::acquire(hwnd)?;
        if EmptyClipboard() == 0 {
            return Err(io::Error::last_os_error());
        }
        if protected {
            put(f.history, &0u32.to_ne_bytes())?;
            put(f.cloud, &0u32.to_ne_bytes())?;
        }
        *RENDER.lock().unwrap() = Some(image);
        for format in [f.png, DIB] {
            // Delayed rendering: a NULL handle. Success returns NULL too, so the error code tells.
            SetLastError(0);
            if SetClipboardData(format, std::ptr::null_mut()).is_null() && GetLastError() != 0 {
                RENDER.lock().unwrap().take();
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}
/// How long a paste waits for a received image's bytes (no progress).
const RENDER_STALL: Duration = wind_core::clipboard::STALL;
/// WM_RENDERFORMAT: the clipboard is open by the pasting app. On a failed or
/// stalled image nothing is set, so that paste fails (no partial image).
/// CCS/1 RULE:only waits on bytes the tokio side pushes; nothing on that path needs this thread.
unsafe fn render(format: u32) -> bool {
    let Some(image) = RENDER.lock().unwrap().clone() else {
        return false;
    };
    let Ok(f) = formats() else { return false };
    let Ok(png) = image.wait_complete(RENDER_STALL) else {
        return false;
    };
    let data = if format == f.png {
        png
    } else if format == DIB {
        match crate::clipboard_image::decode_png(&png) {
            Ok(rgba) => crate::clipboard_image::encode_dib(&rgba),
            Err(_) => return false,
        }
    } else {
        return false;
    };
    unsafe { put(format, &data) }.is_ok()
}
/// WM_RENDERALLFORMATS (this window is going away while it owns the
/// clipboard): render a complete image, else leave an empty clipboard rather
/// than formats nobody can render.
unsafe fn render_all(hwnd: HWND) {
    let Ok(_open) = Open::acquire(hwnd) else {
        return;
    };
    unsafe {
        if GetClipboardOwner() != hwnd {
            return;
        }
        let complete = RENDER
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|i| i.is_complete());
        let Ok(f) = formats() else { return };
        if !(complete && render(f.png) && render(DIB)) {
            EmptyClipboard();
        }
    }
}
// CCS/1 RULE:a change missed because another process held the clipboard is re-read on the window thread by one
// WM_TIMER with backoff (bounded, no queue). Only the sequence that raised it is retried; a newer change,
// disabled monitoring and window teardown cancel it.
const RETRY_TIMER: usize = 0x5752;
const RETRY_DELAYS_MS: [u32; 6] = [50, 100, 200, 400, 800, 1600];
const WM_CANCEL_RETRY: u32 = WM_APP + 1;
/// Listener counters, readable by the isolated OS proof.
#[derive(Default)]
pub struct Stats {
    pub delivered: AtomicU32,
    pub retries: AtomicU32,
    pub given_up: AtomicU32,
    pub busy: AtomicU32,
    pub pending: AtomicBool,
}
#[derive(Clone, Copy)]
struct Retry {
    sequence: u32,
    next: usize,
}
struct Watcher {
    output: watch::Sender<Option<Capture>>,
    enabled: Arc<AtomicBool>,
    stats: Arc<Stats>,
    retry: Cell<Option<Retry>>,
}
impl Watcher {
    fn enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::SeqCst)
    }
    unsafe fn cancel(&self, hwnd: HWND) {
        if self.retry.take().is_some() {
            unsafe { KillTimer(hwnd, RETRY_TIMER) };
        }
        self.stats
            .pending
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
    unsafe fn read(&self, hwnd: HWND, sequence: u32, next: usize) {
        use std::sync::atomic::Ordering::SeqCst;
        match read_inner(false, false) {
            Ok(Some(value)) => {
                self.stats.delivered.fetch_add(1, SeqCst);
                self.output.send_replace(Some(value));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.stats.busy.fetch_add(1, SeqCst);
                match RETRY_DELAYS_MS.get(next) {
                    Some(delay) if unsafe { SetTimer(hwnd, RETRY_TIMER, *delay, None) } != 0 => {
                        self.retry.set(Some(Retry {
                            sequence,
                            next: next + 1,
                        }));
                        self.stats.pending.store(true, SeqCst);
                    }
                    _ => {
                        self.stats.given_up.fetch_add(1, SeqCst);
                    }
                }
            }
            // Own operation busy (re-posted by Access), no text, or a non-transient error.
            _ => {}
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = unsafe { &*(l as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Watcher;
    match msg {
        WM_RENDERFORMAT => {
            unsafe { render(w as u32) };
            return 0;
        }
        WM_RENDERALLFORMATS => {
            unsafe { render_all(hwnd) };
            return 0;
        }
        WM_DESTROYCLIPBOARD => {
            RENDER.lock().unwrap().take();
            return 0;
        }
        _ => {}
    }
    let handled = matches!(msg, WM_CLIPBOARDUPDATE | WM_CANCEL_RETRY)
        || (msg == WM_TIMER && w == RETRY_TIMER);
    if !handled {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    if p.is_null() {
        return 0;
    }
    let watcher = unsafe { &*p };
    match msg {
        WM_CLIPBOARDUPDATE => unsafe {
            watcher.cancel(hwnd);
            if watcher.enabled() {
                watcher.read(hwnd, GetClipboardSequenceNumber(), 0);
            }
        },
        WM_TIMER => unsafe {
            let retry = watcher.retry.get();
            watcher.cancel(hwnd);
            if let Some(retry) = retry {
                if watcher.enabled() && retry.sequence == GetClipboardSequenceNumber() {
                    watcher
                        .stats
                        .retries
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    watcher.read(hwnd, retry.sequence, retry.next);
                }
            }
        },
        _ => unsafe { watcher.cancel(hwnd) },
    }
    0
}
pub struct Native {
    monitoring: Arc<AtomicBool>,
    output: watch::Sender<Option<Capture>>,
    #[cfg_attr(not(debug_assertions), allow(dead_code))] // read only by the isolated OS proof
    stats: Arc<Stats>,
    hwnd: usize,
    id: u32,
    worker: Option<thread::JoinHandle<()>>,
}
impl Native {
    pub fn start() -> io::Result<(Self, watch::Receiver<Option<Capture>>)> {
        let desktop = unsafe {
            windows_sys::Win32::System::StationsAndDesktops::GetThreadDesktop(GetCurrentThreadId())
        } as usize;
        let (output, input) = watch::channel(None);
        let to_worker = output.clone();
        let monitoring = Arc::new(AtomicBool::new(false));
        let enabled = monitoring.clone();
        let stats = Arc::new(Stats::default());
        let worker_stats = stats.clone();
        let (ready, receive) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || unsafe {
            // CCS/1 RULE:the message window belongs to the caller's desktop (also for isolated OS tests).
            if windows_sys::Win32::System::StationsAndDesktops::SetThreadDesktop(desktop as _) == 0
            {
                let _ = ready.send(Err(io::Error::last_os_error()));
                return;
            }
            let name = wide("WindConnect.Clipboard");
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                let _ = ready.send(Err(io::Error::last_os_error()));
                return;
            }
            let mut watcher = Box::new(Watcher {
                output: to_worker,
                enabled,
                stats: worker_stats,
                retry: Cell::new(None),
            });
            let hwnd = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                instance,
                (&mut *watcher as *mut Watcher).cast(),
            );
            if hwnd.is_null() {
                let _ = ready.send(Err(io::Error::last_os_error()));
                return;
            }
            if AddClipboardFormatListener(hwnd) == 0 {
                let _ = ready.send(Err(io::Error::last_os_error()));
                DestroyWindow(hwnd);
                return;
            }
            WINDOW.store(hwnd as usize, std::sync::atomic::Ordering::SeqCst);
            let _ = ready.send(Ok((hwnd as usize, GetCurrentThreadId())));
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            RemoveClipboardFormatListener(hwnd);
            watcher.cancel(hwnd);
            let _ = WINDOW.compare_exchange(
                hwnd as usize,
                0,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            );
            DestroyWindow(hwnd);
            drop(watcher);
        });
        match receive.recv().map_err(io::Error::other)? {
            Ok((hwnd, id)) => Ok((
                Self {
                    monitoring,
                    output,
                    stats,
                    hwnd,
                    id,
                    worker: Some(worker),
                },
                input,
            )),
            Err(e) => {
                let _ = worker.join();
                Err(e)
            }
        }
    }
    pub fn set_monitoring(&self, value: bool) {
        self.monitoring
            .store(value, std::sync::atomic::Ordering::SeqCst);
        if !value {
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_CANCEL_RETRY, 0, 0);
            }
            self.output.send_replace(None);
        }
    }
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn stats_test(&self) -> Arc<Stats> {
        self.stats.clone()
    }
    /// Holds this process's clipboard gate: listener reads defer until it drops (then re-posted).
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn hold_gate_test(&self) -> GateHold {
        GateHold(access(true).unwrap())
    }
    /// Keeps the clipboard open (no change) for `held`, as a slow foreign owner would; `opened` runs once it is.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn hold_open_test(&self, held: Duration, opened: impl FnOnce()) -> io::Result<()> {
        let _open = Open::acquire(self.hwnd as HWND)?;
        opened();
        thread::sleep(held);
        Ok(())
    }
    pub fn write_received(&self, text: &str) -> bool {
        write(self.hwnd as HWND, text, true).is_ok()
    }
    /// A received image, pasteable at once (delayed rendering, WN-36).
    pub fn write_received_image(&self, image: Arc<wind_core::clipboard::IncomingImage>) -> bool {
        write_image(self.hwnd as HWND, image, true).is_ok()
    }
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn write_image_test(
        &self,
        image: Arc<wind_core::clipboard::IncomingImage>,
        protected: bool,
    ) -> io::Result<()> {
        write_image(self.hwnd as HWND, image, protected)
    }
    /// Puts a plain image (CF_DIB and "PNG", rendered now) as another app would.
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn put_image_test(&self, png: &[u8], dib: &[u8]) -> io::Result<()> {
        let _gate = access(true).unwrap();
        unsafe {
            let f = formats()?;
            let _open = Open::acquire(std::ptr::null_mut())?;
            if EmptyClipboard() == 0 {
                return Err(io::Error::last_os_error());
            }
            if !png.is_empty() {
                put(f.png, png)?;
            }
            if !dib.is_empty() {
                put(DIB, dib)?;
            }
            Ok(())
        }
    }
    #[cfg(debug_assertions)]
    #[allow(dead_code)] // CCS/1 IO:external isolated OS proof uses this entry, never shipped in release.
    pub fn write_test(&self, text: &str, protected: bool) -> io::Result<()> {
        write(self.hwnd as HWND, text, protected)
    }
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn clear_test(&self) -> io::Result<()> {
        let _gate = access(true).unwrap();
        unsafe {
            let _open = Open::acquire(self.hwnd as HWND)?;
            // Ours, or ownerless (put_image_test opens without a window, as another app would).
            let owner = GetClipboardOwner();
            if owner != self.hwnd as HWND && !owner.is_null() {
                if CountClipboardFormats() == 0 {
                    return Ok(());
                }
                return Err(io::Error::other(
                    "test no longer owns clipboard; leave newer content alone",
                ));
            }
            if EmptyClipboard() == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn mark_test(&self, name: &str, value: u32) -> io::Result<()> {
        let _gate = access(true).unwrap();
        unsafe {
            let _open = Open::acquire(self.hwnd as HWND)?;
            let format = RegisterClipboardFormatW(wide(name).as_ptr());
            if format == 0 {
                return Err(io::Error::last_os_error());
            }
            put(format, &value.to_ne_bytes())
        }
    }
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
pub struct GateHold(Access);
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            PostThreadMessageW(self.id, WM_QUIT, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
