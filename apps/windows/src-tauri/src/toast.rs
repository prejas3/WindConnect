//! CCS/1 M:the toast sender id shared by battery alerts (WN-49) and the phone's notifications (WN-43).
//! RULE:debug builds aren't installed (no Start menu entry) and send as PowerShell, as tauri's own plugin does; release
//! registers its identifier under HKCU as "WindConnect" (checked from the MSI in WN-118).

/// The AppUserModelID toasts are shown under.
#[cfg(windows)]
pub fn app_id(app: &tauri::AppHandle) -> Result<String, String> {
    #[cfg(debug_assertions)]
    {
        let _ = app;
        Ok(tauri_winrt_notification::Toast::POWERSHELL_APP_ID.to_string())
    }
    #[cfg(not(debug_assertions))]
    {
        register_app_id(&app.config().identifier).map_err(|e| e.to_string())
    }
}

/// Registers `id` as this user's toast sender ("WindConnect"); an unpackaged app needs it.
#[cfg(all(windows, not(debug_assertions)))]
fn register_app_id(id: &str) -> std::io::Result<String> {
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide(&format!(r"Software\Classes\AppUserModelId\{id}"));
    let name = wide("DisplayName");
    let value = wide("WindConnect");
    // SAFETY: NUL-terminated strings; the size is the value's bytes including its NUL.
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_SZ,
            value.as_ptr().cast(),
            (value.len() * 2) as u32,
        )
    };
    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(result as i32));
    }
    Ok(id.to_string())
}
