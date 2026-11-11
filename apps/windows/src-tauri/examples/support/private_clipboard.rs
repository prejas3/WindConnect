//! CCS/1 SEC:separate noninteractive station, initially empty clipboard; never the interactive station/history.
use std::io;
use windows_sys::Win32::System::{StationsAndDesktops::*, Threading::GetCurrentThreadId};
pub struct PrivateClipboard {
    previous: HWINSTA,
    old_desktop: HDESK,
    desktop: HDESK,
    station: HWINSTA,
}
fn object_name(handle: *mut std::ffi::c_void) -> io::Result<String> {
    let mut name = [0u16; 256];
    let mut needed = 0;
    if unsafe {
        GetUserObjectInformationW(handle, UOI_NAME, name.as_mut_ptr().cast(), 512, &mut needed)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(
        &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())],
    ))
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
impl PrivateClipboard {
    /// Station and desktop names, so a second test process can join this private clipboard.
    #[allow(dead_code)]
    pub fn names(&self) -> io::Result<(String, String)> {
        Ok((object_name(self.station)?, object_name(self.desktop)?))
    }
    /// Second test process: enter an existing private station by name (never WinSta0).
    #[allow(dead_code)]
    pub fn join(station: &str, desktop: &str) -> io::Result<()> {
        if station.eq_ignore_ascii_case("WinSta0") {
            return Err(io::Error::other("refusing interactive window station"));
        }
        unsafe {
            let handle = OpenWindowStationW(wide(station).as_ptr(), 0, 0x000f037f);
            if handle.is_null() || SetProcessWindowStation(handle) == 0 {
                return Err(io::Error::other(format!(
                    "join station: {}",
                    io::Error::last_os_error()
                )));
            }
            let desk = OpenDesktopW(wide(desktop).as_ptr(), 0, 0, 0x000f01ff);
            if desk.is_null() || SetThreadDesktop(desk) == 0 {
                return Err(io::Error::other(format!(
                    "join desktop: {}",
                    io::Error::last_os_error()
                )));
            }
        }
        Ok(())
    }
    #[allow(dead_code)]
    pub fn desktop(&self) -> usize {
        self.desktop as usize
    }
    pub fn enter() -> io::Result<Self> {
        unsafe {
            let previous = GetProcessWindowStation();
            let old_desktop = GetThreadDesktop(GetCurrentThreadId());
            // CCS/1 RULE:standard users may name only the logon station implicitly. Never use WinSta0 or a non-empty station.
            let station = CreateWindowStationW(std::ptr::null(), 0, 0x000f037f, std::ptr::null());
            if station.is_null() {
                return Err(io::Error::other(format!(
                    "CreateWindowStation: {}",
                    io::Error::last_os_error()
                )));
            }
            let name = object_name(station)?;
            if name.eq_ignore_ascii_case("WinSta0") || name == object_name(previous)? {
                CloseWindowStation(station);
                return Err(io::Error::other(
                    "refusing interactive/current window station",
                ));
            }
            if SetProcessWindowStation(station) == 0 {
                let error = io::Error::last_os_error();
                CloseWindowStation(station);
                return Err(io::Error::other(format!(
                    "SetProcessWindowStation: {error}"
                )));
            }
            let name = wide(&format!("WindConnectTest{}", std::process::id()));
            let desktop = CreateDesktopW(
                name.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                0x000f01ff,
                std::ptr::null(),
            );
            if desktop.is_null() || SetThreadDesktop(desktop) == 0 {
                let error = io::Error::last_os_error();
                SetProcessWindowStation(previous);
                if !desktop.is_null() {
                    CloseDesktop(desktop);
                }
                CloseWindowStation(station);
                return Err(io::Error::other(format!(
                    "Create/SetThreadDesktop: {error}"
                )));
            }
            if windows_sys::Win32::System::DataExchange::CountClipboardFormats() != 0 {
                SetThreadDesktop(old_desktop);
                SetProcessWindowStation(previous);
                CloseDesktop(desktop);
                CloseWindowStation(station);
                return Err(io::Error::other(
                    "noninteractive clipboard is not empty; test refuses to touch it",
                ));
            }
            Ok(Self {
                previous,
                old_desktop,
                desktop,
                station,
            })
        }
    }
}
impl Drop for PrivateClipboard {
    fn drop(&mut self) {
        unsafe {
            SetThreadDesktop(self.old_desktop);
            SetProcessWindowStation(self.previous);
            CloseDesktop(self.desktop);
            CloseWindowStation(self.station);
        }
    }
}
