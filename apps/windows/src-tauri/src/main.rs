#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::Arc;
use tauri::Manager;
use wind_core::network::NetworkChanges;

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
    tauri::Builder::default()
        .setup(|app| {
            app.manage(NetworkState::new()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_version,
            network_generation,
            ui_dev_mode,
            window_action,
            apply_window_theme
        ])
        .run(tauri::generate_context!())
        .expect("failed to run WindConnect");
}
