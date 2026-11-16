//! CCS/1 M:laser overlay (WN-53) | IO:InputEvent::Laser -> topmost, click-through, per-pixel-alpha red dot that follows the cursor.
//! RULE:the window never activates or takes input, so PowerPoint and browser slideshows keep focus; the timer runs only while on.
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static ON: AtomicBool = AtomicBool::new(false);
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static STARTED: OnceLock<()> = OnceLock::new();
const WAKE: u32 = WM_APP + 53;
const TIMER: usize = 1;
/// Side of the overlay in physical pixels; the dot is centred on the cursor hot spot.
const SIDE: i32 = 48;

/// Shows or hides the laser dot. Idempotent; `false` never starts the overlay thread.
pub fn set(on: bool) {
    if ON.swap(on, Ordering::SeqCst) == on {
        return;
    }
    if on {
        STARTED.get_or_init(|| {
            let _ = std::thread::Builder::new()
                .name("laser-overlay".into())
                .spawn(run);
        });
    }
    let hwnd = WINDOW.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe { PostMessageW(hwnd as HWND, WAKE, 0, 0) };
    }
}

fn run() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = "WindConnectLaser\0".encode_utf16().collect();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            SIDE,
            SIDE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        paint(hwnd);
        WINDOW.store(hwnd as isize, Ordering::SeqCst);
        // A `set` that raced window creation is picked up here.
        PostMessageW(hwnd, WAKE, 0, 0);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WAKE => {
            if ON.load(Ordering::SeqCst) {
                follow(hwnd);
                SetTimer(hwnd, TIMER, 8, None);
            } else {
                KillTimer(hwnd, TIMER);
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_TIMER => {
            if ON.load(Ordering::SeqCst) {
                follow(hwnd);
            }
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

unsafe fn follow(hwnd: HWND) {
    let mut p = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut p) == 0 {
        return;
    }
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        p.x - SIDE / 2,
        p.y - SIDE / 2,
        0,
        0,
        SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

/// Premultiplied BGRA pixels: a bright core with a soft red glow.
pub(crate) fn pixels() -> Vec<u8> {
    let mut out = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    let c = (SIDE as f32 - 1.0) / 2.0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let core = (8.5 - d).clamp(0.0, 1.0);
            let hot = (3.5 - d).clamp(0.0, 1.0);
            let glow = if d < 22.0 {
                0.45 * (1.0 - d / 22.0).powi(2)
            } else {
                0.0
            };
            let a = core + glow * (1.0 - core);
            let r = a;
            let g = core * (0.16 + 0.6 * hot);
            let b = core * (0.16 + 0.6 * hot);
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            out.extend_from_slice(&[byte(b), byte(g), byte(r), byte(a)]);
        }
    }
    out
}

unsafe fn paint(hwnd: HWND) {
    let screen = GetDC(std::ptr::null_mut());
    let mem = CreateCompatibleDC(screen);
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = SIDE;
    info.bmiHeader.biHeight = -SIDE;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits = std::ptr::null_mut();
    let bitmap = CreateDIBSection(
        mem,
        &info,
        DIB_RGB_COLORS,
        &mut bits,
        std::ptr::null_mut(),
        0,
    );
    if !bitmap.is_null() && !bits.is_null() {
        let data = pixels();
        std::ptr::copy_nonoverlapping(data.as_ptr(), bits as *mut u8, data.len());
        let old = SelectObject(mem, bitmap);
        let size = SIZE { cx: SIDE, cy: SIDE };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            hwnd,
            screen,
            std::ptr::null(),
            &size,
            mem,
            &src,
            0,
            &blend,
            ULW_ALPHA,
        );
        SelectObject(mem, old);
        DeleteObject(bitmap);
    }
    DeleteDC(mem);
    ReleaseDC(std::ptr::null_mut(), screen);
}

#[cfg(test)]
mod tests {
    #[test]
    fn dot_is_premultiplied_centred_and_transparent_at_edges() {
        let px = super::pixels();
        let side = super::SIDE as usize;
        assert_eq!(px.len(), side * side * 4);
        for p in px.chunks(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
        }
        let at = |x: usize, y: usize| &px[(y * side + x) * 4..][..4];
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(side - 1, side / 2)[3], 0);
        assert!(at(side / 2, side / 2)[3] >= 250);
    }

    /// CCS/1 TEST:real overlay shows click-through and non-activating at the cursor, then hides; foreground is untouched.
    #[test]
    fn real_overlay_follows_cursor_without_focus_and_hides() {
        use super::*;
        let wait = |want: bool| {
            for _ in 0..200 {
                let hwnd = WINDOW.load(Ordering::SeqCst) as HWND;
                if !hwnd.is_null() && (unsafe { IsWindowVisible(hwnd) } != 0) == want {
                    return hwnd;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("overlay visible != {want}");
        };
        let before = unsafe { GetForegroundWindow() };
        set(true);
        let hwnd = wait(true);
        let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
        for flag in [
            WS_EX_LAYERED,
            WS_EX_TRANSPARENT,
            WS_EX_TOPMOST,
            WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW,
        ] {
            assert_ne!(ex & flag, 0);
        }
        let (mut p, mut r) = (POINT { x: 0, y: 0 }, unsafe {
            std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>()
        });
        std::thread::sleep(std::time::Duration::from_millis(40));
        unsafe {
            GetCursorPos(&mut p);
            GetWindowRect(hwnd, &mut r)
        };
        assert!((r.left + SIDE / 2 - p.x).abs() <= 2 && (r.top + SIDE / 2 - p.y).abs() <= 2);
        assert_eq!(unsafe { GetForegroundWindow() }, before);
        set(false);
        wait(false);
    }
}
