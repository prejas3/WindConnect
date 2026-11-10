//! CCS/1 IO:explicit user choice -> HKCU Run | RULE:disabled by default, no administrator rights.
use std::io;

pub fn command(executable: &std::path::Path) -> io::Result<String> {
    let path = executable
        .to_str()
        .ok_or_else(|| io::Error::other("non-Unicode executable path"))?;
    if path.contains(['"', '\0']) {
        return Err(io::Error::other("invalid executable path"));
    }
    Ok(format!("\"{path}\" --autostart"))
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    pub fn enabled() -> io::Result<bool> {
        enabled_at(KEY)
    }
    fn enabled_at(path: &str) -> io::Result<bool> {
        let (key, name) = (wide(path), wide("WindConnect"));
        let mut size = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        if result != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let mut buffer = vec![0u16; size as usize / 2];
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let text = String::from_utf16(
            &buffer[..buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len())],
        )
        .map_err(io::Error::other)?;
        Ok(text == command(&std::env::current_exe()?)?)
    }
    #[cfg(test)]
    #[test]
    fn registry_roundtrip_uses_a_private_test_key_not_the_login_key() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    RegDeleteKeyW(HKEY_CURRENT_USER, wide(&self.0).as_ptr());
                }
            }
        }
        let path = format!(
            r"Software\WindConnect\Tests\autostart-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let cleanup = Cleanup(path);
        assert!(!enabled_at(&cleanup.0).unwrap());
        set_at(&cleanup.0, true).unwrap();
        assert!(enabled_at(&cleanup.0).unwrap());
        set_at(&cleanup.0, false).unwrap();
        assert!(!enabled_at(&cleanup.0).unwrap());
    }
    pub fn set(enabled: bool) -> io::Result<()> {
        set_at(KEY, enabled)
    }
    fn set_at(path: &str, enabled: bool) -> io::Result<()> {
        let (key, name) = (wide(path), wide("WindConnect"));
        if !enabled {
            let result =
                unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
            return if result == ERROR_SUCCESS || result == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(result as i32))
            };
        }
        let value = wide(&command(&std::env::current_exe()?)?);
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        if result != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let result = unsafe {
            RegSetValueExW(
                handle,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        unsafe {
            RegCloseKey(handle);
        }
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(result as i32))
        }
    }
}
#[cfg(windows)]
pub use platform::{enabled, set};
#[cfg(not(windows))]
pub fn enabled() -> io::Result<bool> {
    Ok(false)
}
#[cfg(not(windows))]
pub fn set(_: bool) -> io::Result<()> {
    Err(io::Error::other("Windows only"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_command_quotes_paths_and_has_no_shell() {
        assert_eq!(
            command(std::path::Path::new(
                r"C:\Program Files\WindConnect\app.exe"
            ))
            .unwrap(),
            r#""C:\Program Files\WindConnect\app.exe" --autostart"#
        );
        assert!(command(std::path::Path::new("bad\"path")).is_err());
    }
}
