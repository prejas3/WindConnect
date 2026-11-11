#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::Arc;
use tauri::{Emitter, Manager};
use wind_core::network::NetworkChanges;
mod actions;
mod autostart;
mod brand_icon;
mod clipboard;
mod clipboard_native;
mod files;
mod hotkey;
pub mod indicator;
mod instance;
mod pairing;
mod panel;
mod send_pipe;
mod shell_menu;
mod tray;

#[tauri::command]
fn tray_snapshot(
    app: tauri::AppHandle,
    state: tauri::State<'_, tray::TrayRuntime>,
) -> Result<TraySnapshot, String> {
    Ok(TraySnapshot {
        state: state.applied.borrow().clone(),
        main_visible: app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false)),
        menu_visible: app
            .get_webview_window("tray-menu")
            .is_some_and(|w| w.is_visible().unwrap_or(false)),
    })
}
#[derive(serde::Serialize)]
struct TraySnapshot {
    #[serde(flatten)]
    state: tray::Applied,
    main_visible: bool,
    menu_visible: bool,
}
#[tauri::command]
fn autostart_enabled() -> Result<bool, String> {
    autostart::enabled().map_err(|e| e.to_string())
}
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    autostart::set(enabled)
        .and_then(|_| autostart::enabled())
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn tray_action(app: tauri::AppHandle, action: &str) -> Result<(), String> {
    match action {
        "menu" => {
            let window = app
                .get_webview_window("main")
                .ok_or("main window unavailable")?;
            let position = window.outer_position().map_err(|e| e.to_string())?;
            let size = window.outer_size().map_err(|e| e.to_string())?;
            tray::show_menu(
                &app,
                tauri::PhysicalPosition::new(
                    (position.x + size.width as i32) as f64,
                    (position.y + size.height as i32) as f64,
                ),
            )
            .map_err(|e| e.to_string())
        }
        "open" => tray::show_main(&app).map_err(|e| e.to_string()),
        "hide" => app
            .get_webview_window("tray-menu")
            .map(|w| w.hide())
            .transpose()
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "quit" => {
            app.exit(0);
            Ok(())
        }
        _ => Err("unsupported tray action".into()),
    }
}
#[tauri::command]
async fn dev_tray_probe(
    state: tauri::State<'_, tray::TrayRuntime>,
) -> Result<Vec<tray::Applied>, String> {
    #[cfg(debug_assertions)]
    {
        tray::probe(&state).await
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = state;
        Err("debug only".into())
    }
}

struct NetworkState {
    changes: Arc<NetworkChanges>,
    #[cfg(windows)]
    _monitor: wind_core::network::WindowsNetworkMonitor,
}

impl NetworkState {
    fn new() -> std::io::Result<Self> {
        let changes = Arc::new(NetworkChanges::default());
        #[cfg(windows)]
        // Changes only (WN-89): an initial report would restart the mDNS
        // advertisement that pairing's NetworkDiscovery has just started.
        let monitor =
            wind_core::network::WindowsNetworkMonitor::start_with(changes.clone(), false)?;
        Ok(Self {
            changes,
            #[cfg(windows)]
            _monitor: monitor,
        })
    }
}

#[tauri::command]
fn network_generation(state: tauri::State<'_, NetworkState>) -> u64 {
    state.changes.generation()
}

#[tauri::command]
fn core_version() -> &'static str {
    wind_core::VERSION
}

#[tauri::command]
fn ui_dev_mode() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
fn window_action(window: tauri::WebviewWindow, action: &str) -> Result<(), String> {
    let result = match action {
        "minimize" => window.minimize(),
        "maximize" => window.is_maximized().and_then(|maximized| {
            if maximized {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }),
        "close" => window.close(),
        "drag" => window.start_dragging(),
        _ => return Err("unsupported window action".into()),
    };
    result.map_err(|error| error.to_string())
}

#[tauri::command]
fn apply_window_theme(window: tauri::WebviewWindow, theme: &str) -> Result<bool, String> {
    let theme = match theme {
        "light" => tauri::Theme::Light,
        "dark" => tauri::Theme::Dark,
        _ => return Err("unsupported theme".into()),
    };
    window
        .set_theme(Some(theme))
        .map_err(|error| error.to_string())?;
    // CCS/1 RULE:actual DWM result controls translucent CSS; unsupported hosts stay opaque.
    #[cfg(windows)]
    return Ok(window_vibrancy::apply_mica(&window, Some(theme == tauri::Theme::Dark)).is_ok());
    #[cfg(not(windows))]
    Ok(false)
}

/// The items after `--send` (Explorer passes one; more are accepted).
fn sent_paths(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<std::path::PathBuf> {
    let mut args = args.skip_while(|a| a != "--send");
    if args.next().is_none() {
        return Vec::new();
    }
    args.map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

fn main() {
    if std::env::args().any(|arg| arg == "--remove-autostart") {
        if let Err(error) = autostart::set(false) {
            eprintln!("autostart cleanup: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--remove-shell-menu") {
        if let Err(error) = shell_menu::remove() {
            eprintln!("shell menu cleanup: {error}");
            std::process::exit(1);
        }
        return;
    }
    // CCS/1 FLOW:Explorer "Wyślij na telefon" runs `--send <item>`: a running app gets the paths over the pipe.
    let sending = sent_paths(std::env::args_os().skip(1));
    let activate = sending.is_empty() && !std::env::args().any(|arg| arg == "--autostart");
    let _instance = match instance::claim(activate) {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            if !sending.is_empty() {
                if let Err(error) = send_pipe::forward(&sending) {
                    eprintln!("send to running app: {error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Err(error) => {
            eprintln!("single instance: {error}");
            return;
        }
    };
    tauri::Builder::default()
        .manage(panel::Runtime::default())
        .manage(pairing::Runtime::default())
        .manage(actions::Runtime::default())
        .manage(files::Runtime::default())
        .manage(clipboard::Runtime::default())
        .setup(move |app| {
            if let Err(error) = shell_menu::install() {
                eprintln!("Explorer menu unavailable: {error}");
            }
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let shown = handle.clone();
                    let served = send_pipe::serve(move |paths| {
                        files::queue(&shown, paths);
                        let _ = panel::show(&shown);
                    });
                    if let Err(error) = served.await {
                        eprintln!("send pipe: {error}");
                    }
                    drop(handle);
                });
            }
            if !sending.is_empty() {
                files::queue(app.handle(), sending.clone());
            }
            app.manage(NetworkState::new()?);
            app.manage(tray::install(app.handle())?);
            if let Some(window) = app.get_webview_window("main") {
                brand_icon::watch(&window)?;
            }
            pairing::start(
                app.handle().clone(),
                app.state::<NetworkState>().changes.clone(),
            );
            #[cfg(windows)]
            match hotkey::install(app.handle().clone()) {
                Ok(hotkey) => {
                    app.manage(hotkey);
                }
                Err(error) => {
                    eprintln!("panel shortcut unavailable: {error}");
                    app.state::<panel::Runtime>()
                        .0
                        .lock()
                        .unwrap()
                        .shortcut_error = Some(error.to_string());
                }
            }
            if !std::env::args().any(|arg| arg == "--autostart") || !sending.is_empty() {
                panel::show(app.handle())?;
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::ScaleFactorChanged { .. } | tauri::WindowEvent::ThemeChanged(_) => {
                tray::refresh_icon(window.app_handle());
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(false)
                if window.label() == "tray-menu"
                    || (window.label() == "panel"
                        && !window
                            .state::<files::Runtime>()
                            .dragging
                            .load(std::sync::atomic::Ordering::Relaxed)) =>
            {
                let _ = window.hide();
            }
            // CCS/1 FLOW:files dropped on the panel or the main window go to the phone (WN-39).
            tauri::WindowEvent::DragDrop(event) if matches!(window.label(), "panel" | "main") => {
                let runtime = window.state::<files::Runtime>();
                let over = match event {
                    tauri::DragDropEvent::Enter { .. } | tauri::DragDropEvent::Over { .. } => true,
                    tauri::DragDropEvent::Drop { paths, .. } => {
                        files::queue(window.app_handle(), paths.clone());
                        false
                    }
                    _ => false,
                };
                runtime
                    .dragging
                    .store(over, std::sync::atomic::Ordering::Relaxed);
                let _ = window.emit_to(window.label(), "files-drag", over);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            core_version,
            network_generation,
            ui_dev_mode,
            window_action,
            apply_window_theme,
            tray_snapshot,
            autostart_enabled,
            set_autostart,
            tray_action,
            dev_tray_probe,
            panel::panel_ready,
            panel::panel_painted,
            panel::panel_action,
            panel::panel_snapshot,
            pairing::pairing_snapshot,
            pairing::pairing_action,
            pairing::dev_pairing_address,
            actions::actions_snapshot,
            actions::set_action_enabled,
            actions::edit_custom_actions,
            files::files_snapshot,
            files::files_action,
            clipboard::clipboard_snapshot,
            clipboard::clipboard_action
        ])
        .run(tauri::generate_context!())
        .expect("failed to run WindConnect");
}

#[cfg(test)]
mod tests {
    #[test]
    fn send_arguments() {
        let args = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        assert!(super::sent_paths(args(&["--autostart"]).into_iter()).is_empty());
        assert_eq!(
            super::sent_paths(args(&["--send", r"C:\a b\c.txt", "relative", r"D:\x"]).into_iter()),
            vec![
                std::path::PathBuf::from(r"C:\a b\c.txt"),
                std::path::PathBuf::from(r"D:\x")
            ]
        );
    }
}
