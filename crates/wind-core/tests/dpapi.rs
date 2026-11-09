//! Windows key store (WC-08): the identity key is kept with DPAPI in the
//! user profile, survives restarts, migrates from the plain format, and
//! another Windows account can't read it.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use wind_core::identity::dpapi::{self, DpapiKeyStore, FORMAT_DPAPI_USER, MAGIC};
use wind_core::identity::{DeviceIdentity, FileKeyStore, PublicKey};

fn header_ok(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC) && bytes.get(MAGIC.len()) == Some(&FORMAT_DPAPI_USER)
}

fn only_file(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn same_key_after_restart_and_file_is_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("WindConnect").join("identity.key");

    let first = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap();
    let pem = first.to_pkcs8_pem().unwrap();
    let key = first.public_key();
    drop(first);

    let bytes = fs::read(&path).unwrap();
    assert!(header_ok(&bytes));
    // Neither the PEM nor its base64 body is in the file.
    let body = pem.lines().nth(1).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("PRIVATE KEY") && !text.contains(body));

    let second = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap();
    assert_eq!(second.public_key(), key);
    assert_eq!(fs::read(&path).unwrap(), bytes, "loading never rewrites");
}

#[test]
fn plain_key_file_is_migrated_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    // What FileKeyStore wrote before WC-08.
    let old = DeviceIdentity::load_or_create(&FileKeyStore::new(&path)).unwrap();
    let plain = fs::read(&path).unwrap();
    assert!(plain.starts_with(b"-----BEGIN PRIVATE KEY-----"));

    let migrated = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap();
    assert_eq!(migrated.public_key(), old.public_key());
    let bytes = fs::read(&path).unwrap();
    assert!(header_ok(&bytes));
    assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE KEY"));
    assert_eq!(only_file(dir.path()), vec!["identity.key".to_string()]);

    // And the migrated file loads like any other.
    let again = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap();
    assert_eq!(again.public_key(), old.public_key());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn concurrent_migration_converges_on_the_old_key() {
    const THREADS: usize = 8;
    for round in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        let old = DeviceIdentity::load_or_create(&FileKeyStore::new(&path))
            .unwrap()
            .public_key();
        let barrier = Arc::new(Barrier::new(THREADS));
        let keys: Vec<PublicKey> = (0..THREADS)
            .map(|_| {
                let (barrier, path) = (barrier.clone(), path.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    DeviceIdentity::load_or_create(&DpapiKeyStore::new(path))
                        .map(|id| id.public_key())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap().expect("every start succeeds"))
            .collect();
        assert!(keys.iter().all(|k| *k == old), "round {round}");
        assert!(header_ok(&fs::read(&path).unwrap()));
        assert_eq!(only_file(dir.path()), vec!["identity.key".to_string()]);
    }
}

#[test]
fn unknown_or_newer_format_is_an_error_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");
    for content in [
        [MAGIC.as_slice(), &[2u8], b"from the future"].concat(),
        b"garbage".to_vec(),
        [MAGIC.as_slice(), &[FORMAT_DPAPI_USER], b"not a dpapi blob"].concat(),
    ] {
        fs::write(&path, &content).unwrap();
        let err = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap_err();
        assert!(err.to_string().contains("identity key"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), content, "never replaced");
    }
}

/// A P-256 PKCS#8 key (a throwaway made with openssl): a valid private key
/// of an algorithm the identity doesn't use.
const P256_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgbZ2dr/BJ/Fd3JIj3
kwvdOQw7NxZHYWVUv9j4Dj8jfvShRANCAAT03oATSTbWu2OX0xYMK5nyLuRc/8XD
SxfOlcjQVeMGV1/duARLLVs2QIsy/msHLMW+xm4GF4DAEsRElDOy9U51
-----END PRIVATE KEY-----
";

#[test]
fn invalid_plain_pem_is_not_migrated() {
    let valid = DeviceIdentity::generate().unwrap().to_pkcs8_pem().unwrap();
    let truncated = valid[..valid.len() - 30].to_string();
    let cert = DeviceIdentity::generate().unwrap().certificate_der();
    let cert_pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        base64(&cert)
    );
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "bad base64",
            b"-----BEGIN PRIVATE KEY-----\n!!!not base64!!!\n-----END PRIVATE KEY-----\n".to_vec(),
        ),
        ("truncated", truncated.into_bytes()),
        ("certificate", cert_pem.into_bytes()),
        ("P-256 key", P256_PEM.as_bytes().to_vec()),
        (
            "not UTF-8",
            b"-----BEGIN PRIVATE KEY-----\n\xff\xfe\n".to_vec(),
        ),
    ];
    for (what, content) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");
        fs::write(&path, &content).unwrap();
        let err = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&path)).unwrap_err();
        assert!(err.to_string().contains("identity key"), "{what}: {err}");
        assert_eq!(fs::read(&path).unwrap(), content, "{what}: file changed");
        assert_eq!(
            only_file(dir.path()),
            vec!["identity.key".to_string()],
            "{what}: folder changed"
        );
    }
}

fn base64(data: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                B64[((n >> (18 - 6 * i)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

#[test]
fn profile_path_is_local_appdata() {
    let store = DpapiKeyStore::in_user_profile().unwrap();
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    assert_eq!(store.path(), local.join("WindConnect").join("identity.key"));
}

// ---------------------------------------------------------------------------
// Another Windows account can't read the key.
//
// Needs a second, standard (non-admin) local account. As an administrator:
//
//     net user wctest <password> /add
//
// then, as the owner of the key:
//
//     set WINDCONNECT_TEST_OTHER_USER=wctest
//     set WINDCONNECT_TEST_OTHER_PASSWORD=<password>
//     cargo test -p wind-core --test dpapi -- --ignored other_account
//
// The test stores a key in this user's %LOCALAPPDATA%, copies this test
// binary and the key file to a folder under C:\Users\Public, and runs the
// copy as the other account (CreateProcessWithLogonW, with that account's
// profile). The copy tries to read the key file in place and to decrypt its
// copy with DPAPI; its exit code says what happened.
// ---------------------------------------------------------------------------

const PROBE_MARKER: &str = "wc-dpapi-probe";
const EXIT_UNREADABLE: i32 = 0;
const EXIT_FILE_READABLE: i32 = 10;
const EXIT_DECRYPTED: i32 = 11;
const EXIT_NO_COPY: i32 = 20;
const EXIT_SAME_USER: i32 = 30;

#[test]
#[ignore = "needs a second Windows account; see the comment above"]
fn other_account_cannot_read_key() {
    let (Some(user), Some(password)) = (
        std::env::var_os("WINDCONNECT_TEST_OTHER_USER"),
        std::env::var_os("WINDCONNECT_TEST_OTHER_PASSWORD"),
    ) else {
        panic!("set WINDCONNECT_TEST_OTHER_USER and WINDCONNECT_TEST_OTHER_PASSWORD");
    };

    let suffix = format!("{}-{}", std::process::id(), rand_suffix());
    // The key, where the real one lives: under this user's %LOCALAPPDATA%.
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
    let key_dir = local.join(format!("WindConnect-test-{suffix}"));
    let key_path = key_dir.join("identity.key");
    let probe_dir = PathBuf::from(r"C:\Users\Public").join(format!("WindConnect-dpapi-{suffix}"));
    let _cleanup = Cleanup(vec![key_dir.clone(), probe_dir.clone()]);

    let id = DeviceIdentity::load_or_create(&DpapiKeyStore::new(&key_path)).unwrap();
    fs::create_dir_all(&probe_dir).unwrap();
    let exe = probe_dir.join("probe.exe");
    fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
    fs::copy(&key_path, probe_dir.join("key.bin")).unwrap();
    fs::write(probe_dir.join("target.txt"), key_path.to_str().unwrap()).unwrap();
    fs::write(
        probe_dir.join("owner.txt"),
        std::env::var("USERNAME").unwrap(),
    )
    .unwrap();
    fs::write(probe_dir.join(PROBE_MARKER), "").unwrap();

    // Positive control: the owner can decrypt the same copy.
    let copy = fs::read(probe_dir.join("key.bin")).unwrap();
    let pem = dpapi::unprotect(&copy[MAGIC.len() + 1..]).unwrap();
    assert_eq!(
        DeviceIdentity::from_pkcs8_pem(std::str::from_utf8(&pem).unwrap())
            .unwrap()
            .public_key(),
        id.public_key()
    );

    let code = run_as(&user, &password, &exe, &probe_dir);
    let detail = fs::read_to_string(probe_dir.join("result.txt")).unwrap_or_default();
    match code {
        EXIT_UNREADABLE => {}
        EXIT_FILE_READABLE => panic!("the other account read the key file: {detail}"),
        EXIT_DECRYPTED => panic!("the other account decrypted the key: {detail}"),
        EXIT_NO_COPY => panic!("probe couldn't read its copy (setup): {detail}"),
        EXIT_SAME_USER => panic!("probe ran as the same user: {detail}"),
        other => panic!("probe exited with {other}: {detail}"),
    }
}

/// Runs inside the copy, as the other account. Does nothing elsewhere.
#[test]
#[ignore = "runs only as the probe of other_account_cannot_read_key"]
fn other_account_probe() {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if !dir.join(PROBE_MARKER).exists() {
        return;
    }
    let mut log = String::new();
    let me = std::env::var("USERNAME").unwrap_or_default();
    log.push_str(&format!("user={me}\n"));
    let owner = fs::read_to_string(dir.join("owner.txt")).unwrap_or_default();
    let exit = |code: i32, log: &str| -> ! {
        let _ = fs::write(dir.join("result.txt"), log);
        std::process::exit(code)
    };
    if me.is_empty() || me.eq_ignore_ascii_case(owner.trim()) {
        exit(EXIT_SAME_USER, &log);
    }

    let target = fs::read_to_string(dir.join("target.txt")).unwrap_or_default();
    match fs::read(target.trim()) {
        Ok(_) => exit(EXIT_FILE_READABLE, &format!("{log}read {target}\n")),
        Err(e) => log.push_str(&format!("key file: {e}\n")),
    }

    let copy = match fs::read(dir.join("key.bin")) {
        Ok(c) if c.len() > MAGIC.len() + 1 => c,
        other => exit(EXIT_NO_COPY, &format!("{log}copy: {other:?}\n")),
    };
    match dpapi::unprotect(&copy[MAGIC.len() + 1..]) {
        Ok(_) => exit(EXIT_DECRYPTED, &format!("{log}decrypted the copy\n")),
        Err(e) => log.push_str(&format!("decrypt copy: {e}\n")),
    }
    exit(EXIT_UNREADABLE, &log);
}

fn rand_suffix() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u64(0);
    format!("{:016x}", h.finish())
}

struct Cleanup(Vec<PathBuf>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for dir in &self.0 {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Runs `exe` (the probe test) as `user` with its profile; its exit code.
fn run_as(user: &std::ffi::OsStr, password: &std::ffi::OsStr, exe: &Path, dir: &Path) -> i32 {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateProcessWithLogonW, GetExitCodeProcess, WaitForSingleObject, CREATE_NO_WINDOW,
        LOGON_WITH_PROFILE, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let user = wide(user);
    let domain = wide(".");
    let mut password = wide(password);
    let app = wide(exe);
    let mut cmd = wide(format!(
        "\"{}\" other_account_probe --exact --ignored --test-threads=1",
        exe.display()
    ));
    let cwd = wide(dir);
    // SAFETY: plain C structs, zero is their documented empty state.
    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // SAFETY: every pointer is a live, NUL-terminated buffer or a struct
    // owned by this frame; the handles returned are closed below.
    let ok = unsafe {
        CreateProcessWithLogonW(
            user.as_ptr(),
            domain.as_ptr(),
            password.as_ptr(),
            LOGON_WITH_PROFILE,
            app.as_ptr(),
            cmd.as_mut_ptr(),
            CREATE_NO_WINDOW,
            std::ptr::null(),
            cwd.as_ptr(),
            &si,
            &mut pi,
        )
    };
    password.iter_mut().for_each(|c| *c = 0);
    assert!(
        ok != 0,
        "CreateProcessWithLogonW failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: handles from a successful CreateProcessWithLogonW.
    unsafe {
        let waited = WaitForSingleObject(pi.hProcess, 120_000);
        let mut code = u32::MAX;
        let got = GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        assert_eq!(waited, WAIT_OBJECT_0, "probe did not finish in 2 minutes");
        assert!(got != 0);
        code as i32
    }
}
