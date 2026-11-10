//! CCS/1 IO:real CF_UNICODETEXT / WM_CLIPBOARDUPDATE | SEC:privacy formats respected, bounded copies, no payload logs.
#![cfg(windows)]
use std::{
    io,
    sync::{mpsc, OnceLock},
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
        }
    });
    if f.exclude == 0 || f.history == 0 || f.cloud == 0 {
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
#[derive(Clone)]
pub struct Capture {
    pub text: String,
    pub sensitive: bool,
    pub sequence: u32,
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
        let _open = Open::acquire(std::ptr::null_mut())?;
        let f = formats()?;
        if IsClipboardFormatAvailable(TEXT) == 0 {
            return Ok(None);
        }
        let sensitive = IsClipboardFormatAvailable(f.exclude) != 0
            || private_flag(f.history)
            || private_flag(f.cloud);
        // CCS/1 SEC:never even copy a password payload out of the OS when marked private.
        if sensitive && !include_sensitive {
            return Ok(Some(Capture {
                text: String::new(),
                sensitive: true,
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
        put(TEXT, &data)
    }
}
struct Watcher {
    output: watch::Sender<Option<Capture>>,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = unsafe { &*(l as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    if msg == WM_CLIPBOARDUPDATE {
        let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Watcher;
        if !p.is_null()
            && unsafe { &*p }
                .enabled
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            if let Ok(Some(value)) = read_inner(false, false) {
                unsafe { &*p }.output.send_replace(Some(value));
            }
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
pub struct Native {
    monitoring: std::sync::Arc<std::sync::atomic::AtomicBool>,
    output: watch::Sender<Option<Capture>>,
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
        let monitoring = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let enabled = monitoring.clone();
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
            self.output.send_replace(None);
        }
    }
    pub fn write_received(&self, text: &str) -> bool {
        write(self.hwnd as HWND, text, true).is_ok()
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
            if GetClipboardOwner() != self.hwnd as HWND {
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
