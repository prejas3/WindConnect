//! Device identity (PROTOCOL.md §2): the device's Ed25519 key, the
//! self-signed certificate it presents in TLS 1.3, and the key's
//! fingerprint for display.
//!
//! A device has exactly one identity key. It is created on first start and
//! loaded from its [`KeyStore`] on every start after that; a stored key that
//! cannot be read is an error, never a reason to make a new one, because a
//! new key unpairs the device from every peer.
//!
//! Only audited libraries do cryptography here: `ed25519-dalek` holds the
//! key, `rcgen` (on `ring`) signs the certificate, `sha2` hashes the
//! fingerprint.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::OsRng;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

#[cfg(windows)]
pub mod dpapi;
#[cfg(windows)]
pub use dpapi::DpapiKeyStore;

/// Subject alternative name in every device certificate. It carries no
/// trust (PROTOCOL.md §2); it only gives TLS a name to use.
pub const CERT_SERVER_NAME: &str = "windconnect";

/// Why an identity could not be made or loaded.
#[derive(Debug)]
pub enum IdentityError {
    /// The key store failed to read or write.
    Io(io::Error),
    /// The stored key is not a valid PKCS#8 Ed25519 key.
    Corrupt(String),
    /// The certificate could not be built.
    Certificate(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::Io(e) => write!(f, "identity key store: {e}"),
            IdentityError::Corrupt(e) => write!(f, "stored identity key is unreadable: {e}"),
            IdentityError::Certificate(e) => write!(f, "identity certificate: {e}"),
        }
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IdentityError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for IdentityError {
    fn from(e: io::Error) -> Self {
        IdentityError::Io(e)
    }
}

/// Where the identity key lives between starts.
///
/// The key crosses this boundary as PKCS#8 PEM. [`FileKeyStore`] keeps it
/// in a plain file; on Windows `DpapiKeyStore` keeps it encrypted for the
/// user; the Android Keystore plugs in here too.
pub trait KeyStore {
    /// The stored key, or `None` when nothing has been stored yet.
    fn load(&self) -> io::Result<Option<Zeroizing<String>>>;
    /// Stores `pem` unless a key is already stored. Returns `false`, storing
    /// nothing, when one is (another process won a first-start race).
    fn store_new(&self, pem: &str) -> io::Result<bool>;
}

/// A key store in one file, as PKCS#8 PEM.
///
/// The file is written once, atomically (a uniquely named temporary file
/// in the same folder, then linked into place), and never overwritten, so
/// concurrent first starts, in one process or several, all end up with the
/// same key. A write removes only the temporary file it created; one left by
/// a crashed write stays (it can't be told from another writer's) and
/// blocks nothing. On Windows a read, link or rename the system briefly
/// refuses (another process holding the file open without
/// `FILE_SHARE_DELETE`, such as a virus scanner, or a concurrent replace) is
/// retried a few times; see [`RETRY_DELAYS`]. On Unix it is
/// readable by its owner only; on Windows it relies on the folder's ACL
/// alone, so Windows uses `DpapiKeyStore`, which migrates a file written
/// here.
#[derive(Debug, Clone)]
pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        FileKeyStore { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> io::Result<Option<Zeroizing<String>>> {
        match read_key_file(&self.path)? {
            None => Ok(None),
            Some(bytes) => String::from_utf8(bytes.to_vec())
                .map(|s| Some(Zeroizing::new(s)))
                .map_err(|e| {
                    let mut raw = e.into_bytes();
                    raw.zeroize();
                    io::Error::new(io::ErrorKind::InvalidData, "identity key is not text")
                }),
        }
    }

    fn store_new(&self, pem: &str) -> io::Result<bool> {
        write_new_key_file(&self.path, pem.as_bytes())
    }
}

/// How many fresh temporary names one write tries before giving up.
const TMP_ATTEMPTS: usize = 8;

#[cfg(test)]
impl FileKeyStore {
    fn store_new_via(
        &self,
        pem: &str,
        tmp_names: impl IntoIterator<Item = PathBuf>,
    ) -> io::Result<bool> {
        write_new_key_file_via(&self.path, pem.as_bytes(), tmp_names)
    }
}

/// The folder of a key file (`.` for a bare file name).
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Pauses between attempts of a key file operation Windows briefly refused
/// (five attempts, about 300 ms in all). Each pause is lengthened by a
/// random jitter of up to half of it, so that concurrent writers don't keep
/// colliding in step.
///
/// Windows refuses to read, link or replace a file while another process
/// holds it open without `FILE_SHARE_DELETE` (Microsoft Defender scanning a
/// freshly written file, the search indexer), and while another thread is
/// replacing it: `ERROR_ACCESS_DENIED` or a sharing violation, gone a moment
/// later. A first start after an update migrates the key file, so without a
/// retry the app could fail to start. Retrying never removes the key file:
/// once the attempts run out the error is returned and the file is as it was.
pub const RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(160),
];

/// Whether `e` may be a brief refusal by the system rather than a lasting
/// one. Only on Windows: on Unix a permission error is the file's mode.
pub(crate) fn is_transient(e: &io::Error) -> bool {
    #[cfg(windows)]
    {
        const ERROR_SHARING_VIOLATION: i32 = 32;
        const ERROR_LOCK_VIOLATION: i32 = 33;
        e.kind() == io::ErrorKind::PermissionDenied
            || matches!(
                e.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            )
    }
    #[cfg(not(windows))]
    {
        let _ = e;
        false
    }
}

/// `op`, tried again after each of [`RETRY_DELAYS`] while it fails with a
/// transient error; the last error once they run out.
fn retry_transient<T>(op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    retry_with(&RETRY_DELAYS, is_transient, op)
}

/// `d` plus a random jitter of up to half of it.
pub(crate) fn jittered(d: Duration) -> Duration {
    use rand_core::RngCore;
    let half = u64::try_from(d.as_micros() / 2).unwrap_or(u64::MAX);
    if half == 0 {
        return d;
    }
    d + Duration::from_micros(OsRng.next_u64() % (half + 1))
}

fn retry_with<T>(
    delays: &[Duration],
    transient: impl Fn(&io::Error) -> bool,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let mut delays = delays.iter();
    loop {
        match op() {
            Err(e) if transient(&e) => match delays.next() {
                Some(d) => std::thread::sleep(jittered(*d)),
                None => return Err(e),
            },
            result => return result,
        }
    }
}

/// A key file's bytes, or `None` when it doesn't exist.
pub(crate) fn read_key_file(path: &Path) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    match retry_transient(|| fs::read(path)) {
        Ok(b) => Ok(Some(Zeroizing::new(b))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Fresh, unique temporary names beside `path`.
fn tmp_names(path: &Path) -> io::Result<impl Iterator<Item = PathBuf>> {
    let dir = parent_dir(path);
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "key path has no file name"))?
        .to_os_string();
    Ok((0..TMP_ATTEMPTS).map(move |_| {
        let mut tmp = name.clone();
        tmp.push(format!(".{}.tmp", unique_suffix()));
        dir.join(tmp)
    }))
}

/// Writes `data` to `path` unless it exists: `false`, writing nothing, when
/// it does. Atomic, and safe against concurrent writers.
pub(crate) fn write_new_key_file(path: &Path, data: &[u8]) -> io::Result<bool> {
    fs::create_dir_all(parent_dir(path))?;
    write_new_key_file_via(path, data, tmp_names(path)?)
}

/// Replaces `path` with `data` atomically (a format migration): readers see
/// the old file or the new one, never a mix. A rename Windows briefly
/// refuses is retried ([`RETRY_DELAYS`]); on failure `path` is untouched.
// Only platform stores migrate formats; none does off Windows yet.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn replace_key_file(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = create_first_free(tmp_names(path)?)?;
    tmp.write_synced(data)?;
    tmp.persist_over(path)?;
    sync_dir(&parent_dir(path));
    Ok(())
}

fn create_first_free(names: impl IntoIterator<Item = PathBuf>) -> io::Result<OwnedTempFile> {
    for name in names {
        match OwnedTempFile::create(name) {
            Ok(t) => return Ok(t),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary name for the identity key",
    ))
}

/// Writes `data` to the first of `tmp_names` it can create exclusively, then
/// links it into place. A name that already exists belongs to someone else
/// and is left alone.
fn write_new_key_file_via(
    path: &Path,
    data: &[u8],
    tmp_names: impl IntoIterator<Item = PathBuf>,
) -> io::Result<bool> {
    let mut tmp = create_first_free(tmp_names)?;
    tmp.write_synced(data)?;
    // A hard link fails if the target exists, so a key stored by another
    // writer in the meantime is never replaced.
    let stored = match retry_transient(|| fs::hard_link(&tmp.path, path)) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
        Err(e) => return Err(e),
    };
    drop(tmp);
    if stored {
        sync_dir(&parent_dir(path));
    }
    Ok(stored)
}

/// A suffix no other write attempt uses: process, a per-process counter and
/// 64 random bits.
fn unique_suffix() -> String {
    use rand_core::RngCore;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{:016x}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        OsRng.next_u64()
    )
}

/// A temporary file this attempt created (exclusively, owner-only on Unix).
/// Dropping it removes it; nothing else is ever removed.
struct OwnedTempFile {
    path: PathBuf,
    file: Option<File>,
}

impl OwnedTempFile {
    fn create(path: PathBuf) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        Ok(OwnedTempFile {
            path,
            file: Some(file),
        })
    }

    fn write_synced(&mut self, data: &[u8]) -> io::Result<()> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("temporary file already written"))?;
        file.write_all(data)?;
        file.sync_all()
    }

    /// Renames the file over `target`, replacing it; it is no longer ours
    /// to remove.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn persist_over(mut self, target: &Path) -> io::Result<()> {
        drop(self.file.take());
        retry_transient(|| fs::rename(&self.path, target))?;
        self.path = PathBuf::new();
        Ok(())
    }
}

impl Drop for OwnedTempFile {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn sync_dir(dir: &Path) {
    // Makes the new name durable on Unix; directories can't be opened as
    // files on Windows, where NTFS journals the link itself.
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// An Ed25519 public key: what peers pin (PROTOCOL.md §2).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// A public key from its 32 raw bytes, if they are a valid point.
    pub fn from_bytes(bytes: &[u8; 32]) -> Option<Self> {
        VerifyingKey::from_bytes(bytes)
            .ok()
            .map(|_| PublicKey(*bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key as a DER SubjectPublicKeyInfo (RFC 8410), as in the
    /// certificate.
    pub fn spki_der(&self) -> Vec<u8> {
        let mut der = Vec::with_capacity(SPKI_PREFIX.len() + 32);
        der.extend_from_slice(&SPKI_PREFIX);
        der.extend_from_slice(&self.0);
        der
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint(Sha256::digest(self.spki_der()).into())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.fingerprint())
    }
}

/// DER header of an Ed25519 SubjectPublicKeyInfo (RFC 8410 §4): the key's
/// 32 bytes follow it.
const SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// The fingerprint of a public key: SHA-256 of its DER
/// SubjectPublicKeyInfo (PROTOCOL.md §2.1).
///
/// `Display` gives the form people compare: the first 160 bits in
/// Crockford base32, eight groups of four (`7KQM 3D1X …`). [`Fingerprint::to_hex`]
/// gives all 256 bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

/// Bytes of the fingerprint shown to people: 160 bits.
pub const DISPLAY_BYTES: usize = 20;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

impl Fingerprint {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// All 256 bits as lowercase hex.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The displayed bits as 32 Crockford base32 characters, ungrouped.
    pub fn to_compact(&self) -> String {
        let mut out = String::with_capacity(32);
        let mut acc: u64 = 0;
        let mut bits = 0;
        for &b in &self.0[..DISPLAY_BYTES] {
            acc = (acc << 8) | u64::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(CROCKFORD[((acc >> bits) & 31) as usize] as char);
            }
        }
        debug_assert_eq!(bits, 0);
        out
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let compact = self.to_compact();
        for (i, group) in compact.as_bytes().chunks(4).enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            // Crockford base32 is ASCII.
            f.write_str(std::str::from_utf8(group).map_err(|_| fmt::Error)?)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// This device's identity: its key and the certificate made from it.
pub struct DeviceIdentity {
    key: SigningKey,
    cert: CertificateDer<'static>,
}

impl DeviceIdentity {
    /// A new random identity (not stored).
    pub fn generate() -> Result<Self, IdentityError> {
        Self::from_key(SigningKey::generate(&mut OsRng))
    }

    /// The stored identity, or a new one stored now if there is none.
    ///
    /// A stored key that can't be read is [`IdentityError::Corrupt`]: the
    /// caller decides whether to reset it, since that unpairs the device.
    pub fn load_or_create(store: &impl KeyStore) -> Result<Self, IdentityError> {
        if let Some(pem) = store.load()? {
            return Self::from_pkcs8_pem(&pem);
        }
        let identity = Self::generate()?;
        if store.store_new(&identity.to_pkcs8_pem()?)? {
            return Ok(identity);
        }
        // Another process stored its key first: use that one.
        match store.load()? {
            Some(pem) => Self::from_pkcs8_pem(&pem),
            None => Err(IdentityError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "identity key vanished after it was stored",
            ))),
        }
    }

    /// An identity from a PKCS#8 PEM Ed25519 private key.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self, IdentityError> {
        let key =
            SigningKey::from_pkcs8_pem(pem).map_err(|e| IdentityError::Corrupt(e.to_string()))?;
        Self::from_key(key)
    }

    /// The private key as PKCS#8 PEM, for a [`KeyStore`].
    pub fn to_pkcs8_pem(&self) -> Result<Zeroizing<String>, IdentityError> {
        self.key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| IdentityError::Corrupt(e.to_string()))
    }

    fn from_key(key: SigningKey) -> Result<Self, IdentityError> {
        let cert = self_signed_certificate(&key)?;
        Ok(DeviceIdentity { key, cert })
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.key.verifying_key().to_bytes())
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.public_key().fingerprint()
    }

    /// The self-signed certificate to present in TLS 1.3.
    ///
    /// It is the same for the same key: the serial is derived from the key,
    /// the validity dates are fixed and Ed25519 signatures are
    /// deterministic.
    pub fn certificate_der(&self) -> CertificateDer<'static> {
        self.cert.clone()
    }

    /// The private key for TLS (PKCS#8 DER).
    pub fn private_key_der(&self) -> Result<PrivateKeyDer<'static>, IdentityError> {
        let der = self
            .key
            .to_pkcs8_der()
            .map_err(|e| IdentityError::Corrupt(e.to_string()))?;
        Ok(PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            der.as_bytes().to_vec(),
        )))
    }
}

impl fmt::Debug for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceIdentity")
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

fn self_signed_certificate(key: &SigningKey) -> Result<CertificateDer<'static>, IdentityError> {
    let cert_err = |e: rcgen::Error| IdentityError::Certificate(e.to_string());
    let der = key
        .to_pkcs8_der()
        .map_err(|e| IdentityError::Corrupt(e.to_string()))?;
    let pkcs8 = PrivatePkcs8KeyDer::from(der.as_bytes());
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&pkcs8, &rcgen::PKCS_ED25519)
        .map_err(cert_err)?;
    let mut params =
        rcgen::CertificateParams::new(vec![CERT_SERVER_NAME.to_string()]).map_err(cert_err)?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "WindConnect device");
    let cert = params.self_signed(&key_pair).map_err(cert_err)?;
    Ok(cert.der().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::pkcs8::EncodePublicKey;

    #[test]
    fn spki_prefix_matches_rfc8410_encoder() {
        let key = SigningKey::generate(&mut OsRng);
        let expected = key.verifying_key().to_public_key_der().unwrap();
        assert_eq!(
            PublicKey(key.verifying_key().to_bytes()).spki_der(),
            expected.as_bytes()
        );
    }

    fn pem() -> Zeroizing<String> {
        DeviceIdentity::generate().unwrap().to_pkcs8_pem().unwrap()
    }

    #[test]
    fn taken_temp_name_is_skipped_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileKeyStore::new(dir.path().join("identity.pem"));
        let taken = dir.path().join("identity.pem.other-writer.tmp");
        fs::write(&taken, "another writer's pending data").unwrap();
        let fresh = dir.path().join("identity.pem.mine.tmp");

        assert!(store
            .store_new_via(&pem(), [taken.clone(), fresh.clone()])
            .unwrap());
        assert_eq!(
            fs::read_to_string(&taken).unwrap(),
            "another writer's pending data"
        );
        assert!(!fresh.exists(), "own temporary file is removed");
        assert!(store.load().unwrap().is_some());
    }

    #[test]
    fn no_free_temp_name_fails_and_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileKeyStore::new(dir.path().join("identity.pem"));
        let taken = dir.path().join("identity.pem.x.tmp");
        fs::write(&taken, "pending").unwrap();

        let err = store.store_new_via(&pem(), [taken.clone()]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&taken).unwrap(), "pending");
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn losing_link_removes_only_its_own_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileKeyStore::new(dir.path().join("identity.pem"));
        assert!(store.store_new(&pem()).unwrap());
        let mine = dir.path().join("identity.pem.loser.tmp");
        assert!(!store.store_new_via(&pem(), [mine.clone()]).unwrap());
        assert!(!mine.exists());
    }

    #[test]
    fn retry_stops_at_success_or_lasting_error() {
        let zero = [Duration::ZERO; 4];
        let busy = || io::Error::from(io::ErrorKind::PermissionDenied);
        let transient = |e: &io::Error| e.kind() == io::ErrorKind::PermissionDenied;

        let mut calls = 0;
        let r = retry_with(&zero, transient, || {
            calls += 1;
            if calls < 3 {
                Err(busy())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(r.unwrap(), 3);

        let mut calls = 0;
        let r: io::Result<()> = retry_with(&zero, transient, || {
            calls += 1;
            Err(busy())
        });
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 5, "one try and one per delay");

        let mut calls = 0;
        let r: io::Result<()> = retry_with(&zero, transient, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(calls, 1, "a lasting error is not retried");
    }

    #[test]
    fn jitter_adds_at_most_half() {
        assert_eq!(jittered(Duration::ZERO), Duration::ZERO);
        let d = Duration::from_millis(40);
        for _ in 0..200 {
            let j = jittered(d);
            assert!(j >= d && j <= d + d / 2, "{j:?}");
        }
    }

    #[test]
    fn transient_errors_are_windows_refusals_only() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        let sharing = io::Error::from_raw_os_error(32);
        assert_eq!(is_transient(&denied), cfg!(windows));
        assert_eq!(is_transient(&sharing), cfg!(windows));
        assert!(!is_transient(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!is_transient(&io::Error::from(
            io::ErrorKind::AlreadyExists
        )));
    }

    #[test]
    fn temp_suffixes_are_unique() {
        let names: std::collections::HashSet<String> = (0..1000).map(|_| unique_suffix()).collect();
        assert_eq!(names.len(), 1000);
    }
}
