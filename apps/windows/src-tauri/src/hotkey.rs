// CCS/1 RULE:thread owns Win+Shift+W registration; Drop stops message loop and unregisters.
#[cfg(windows)]
mod platform {
    use std::{io, sync::mpsc, thread};
    use windows_sys::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::{
            Input::KeyboardAndMouse::{
                RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
            },
            WindowsAndMessaging::{GetMessageW, PostThreadMessageW, MSG, WM_HOTKEY, WM_QUIT},
        },
    };
    pub struct Hotkey {
        id: u32,
        worker: Option<thread::JoinHandle<()>>,
    }
    impl Drop for Hotkey {
        fn drop(&mut self) {
            unsafe {
                PostThreadMessageW(self.id, WM_QUIT, 0, 0);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    pub fn install(app: tauri::AppHandle) -> io::Result<Hotkey> {
        register(MOD_WIN | MOD_SHIFT | MOD_NOREPEAT, b'W' as u32, move || {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(error) = crate::panel::show(&handle) {
                    eprintln!("panel shortcut: {error}");
                }
            });
        })
    }
    fn register(
        modifiers: u32,
        key: u32,
        callback: impl Fn() + Send + 'static,
    ) -> io::Result<Hotkey> {
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || unsafe {
            if RegisterHotKey(std::ptr::null_mut(), 1, modifiers, key) == 0 {
                let _ = tx.send(Err(io::Error::last_os_error()));
                return;
            }
            let _ = tx.send(Ok(GetCurrentThreadId()));
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                if message.message == WM_HOTKEY && message.wParam == 1 {
                    callback();
                }
            }
            UnregisterHotKey(std::ptr::null_mut(), 1);
        });
        match rx.recv().map_err(io::Error::other)? {
            Ok(id) => Ok(Hotkey {
                id,
                worker: Some(worker),
            }),
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL};
        #[test]
        fn registration_conflict_dispatch_and_cleanup() {
            let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT;
            let (tx, rx) = mpsc::channel();
            let hotkey = register(modifiers, 0x87, move || {
                let _ = tx.send(());
            })
            .unwrap();
            assert!(register(modifiers, 0x87, || {}).is_err());
            unsafe {
                assert_ne!(PostThreadMessageW(hotkey.id, WM_HOTKEY, 1, 0), 0);
            }
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
            drop(hotkey);
            drop(register(modifiers, 0x87, || {}).unwrap());
        }
    }
}
#[cfg(windows)]
pub use platform::install;
