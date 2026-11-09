#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn core_version() -> &'static str {
    wind_core::VERSION
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_version])
        .run(tauri::generate_context!())
        .expect("failed to run WindConnect");
}
