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
    #[cfg(test)]
    mod registry_tests {
        use super::*;
        use windows_sys::Win32::UI::Shell::SHDeleteEmptyKeyW;

        struct TestKeys {
            path: String,
            created: Vec<String>,
        }
        impl TestKeys {
            fn new(path: String) -> io::Result<Self> {
                let mut keys = Self {
                    path,
                    created: Vec::new(),
                };
                let mut prefix = String::new();
                // CCS/1 RULE:one component per create, so disposition identifies ownership of every ancestor.
                for part in keys.path.split('\\') {
                    if !prefix.is_empty() {
                        prefix.push('\\');
                    }
                    prefix.push_str(part);
                    let mut handle = std::ptr::null_mut();
                    let mut disposition = 0;
                    let result = unsafe {
                        RegCreateKeyExW(
                            HKEY_CURRENT_USER,
                            wide(&prefix).as_ptr(),
                            0,
                            std::ptr::null(),
                            REG_OPTION_NON_VOLATILE,
                            KEY_READ | KEY_WRITE,
                            std::ptr::null(),
                            &mut handle,
                            &mut disposition,
                        )
                    };
                    if result != ERROR_SUCCESS {
                        return Err(io::Error::from_raw_os_error(result as i32));
                    }
                    unsafe {
                        RegCloseKey(handle);
                    }
                    if disposition == REG_CREATED_NEW_KEY {
                        keys.created.push(prefix.clone());
                    }
                }
                // Never adopt an existing leaf or overwrite another test's/user's autostart value.
                if !keys.created.contains(&keys.path) {
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                Ok(keys)
            }
            fn cleanup(&mut self) {
                if self.created.contains(&self.path) {
                    // Only our named leaf value, including on panic before the normal set(false).
                    let _ = set_at(&self.path, false);
                }
                let mut remaining = Vec::new();
                for path in std::mem::take(&mut self.created).into_iter().rev() {
                    // CCS/1 SEC:nonrecursive, refuses values AND subkeys; never remove borrowed ancestors.
                    let result =
                        unsafe { SHDeleteEmptyKeyW(HKEY_CURRENT_USER, wide(&path).as_ptr()) };
                    if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
                        remaining.push(path);
                    }
                }
                remaining.reverse();
                self.created = remaining;
            }
        }
        impl Drop for TestKeys {
            fn drop(&mut self) {
                self.cleanup();
            }
        }
        fn unique_path() -> String {
            format!(
                r"Software\WindConnect\Tests\autostart-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )
        }
        fn exists(path: &str) -> bool {
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    KEY_READ,
                    &mut handle,
                )
            };
            if result == ERROR_SUCCESS {
                unsafe {
                    RegCloseKey(handle);
                }
                true
            } else {
                assert_eq!(result, ERROR_FILE_NOT_FOUND);
                false
            }
        }

        #[test]
        fn registry_roundtrip_uses_a_private_test_key_not_the_login_key() {
            let mut keys = TestKeys::new(unique_path()).unwrap();
            assert!(!enabled_at(&keys.path).unwrap());
            set_at(&keys.path, true).unwrap();
            assert!(enabled_at(&keys.path).unwrap());
            set_at(&keys.path, false).unwrap();
            assert!(!enabled_at(&keys.path).unwrap());
            keys.cleanup();
            assert!(!exists(&keys.path));
        }

        #[test]
        fn removes_own_empty_parents_but_preserves_preexisting_keys_and_values() {
            let fixture = TestKeys::new(unique_path()).unwrap();
            set_at(&fixture.path, true).unwrap();
            assert_eq!(
                TestKeys::new(fixture.path.clone()).err().unwrap().kind(),
                io::ErrorKind::AlreadyExists
            );
            let existing = TestKeys::new(format!(r"{}\existing", fixture.path)).unwrap();
            let mut keys = TestKeys::new(format!(r"{}\ours\leaf", existing.path)).unwrap();
            let parent = format!(r"{}\ours", existing.path);
            keys.cleanup();
            assert!(!exists(&parent));
            assert!(exists(&existing.path), "preexisting empty key stays");
            assert!(
                enabled_at(&fixture.path).unwrap(),
                "preexisting value stays"
            );
        }

        #[test]
        fn preserves_a_parallel_tests_subkey_until_it_is_gone() {
            let fixture = TestKeys::new(unique_path()).unwrap();
            let parent = format!(r"{}\shared", fixture.path);
            let mut first = TestKeys::new(format!(r"{parent}\first")).unwrap();
            let second = TestKeys::new(format!(r"{parent}\second")).unwrap();
            set_at(&second.path, true).unwrap();
            first.cleanup();
            assert!(exists(&parent));
            assert!(enabled_at(&second.path).unwrap());
            drop(second);
            first.cleanup();
            assert!(!exists(&parent));
        }

        #[test]
        fn preserves_an_owned_parent_that_gains_a_value() {
            struct Value(String);
            impl Drop for Value {
                fn drop(&mut self) {
                    let _ = set_at(&self.0, false);
                }
            }
            let fixture = TestKeys::new(unique_path()).unwrap();
            let parent = format!(r"{}\with-value", fixture.path);
            let mut keys = TestKeys::new(format!(r"{parent}\leaf")).unwrap();
            let value = Value(parent.clone());
            set_at(&value.0, true).unwrap();
            keys.cleanup();
            assert!(
                enabled_at(&parent).unwrap(),
                "even an owned parent with data stays"
            );
            drop(value);
            keys.cleanup();
            assert!(!exists(&parent));
        }

        #[test]
        fn drop_cleans_its_leaf_value_and_empty_parents_after_panic() {
            let fixture = TestKeys::new(unique_path()).unwrap();
            let parent = format!(r"{}\panic", fixture.path);
            let leaf = format!(r"{parent}\leaf");
            assert!(std::panic::catch_unwind(|| {
                let keys = TestKeys::new(leaf).unwrap();
                set_at(&keys.path, true).unwrap();
                panic!("exercise cleanup on unwind");
            })
            .is_err());
            assert!(!exists(&parent));
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
