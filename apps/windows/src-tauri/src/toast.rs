//! CCS/1 M:the toast sender id shared by battery alerts (WN-49) and the phone's notifications (WN-43).
//! RULE:debug builds aren't installed (no Start menu entry) and send as PowerShell, as tauri's own plugin does; release
//! registers its identifier under HKCU as "WindConnect" with the app's icon (checked from the MSI in WN-118).
//! FLOW:a battery toast clicked once its process is gone (Action Center after a restart, the app closed) can't reach
//! an in-process handler: it launches PANEL_URI, a per-user URL scheme -> this exe -> the running app shows the panel
//! (send_pipe), or a new start does (WN-118). The MSI shortcut also carries the id; the HKCU entries serve portable
//! release builds too and are removed on uninstall (`--remove-toasts`).

/// tauri.conf.json `identifier`, for the uninstall cleanup that runs before tauri starts (a test keeps them equal).
pub const IDENTIFIER: &str = "io.github.prejas3.windconnect";

/// A battery toast's launch: opens the panel (WN-118).
pub const PANEL_URI: &str = "windconnect-panel:";

/// Whether this process was started by a toast's [`PANEL_URI`].
pub fn panel_requested(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| {
        arg.eq_ignore_ascii_case(PANEL_URI) || arg.eq_ignore_ascii_case("windconnect-panel:/")
    })
}

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
        register(&app.config().identifier).map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod registry {
    use std::io;
    use windows_sys::Win32::System::Registry::{
        RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
    };
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    /// `HKCU\key` value `name` (None: the key's default value) = `value`.
    #[cfg_attr(debug_assertions, allow(dead_code))]
    pub fn set(key: &str, name: Option<&str>, value: &str) -> io::Result<()> {
        let (key, value) = (wide(key), wide(value));
        let name = name.map(wide);
        // SAFETY: NUL-terminated strings; the size is the value's bytes including its NUL.
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        match result {
            0 => Ok(()),
            e => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
    /// Removes `HKCU\key` with its subkeys; a missing key is fine.
    pub fn delete(key: &str) -> io::Result<()> {
        let key = wide(key);
        // SAFETY: NUL-terminated key under the current user's hive.
        match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) } {
            0 | 2 => Ok(()),
            e => Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
}

const SCHEME_KEY: &str = r"Software\Classes\windconnect-panel";
const ICON_FILE: &str = "toast-icon.png";
fn app_id_key(id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{id}")
}
/// The command a click on [`PANEL_URI`] runs.
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn open_command(exe: &std::path::Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Registers `id` as this user's toast sender ("WindConnect" with the app icon) and [`PANEL_URI`] for this exe.
#[cfg(all(windows, not(debug_assertions)))]
fn register(id: &str) -> std::io::Result<String> {
    let dir = crate::pairing::data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let icon = dir.join(ICON_FILE);
    let png: &[u8] = include_bytes!("../icons/app-96.png");
    if std::fs::read(&icon).ok().as_deref() != Some(png) {
        std::fs::write(&icon, png)?;
    }
    let key = app_id_key(id);
    registry::set(&key, Some("DisplayName"), "WindConnect")?;
    registry::set(&key, Some("IconUri"), &icon.to_string_lossy())?;
    registry::set(SCHEME_KEY, None, "URL:WindConnect")?;
    registry::set(SCHEME_KEY, Some("URL Protocol"), "")?;
    let exe = std::env::current_exe()?;
    registry::set(
        &format!(r"{SCHEME_KEY}\shell\open\command"),
        None,
        &open_command(&exe),
    )?;
    Ok(id.to_string())
}

/// Uninstall: the HKCU entries and icon [`register`] wrote (best effort, like the other per-user cleanups).
#[cfg(windows)]
pub fn remove(id: &str) -> std::io::Result<()> {
    registry::delete(&app_id_key(id))?;
    registry::delete(SCHEME_KEY)?;
    if let Ok(dir) = crate::pairing::data_dir() {
        let _ = std::fs::remove_file(dir.join(ICON_FILE));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_panel_uri_requests_the_panel() {
        let args = |a: &[&str]| {
            a.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert!(panel_requested(args(&["app.exe", "windconnect-panel:"])));
        assert!(panel_requested(args(&["app.exe", "WINDCONNECT-PANEL:/"])));
        assert!(!panel_requested(args(&["app.exe"])));
        assert!(
            !panel_requested(args(&["app.exe", "windconnect-panel:x"])),
            "nothing else rides on the scheme"
        );
        assert!(!panel_requested(args(&[
            "app.exe",
            "--send",
            r"C:\windconnect-panel:"
        ])));
    }

    #[test]
    fn identifier_matches_the_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], IDENTIFIER);
    }

    #[test]
    fn the_scheme_runs_this_exe_with_the_uri_quoted() {
        let exe = std::path::Path::new(r"C:\Program Files\WindConnect\windconnect-windows.exe");
        assert_eq!(
            open_command(exe),
            r#""C:\Program Files\WindConnect\windconnect-windows.exe" "%1""#
        );
    }
}
