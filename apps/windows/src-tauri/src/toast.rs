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
    /// `HKCU\key` value `name` (None: the default value) as a string; None when it isn't there.
    #[cfg_attr(debug_assertions, allow(dead_code))]
    pub fn get(key: &str, name: Option<&str>) -> Option<String> {
        use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
        let key = wide(key);
        let name = name.map(wide);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr());
        let mut buffer = vec![0u16; 2048];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: NUL-terminated key and name; the buffer and its byte size are ours.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name_ptr,
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if result != 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
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

/// The exe a registered scheme command runs (`"path" "%1"` or `path "%1"`).
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn command_exe(command: &str) -> Option<std::path::PathBuf> {
    let command = command.trim();
    let path = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

/// Whether `path` lies under `dir` (case-insensitive, as Windows paths are).
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn is_under(path: &std::path::Path, dir: &std::path::Path) -> bool {
    let (path, dir) = (
        path.to_string_lossy().to_lowercase(),
        dir.to_string_lossy().to_lowercase(),
    );
    let dir = dir.trim_end_matches('\\');
    path.starts_with(dir) && path[dir.len()..].starts_with('\\')
}

/// WN-146: whether this exe should point [`PANEL_URI`] at itself. Yes when nothing is registered, the registered exe
/// is gone, it already is this exe, or this is the installed app (under Program Files). So a portable release EXE
/// (test builds) never takes over an installed app's scheme that works; it takes a missing or dead one, which the next
/// start of the installed app takes back. (Uninstalling the MSI removes the scheme whoever wrote it; a portable EXE
/// registers it again on its next toast.)
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn takes_scheme(
    registered: Option<&str>,
    this: &std::path::Path,
    exists: impl Fn(&std::path::Path) -> bool,
    installed: bool,
) -> bool {
    let Some(owner) = registered.and_then(command_exe) else {
        return true;
    };
    installed
        || !exists(&owner)
        || owner
            .to_string_lossy()
            .eq_ignore_ascii_case(&this.to_string_lossy())
}

/// Registers `id` as this user's toast sender ("WindConnect" with the app icon) and [`PANEL_URI`] for this exe.
#[cfg(all(windows, not(debug_assertions)))]
fn register(id: &str) -> std::io::Result<String> {
    let key = app_id_key(id);
    // The one write toasts need; the icon and the click scheme are best effort (WN-146): a failure there costs the
    // icon or a click after a restart, never every toast (battery and the phone's notifications).
    registry::set(&key, Some("DisplayName"), "WindConnect")?;
    if let Err(error) = register_icon(&key) {
        eprintln!("toast icon: {error}");
    }
    if let Err(error) = register_scheme() {
        eprintln!("toast click scheme: {error}");
    }
    Ok(id.to_string())
}

#[cfg(all(windows, not(debug_assertions)))]
fn register_icon(key: &str) -> std::io::Result<()> {
    let dir = crate::pairing::data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let icon = dir.join(ICON_FILE);
    let png: &[u8] = include_bytes!("../icons/app-96.png");
    if std::fs::read(&icon).ok().as_deref() != Some(png) {
        std::fs::write(&icon, png)?;
    }
    registry::set(key, Some("IconUri"), &icon.to_string_lossy())
}

/// Points [`PANEL_URI`] at this exe, unless another working copy owns it ([`takes_scheme`]).
#[cfg(all(windows, not(debug_assertions)))]
fn register_scheme() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let command_key = format!(r"{SCHEME_KEY}\shell\open\command");
    let registered = registry::get(&command_key, None);
    let installed = std::env::var_os("ProgramFiles")
        .is_some_and(|dir| is_under(&exe, std::path::Path::new(&dir)));
    if !takes_scheme(registered.as_deref(), &exe, |p| p.is_file(), installed) {
        return Ok(());
    }
    registry::set(SCHEME_KEY, None, "URL:WindConnect")?;
    registry::set(SCHEME_KEY, Some("URL Protocol"), "")?;
    registry::set(&command_key, None, &open_command(&exe))
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
    fn the_registered_command_names_its_exe() {
        let exe = std::path::Path::new(r"C:\Program Files\WindConnect\windconnect-windows.exe");
        assert_eq!(command_exe(&open_command(exe)).as_deref(), Some(exe));
        assert_eq!(
            command_exe(r#"C:\Tools\wc.exe "%1""#).as_deref(),
            Some(std::path::Path::new(r"C:\Tools\wc.exe"))
        );
        assert_eq!(command_exe("  "), None);
        assert_eq!(command_exe(r#""" "%1""#), None);
    }

    #[test]
    fn under_a_folder_ignores_case_and_needs_a_separator() {
        let dir = std::path::Path::new(r"C:\Program Files");
        assert!(is_under(
            std::path::Path::new(r"c:\program files\WindConnect\a.exe"),
            dir
        ));
        assert!(is_under(
            std::path::Path::new(r"C:\Program Files\a.exe"),
            std::path::Path::new(r"C:\Program Files\")
        ));
        assert!(!is_under(
            std::path::Path::new(r"C:\Program Files (x86)\a.exe"),
            dir
        ));
        assert!(!is_under(std::path::Path::new(r"D:\Temp\a.exe"), dir));
    }

    #[test]
    fn a_portable_copy_never_takes_a_working_installed_scheme() {
        let installed =
            std::path::Path::new(r"C:\Program Files\WindConnect\windconnect-windows.exe");
        let portable =
            std::path::Path::new(r"C:\Users\a\AppData\Local\Temp\WindConnect-0.70.0.exe");
        let owned_by = |exe: &std::path::Path| open_command(exe);
        let alive = |_: &std::path::Path| true;
        let dead = |_: &std::path::Path| false;
        // Nothing registered: anyone takes it.
        assert!(takes_scheme(None, portable, alive, false));
        // The installed app owns a working one: a portable copy leaves it.
        assert!(!takes_scheme(
            Some(&owned_by(installed)),
            portable,
            alive,
            false
        ));
        // ...but takes it once that exe is gone.
        assert!(takes_scheme(
            Some(&owned_by(installed)),
            portable,
            dead,
            false
        ));
        // The installed app always takes it back, also from a working portable copy.
        assert!(takes_scheme(
            Some(&owned_by(portable)),
            installed,
            alive,
            true
        ));
        // Already this exe (any case): rewritten, harmless.
        let upper = std::path::PathBuf::from(portable.to_string_lossy().to_uppercase());
        assert!(takes_scheme(
            Some(&owned_by(&upper)),
            portable,
            alive,
            false
        ));
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
