//! CCS/1 SOURCE:assets/brand/orbita-small.svg -> target-size RGBA; no runtime scaling from a small bitmap.
use crate::indicator::Phase;
use tauri::image::Image;
use tauri::Manager;

const SIZES: [u32; 4] = [16, 20, 24, 32];
macro_rules! images {
    ($state:literal, $theme:literal) => {
        [
            include_bytes!(concat!("../icons/tray/", $state, "-", $theme, "-16.rgba")).as_slice(),
            include_bytes!(concat!("../icons/tray/", $state, "-", $theme, "-20.rgba")).as_slice(),
            include_bytes!(concat!("../icons/tray/", $state, "-", $theme, "-24.rgba")).as_slice(),
            include_bytes!(concat!("../icons/tray/", $state, "-", $theme, "-32.rgba")).as_slice(),
        ]
    };
}
const LIGHT: [[&[u8]; 4]; 3] = [
    images!("connected", "light"),
    images!("searching", "light"),
    images!("unavailable", "light"),
];
const DARK: [[&[u8]; 4]; 3] = [
    images!("connected", "dark"),
    images!("searching", "dark"),
    images!("unavailable", "dark"),
];

pub fn at(phase: Phase, pixels: u32, light: bool) -> Image<'static> {
    let index = SIZES.iter().position(|size| *size >= pixels).unwrap_or(3);
    let state = match phase {
        Phase::Connected => 0,
        Phase::Searching => 1,
        Phase::Unavailable => 2,
    };
    let bytes = if light { LIGHT } else { DARK }[state][index];
    Image::new(bytes, SIZES[index], SIZES[index])
}
pub fn current(phase: Phase) -> Image<'static> {
    let (size, light) = desktop();
    at(phase, size, light)
}
// CCS/1 FLOW:broadcasts reach the existing hidden main HWND; no polling, extra window or worker. WM_NCDESTROY frees the hook.
pub fn watch(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    let hwnd = window.hwnd()?.0 as _;
    let app = Box::into_raw(Box::new(window.app_handle().clone()));
    if unsafe { SetWindowSubclass(hwnd, Some(style_changed), 0x574e3933, app as usize) } == 0 {
        unsafe {
            drop(Box::from_raw(app));
        }
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
unsafe extern "system" fn style_changed(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    w: windows_sys::Win32::Foundation::WPARAM,
    l: windows_sys::Win32::Foundation::LPARAM,
    id: usize,
    data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass},
        WindowsAndMessaging::*,
    };
    if matches!(
        message,
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED
    ) {
        crate::tray::refresh_icon(unsafe { &*(data as *const tauri::AppHandle) });
    }
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(style_changed), id);
            drop(Box::from_raw(data as *mut tauri::AppHandle));
        }
    }
    unsafe { DefSubclassProc(hwnd, message, w, l) }
}
fn desktop() -> (u32, bool) {
    use windows_sys::Win32::{
        System::Registry::*,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::*,
        },
    };
    unsafe {
        let class: Vec<u16> = "Shell_TrayWnd".encode_utf16().chain(Some(0)).collect();
        let taskbar = FindWindowW(class.as_ptr(), std::ptr::null());
        let dpi = GetDpiForWindow(taskbar);
        let size =
            GetSystemMetricsForDpi(SM_CXSMICON, if dpi == 0 { 96 } else { dpi }).max(16) as u32;
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let name: Vec<u16> = "SystemUsesLightTheme"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut value = 0u32;
        let mut bytes = 4u32;
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        );
        (size, result == 0 && value != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_dpi_state_has_target_size_antialiased_distinct_artwork() {
        for table in [LIGHT, DARK] {
            for (index, size) in SIZES.into_iter().enumerate() {
                for state in table {
                    assert_eq!(state[index].len(), (size * size * 4) as usize);
                    assert!(state[index].chunks_exact(4).any(|p| p[3] == 0));
                    assert!(state[index].chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
                }
                let alpha = |state: usize| {
                    table[state][index]
                        .chunks_exact(4)
                        .map(|p| p[3])
                        .collect::<Vec<_>>()
                };
                assert_ne!(alpha(0), alpha(1), "searching differs by shape");
                assert_ne!(alpha(0), alpha(2), "offline differs by shape");
                assert_ne!(alpha(1), alpha(2));
            }
        }
    }
}
