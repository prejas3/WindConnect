//! CCS/1 RULE:one process per logon session; a second normal launch restores the existing window.
#[cfg(windows)]
mod platform {
    use std::io;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE},
    };
    pub struct Instance(HANDLE);
    impl Drop for Instance {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn claim(activate: bool) -> io::Result<Option<Instance>> {
        let name: Vec<u16> = "Local\\io.github.prejas3.windconnect"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if exists {
            unsafe {
                CloseHandle(handle);
            }
            if activate {
                let class: Vec<u16> = "WindConnect.MainWindow"
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let window = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
                if !window.is_null() {
                    unsafe {
                        ShowWindow(window, SW_RESTORE);
                        SetForegroundWindow(window);
                    }
                }
            }
            return Ok(None);
        }
        Ok(Some(Instance(handle)))
    }
}
#[cfg(windows)]
pub use platform::claim;
#[cfg(not(windows))]
pub fn claim(_: bool) -> std::io::Result<Option<()>> {
    Ok(Some(()))
}
