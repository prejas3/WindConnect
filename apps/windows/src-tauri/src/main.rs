#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::Arc;
use tauri::Manager;
use wind_core::network::NetworkChanges;
mod autostart;
mod hotkey;
pub mod indicator;
mod instance;
mod panel;
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
        let monitor = wind_core::network::WindowsNetworkMonitor::start(changes.clone())?;
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

fn main() {
    if std::env::args().any(|arg| arg == "--remove-autostart") {
        if let Err(error) = autostart::set(false) {
            eprintln!("autostart cleanup: {error}");
            std::process::exit(1);
        }
        return;
    }
    let activate = !std::env::args().any(|arg| arg == "--autostart");
    let _instance = match instance::claim(activate) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            eprintln!("single instance: {error}");
            return;
        }
    };
    tauri::Builder::default()
        .manage(panel::Runtime::default())
        .setup(|app| {
            app.manage(NetworkState::new()?);
            app.manage(tray::install(app.handle())?);
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
            if !std::env::args().any(|arg| arg == "--autostart") {
                tray::show_main(app.handle())?;
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(false)
                if matches!(window.label(), "tray-menu" | "panel") =>
            {
                let _ = window.hide();
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
            panel::panel_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("failed to run WindConnect");
}
