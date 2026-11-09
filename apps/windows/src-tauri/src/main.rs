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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(NetworkState::new()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![core_version, network_generation])
        .run(tauri::generate_context!())
        .expect("failed to run WindConnect");
}
