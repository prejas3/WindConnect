//! CCS/1 IO:Explorer context menu "Send to phone" (WN-39) -> HKCU\Software\Classes\{*,Directory}\shell
//! RULE:current user only, no administrator rights; rewritten at every start (the exe may move), removed on uninstall.
//! SEC:the command is this exe with --send and the quoted item; no shell, no other arguments.
use std::io;

/// The verb's key name under `shell`.
pub const VERB: &str = "WindConnect.SendToPhone";
/// Where the verb goes: every file, and folders.
pub const CLASSES: [&str; 2] = [
    r"Software\Classes\*\shell",
    r"Software\Classes\Directory\shell",
];
/// The Explorer menu's label, in the UI language when it was registered (WN-63).
pub fn label() -> &'static str {
    crate::i18n::tr("files.send_to_phone")
}

/// The command Explorer runs for one selected item.
pub fn command(executable: &std::path::Path) -> io::Result<String> {
    let path = executable
        .to_str()
        .ok_or_else(|| io::Error::other("non-Unicode executable path"))?;
    if path.contains(['"', '\0']) {
        return Err(io::Error::other("invalid executable path"));
    }
    Ok(format!("\"{path}\" --send \"%1\""))
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn check(result: u32) -> io::Result<()> {
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(result as i32))
        }
    }
    fn set_value(key: &str, name: Option<&str>, value: &str) -> io::Result<()> {
        let (key, data) = (wide(key), wide(value));
        let name = name.map(wide);
        // SAFETY: NUL-terminated buffers live across the call; the size is in bytes.
        check(unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        })
    }
    pub fn install_at(roots: &[&str], executable: &std::path::Path) -> io::Result<()> {
        let command = command(executable)?;
        let path = executable.to_str().unwrap_or_default();
        for root in roots {
            let verb = format!(r"{root}\{VERB}");
            set_value(&verb, Some("MUIVerb"), label())?;
            set_value(&verb, Some("Icon"), &format!("\"{path}\",0"))?;
            // One launch per item even past 15 selected; each forwards to the running app.
            set_value(&verb, Some("MultiSelectModel"), "Player")?;
            set_value(&format!(r"{verb}\command"), None, &command)?;
        }
        Ok(())
    }
    pub fn remove_at(roots: &[&str]) -> io::Result<()> {
        for root in roots {
            let verb = wide(&format!(r"{root}\{VERB}"));
            // SAFETY: NUL-terminated key path.
            let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, verb.as_ptr()) };
            if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
                return Err(io::Error::from_raw_os_error(result as i32));
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn read_command_at(root: &str) -> io::Result<Option<String>> {
        let key = wide(&format!(r"{root}\{VERB}\command"));
        let mut size = 0u32;
        // SAFETY: size query with a null buffer.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(result)?;
        let mut buffer = vec![0u16; size as usize / 2];
        // SAFETY: the buffer holds `size` bytes.
        check(unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        })?;
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16(&buffer[..end])
            .map(Some)
            .map_err(io::Error::other)
    }

    #[cfg(test)]
    #[test]
    fn registry_round_trip_on_a_private_test_key() {
        let root = format!(
            r"Software\WindConnect\Tests\shell-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let key = wide(&self.0);
                unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
            }
        }
        let cleanup = Cleanup(root);
        let roots = [cleanup.0.as_str()];
        let exe = std::path::Path::new(r"C:\Program Files\WindConnect\WindConnect.exe");
        assert_eq!(read_command_at(&cleanup.0).unwrap(), None);
        install_at(&roots, exe).unwrap();
        assert_eq!(
            read_command_at(&cleanup.0).unwrap().as_deref(),
            Some(r#""C:\Program Files\WindConnect\WindConnect.exe" --send "%1""#)
        );
        remove_at(&roots).unwrap();
        assert_eq!(read_command_at(&cleanup.0).unwrap(), None);
        remove_at(&roots).unwrap();
    }
}

/// Points the menu at this exe.
#[cfg(windows)]
pub fn install() -> io::Result<()> {
    platform::install_at(&CLASSES, &std::env::current_exe()?)
}
#[cfg(windows)]
pub fn remove() -> io::Result<()> {
    platform::remove_at(&CLASSES)
}
#[cfg(not(windows))]
pub fn install() -> io::Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn remove() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_quotes_the_exe_and_the_item() {
        assert_eq!(
            command(std::path::Path::new(
                r"C:\Program Files\WindConnect\app.exe"
            ))
            .unwrap(),
            r#""C:\Program Files\WindConnect\app.exe" --send "%1""#
        );
        assert!(command(std::path::Path::new("bad\"path")).is_err());
    }
}
