//! CCS/1 CHECK:finite isolated native tray/subclass proof; no singleton, pairing, shell registrations, settings or installed app access.
#[path = "../src/brand_icon.rs"]
mod brand_icon;
mod indicator {
    #[derive(Clone, Copy)]
    pub enum Phase {
        Connected,
        Searching,
        Unavailable,
    }
}
// The proof has no phone: never a low-battery badge (WN-49).
mod battery {
    pub fn current() -> Option<()> {
        None
    }
    pub fn is_low(_: Option<()>) -> bool {
        false
    }
}
mod tray {
    use tauri::Manager;
    pub fn refresh_icon(app: &tauri::AppHandle) {
        if let Some(proof) = app.try_state::<super::Proof>() {
            proof
                .tray
                .set_icon(Some(super::brand_icon::current(
                    super::indicator::Phase::Connected,
                )))
                .unwrap();
            proof
                .refreshes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::Manager;
struct Proof {
    tray: tauri::tray::TrayIcon,
    refreshes: AtomicU32,
}
fn main() {
    let output = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("TEMP report path required"),
    );
    let app = tauri::Builder::default()
        .setup(|app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .window_classname("WindConnect.IconProof")
            .icon(brand_icon::current(indicator::Phase::Connected))?
            .title("WindConnect icon proof")
            .visible(false)
            .build()?;
            let tray = tauri::tray::TrayIconBuilder::with_id("icon-proof")
                .icon(brand_icon::current(indicator::Phase::Connected))
                .tooltip("WindConnect · temporary icon proof")
                .build(app)?;
            app.manage(Proof {
                tray,
                refreshes: AtomicU32::new(0),
            });
            brand_icon::watch(&window)?;
            Ok(())
        })
        .build(tauri::generate_context!(
            "examples/icon-probe/tauri.conf.json"
        ))
        .expect("isolated proof app");
    app.run(move |app, event| {
        if matches!(event, tauri::RunEvent::Ready) {
            let proof = app.state::<Proof>();
            let mut writes = 0;
            for light in [false, true] {
                for size in [16, 20, 24, 32] {
                    for phase in [
                        indicator::Phase::Connected,
                        indicator::Phase::Searching,
                        indicator::Phase::Unavailable,
                    ] {
                        let image = brand_icon::at(phase, size, light);
                        assert_eq!(image.width(), size);
                        proof.tray.set_icon(Some(image)).expect("native set_icon");
                        writes += 1;
                    }
                }
            }
            let hwnd = app.get_webview_window("main").unwrap().hwnd().unwrap().0 as _;
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                SendMessageW(hwnd, WM_SETTINGCHANGE, 0, 0);
                SendMessageW(hwnd, WM_THEMECHANGED, 0, 0);
                SendMessageW(hwnd, WM_DISPLAYCHANGE, 0, 0);
            }
            assert!(proof.refreshes.load(Ordering::SeqCst) >= 3);
            std::fs::write(
                &output,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "nativeTrayWrites": writes,
                    "sizes": [16,20,24,32],
                    "palettes": 2,
                    "states": 3,
                    "nativeStyleRefreshes": proof.refreshes.load(Ordering::SeqCst),
                    "existingWindowSubclass": true,
                    "installedAppUntouched": true
                }))
                .unwrap(),
            )
            .unwrap();
            app.exit(0);
        }
    });
}
