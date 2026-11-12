//! Windows key store: the identity key in the user profile, encrypted with
//! DPAPI for the current user (WC-08).
//!
//! The key file starts with a format header, so the format can change later
//! without losing the key:
//!
//! | bytes | content |
//! |---|---|
//! | 4 | magic `WCID` |
//! | 1 | format: `1` = DPAPI (current user), PKCS#8 PEM inside |
//! | rest | `CryptProtectData` output |
//!
//! A file that is plain PKCS#8 PEM (format 0, what [`FileKeyStore`] wrote
//! before WC-08) is migrated on load: checked to be an Ed25519 PKCS#8
//! private key, then encrypted and replaced atomically. Any other PEM, and a
//! format newer than this code knows, is an error that leaves the file as
//! it is, never a reason to make a new key.
//!
//! Windows can briefly refuse the replace (or the read before it): a virus
//! scanner or the indexer holding the file open without `FILE_SHARE_DELETE`,
//! often right after the file was written, or other starts migrating the
//! same file at the same moment. So each replace is retried a few times
//! ([`RETRY_DELAYS`], about 300 ms in all, with jitter), and a start whose
//! replace still fails checks whether the file now holds the same key in
//! format 1 (another start migrated it) and, if so, carries on. If the file
//! is still plain, nothing is settled yet: the start pauses
//! ([`MIGRATION_PAUSES`], growing, with jitter) and does the whole migration
//! again, read included, up to [`MIGRATION_ROUNDS`] times (about 2 to 2.5 s
//! in all, only while the file stays locked). Only then does load
//! return the error, and the file stays as it was: it is never removed, and
//! the next start tries the migration again.
//!
//! DPAPI is the operating system's: this module only calls it. The data is
//! bound to the current user's DPAPI master key (never
//! `CRYPTPROTECT_LOCAL_MACHINE`, which any account on the machine could
//! decrypt), so another Windows account can't decrypt it even with a copy of
//! the file. The file itself lives in `%LOCALAPPDATA%`, whose ACL already
//! keeps other standard accounts out.
//!
//! [`FileKeyStore`]: super::FileKeyStore
//! [`RETRY_DELAYS`]: super::RETRY_DELAYS

use std::io;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::SigningKey;
use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use zeroize::Zeroizing;

use super::{
    is_transient, jittered, read_key_file, replace_key_file, write_new_key_file, KeyStore,
};

/// Magic at the start of a key file with a format header.
pub const MAGIC: &[u8; 4] = b"WCID";
/// Format 1: DPAPI, current user, PKCS#8 PEM inside.
pub const FORMAT_DPAPI_USER: u8 = 1;
/// Pauses between the rounds of one start's migration (read, check, replace
/// with [`RETRY_DELAYS`](super::RETRY_DELAYS)), before jitter. They grow, so
/// that concurrent starts spread out.
pub const MIGRATION_PAUSES: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
];
/// How many times one start does the whole migration before it gives up.
pub const MIGRATION_ROUNDS: usize = MIGRATION_PAUSES.len() + 1;
/// Folder under `%LOCALAPPDATA%` and file name of the default key file.
pub const PROFILE_DIR: &str = "WindConnect";
pub const KEY_FILE: &str = "identity.key";

/// Extra entropy for DPAPI, so that a blob from this store isn't accepted by
/// a plain `CryptUnprotectData` call of another program of the same user.
const ENTROPY: &[u8] = b"WindConnect device identity key v1";

/// The identity key in one file, encrypted with DPAPI for the current user.
#[derive(Debug, Clone)]
pub struct DpapiKeyStore {
    path: PathBuf,
}

impl DpapiKeyStore {
    /// A store at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        DpapiKeyStore { path: path.into() }
    }

    /// The store in the user profile:
    /// `%LOCALAPPDATA%\WindConnect\identity.key`.
    ///
    /// Local, not roaming: a roaming profile would carry the key to another
    /// PC, and two PCs with one key would be one device to every peer.
    pub fn in_user_profile() -> io::Result<Self> {
        let local = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "%LOCALAPPDATA% is not set"))?;
        Ok(Self::new(
            PathBuf::from(local).join(PROFILE_DIR).join(KEY_FILE),
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file now holds `pem`'s key in format 1.
    fn holds_migrated(&self, pem: &str) -> bool {
        let public = |pem: &str| {
            SigningKey::from_pkcs8_pem(pem)
                .ok()
                .map(|k| k.verifying_key())
        };
        let Ok(Some(bytes)) = read_key_file(&self.path) else {
            return false;
        };
        let Some(blob) = bytes
            .strip_prefix(&MAGIC[..])
            .and_then(|rest| rest.strip_prefix(&[FORMAT_DPAPI_USER]))
        else {
            return false;
        };
        let Ok(stored) = unprotect(blob).and_then(into_pem) else {
            return false;
        };
        let theirs = public(&stored);
        theirs.is_some() && theirs == public(pem)
    }

    fn encode(pem: &str) -> io::Result<Vec<u8>> {
        let blob = protect(pem.as_bytes())?;
        let mut out = Vec::with_capacity(MAGIC.len() + 1 + blob.len());
        out.extend_from_slice(MAGIC);
        out.push(FORMAT_DPAPI_USER);
        out.extend_from_slice(&blob);
        Ok(out)
    }
}

impl KeyStore for DpapiKeyStore {
    fn load(&self) -> io::Result<Option<Zeroizing<String>>> {
        let mut pauses = MIGRATION_PAUSES.iter();
        loop {
            let pem = match self.load_once()? {
                Loaded::Ready(pem) => return Ok(pem),
                Loaded::Plain(pem) => pem,
            };
            let err = match replace_key_file(&self.path, &Self::encode(&pem)?) {
                Ok(()) => return Ok(Some(pem)),
                Err(e) => e,
            };
            // Another start may have migrated it in the meantime.
            if self.holds_migrated(&pem) {
                return Ok(Some(pem));
            }
            match pauses.next() {
                Some(pause) if is_transient(&err) => std::thread::sleep(jittered(*pause)),
                _ => return Err(err),
            }
        }
    }

    fn store_new(&self, pem: &str) -> io::Result<bool> {
        write_new_key_file(&self.path, &Self::encode(pem)?)
    }
}

/// What one read of the key file found.
enum Loaded {
    /// Nothing stored, or the key from a current file.
    Ready(Option<Zeroizing<String>>),
    /// A valid Ed25519 key in plain PEM (format 0), still to be migrated.
    Plain(Zeroizing<String>),
}

impl DpapiKeyStore {
    fn load_once(&self) -> io::Result<Loaded> {
        let Some(bytes) = read_key_file(&self.path)? else {
            return Ok(Loaded::Ready(None));
        };
        match bytes.get(..MAGIC.len() + 1) {
            Some(head) if head[..MAGIC.len()] == MAGIC[..] => {
                let format = head[MAGIC.len()];
                if format != FORMAT_DPAPI_USER {
                    return Err(invalid(format!(
                        "identity key file has format {format}, newer than this version knows"
                    )));
                }
                let pem = unprotect(&bytes[MAGIC.len() + 1..])?;
                into_pem(pem).map(|p| Loaded::Ready(Some(p)))
            }
            _ if bytes.starts_with(b"-----BEGIN ") => {
                // Format 0: plain PEM from before WC-08. To be encrypted in
                // place, but only a valid Ed25519 PKCS#8 key: anything else
                // stays as it is and is reported.
                let pem = into_pem(Zeroizing::new(bytes.to_vec()))?;
                SigningKey::from_pkcs8_pem(&pem).map_err(|e| {
                    invalid(format!(
                        "identity key file is PEM but not an Ed25519 PKCS#8 private key: {e}"
                    ))
                })?;
                Ok(Loaded::Plain(pem))
            }
            _ => Err(invalid("identity key file has an unknown format".into())),
        }
    }
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn into_pem(bytes: Zeroizing<Vec<u8>>) -> io::Result<Zeroizing<String>> {
    match std::str::from_utf8(&bytes) {
        Ok(s) => Ok(Zeroizing::new(s.to_owned())),
        Err(_) => Err(invalid("identity key is not text".into())),
    }
}

fn blob_in(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "key data too large"))?,
        pbData: data.as_ptr().cast_mut(),
    })
}

/// Output of DPAPI, freed with `LocalFree` (after zeroing) on drop.
struct LocalBlob(CRYPT_INTEGER_BLOB);

impl LocalBlob {
    fn as_slice(&self) -> &[u8] {
        if self.0.pbData.is_null() {
            return &[];
        }
        // SAFETY: DPAPI returned `cbData` bytes at `pbData`, owned by us
        // until LocalFree.
        unsafe { std::slice::from_raw_parts(self.0.pbData, self.0.cbData as usize) }
    }
}

impl Drop for LocalBlob {
    fn drop(&mut self) {
        if self.0.pbData.is_null() {
            return;
        }
        // SAFETY: as in as_slice; zeroed before it is handed back.
        unsafe {
            ptr::write_bytes(self.0.pbData, 0, self.0.cbData as usize);
            LocalFree(self.0.pbData as HLOCAL);
        }
    }
}

/// Encrypts `data` for the current user.
pub fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    protect_with(data, ENTROPY)
}

/// Encrypts `data` for the current user, bound to `purpose` (DPAPI optional
/// entropy): a blob for one purpose doesn't decrypt as another's (WN-37's
/// clipboard pins use their own).
pub fn protect_with(data: &[u8], purpose: &[u8]) -> io::Result<Vec<u8>> {
    let input = blob_in(data)?;
    let entropy = blob_in(purpose)?;
    let mut out = LocalBlob(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    });
    // SAFETY: input and entropy point at live slices; DPAPI allocates the
    // output, which LocalBlob frees.
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out.0,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(out.as_slice().to_vec())
}

/// Decrypts `blob`, which only the user who encrypted it can do.
pub fn unprotect(blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
    unprotect_with(blob, ENTROPY)
}

/// Decrypts a [`protect_with`] blob made for the same `purpose`.
pub fn unprotect_with(blob: &[u8], purpose: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
    let input = blob_in(blob)?;
    let entropy = blob_in(purpose)?;
    let mut out = LocalBlob(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    });
    // SAFETY: as in protect.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            &entropy,
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out.0,
        )
    };
    if ok == 0 {
        let os = io::Error::last_os_error();
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("data can't be decrypted for this user: {os}"),
        ));
    }
    Ok(Zeroizing::new(out.as_slice().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_ciphertext_hides_plaintext() {
        let secret = b"-----BEGIN PRIVATE KEY-----\nsecret\n";
        let blob = protect(secret).unwrap();
        assert!(!blob.windows(6).any(|w| w == b"secret"));
        assert_eq!(&unprotect(&blob).unwrap()[..], secret);
    }

    #[test]
    fn tampered_or_foreign_blob_is_rejected() {
        let mut blob = protect(b"key").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 1;
        assert_eq!(
            unprotect(&blob).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
