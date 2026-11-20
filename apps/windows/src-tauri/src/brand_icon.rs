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
    let image = at(phase, size, light);
    // WN-49: an overlay on the WN-23 state, not a state of its own.
    if matches!(phase, Phase::Connected) && crate::battery::is_low(crate::battery::current()) {
        badged(&image, light)
    } else {
        image
    }
}
/// The low-battery badge (WN-49): an amber dot in the bottom-right corner, with a transparent ring
/// cut around it so it stays apart from the artwork at 16 px. Antialiased like the artwork.
pub fn badged(image: &Image<'_>, light: bool) -> Image<'static> {
    let (w, h) = (image.width(), image.height());
    let mut rgba = image.rgba().to_vec();
    let size = w as f32;
    let radius = (size * 0.22).max(3.0);
    let ring = radius + (size * 0.08).max(1.0);
    let (cx, cy) = (size - radius, h as f32 - radius);
    // The warning tokens of the panel: darker on a light taskbar.
    let color: [f32; 3] = if light {
        [180.0, 87.0, 15.0]
    } else {
        [240.0, 163.0, 94.0]
    };
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cut = (ring + 0.5 - d).clamp(0.0, 1.0);
            let fill = (radius + 0.5 - d).clamp(0.0, 1.0);
            if cut == 0.0 {
                continue;
            }
            let p = &mut rgba[((y * w + x) * 4) as usize..][..4];
            let below = p[3] as f32 / 255.0 * (1.0 - cut);
            let alpha = fill + below * (1.0 - fill);
            for c in 0..3 {
                let mixed = color[c] * fill + p[c] as f32 * below * (1.0 - fill);
                p[c] = if alpha > 0.0 {
                    (mixed / alpha).round() as u8
                } else {
                    0
                };
            }
            p[3] = (alpha * 255.0).round() as u8;
        }
    }
    Image::new_owned(rgba, w, h)
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
                    assert!(state[index].as_chunks::<4>().0.iter().any(|p| p[3] == 0));
                    assert!(state[index]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] > 0 && p[3] < 255));
                }
                let alpha = |state: usize| {
                    table[state][index]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| p[3])
                        .collect::<Vec<_>>()
                };
                assert_ne!(alpha(0), alpha(1), "searching differs by shape");
                assert_ne!(alpha(0), alpha(2), "offline differs by shape");
                assert_ne!(alpha(1), alpha(2));
            }
        }
    }

    #[test]
    fn low_battery_badge_marks_the_corner_only() {
        for light in [true, false] {
            for size in SIZES {
                let plain = at(Phase::Connected, size, light);
                let badged = badged(&plain, light);
                let (a, b) = (plain.rgba(), badged.rgba());
                assert_eq!(a.len(), b.len());
                let px = |x: u32, y: u32| ((y * size + x) * 4) as usize;
                // The corner is the badge: opaque amber.
                let corner = px(size - 3, size - 3);
                assert_eq!(b[corner + 3], 255, "{size} {light}");
                assert!(b[corner] > b[corner + 2], "amber: more red than blue");
                // The top-left quarter is untouched.
                for y in 0..size / 2 {
                    for x in 0..size / 2 {
                        assert_eq!(a[px(x, y)..px(x, y) + 4], b[px(x, y)..px(x, y) + 4]);
                    }
                }
                assert_ne!(a, b);
            }
        }
    }
}
