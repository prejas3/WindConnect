//! Paired devices, kept between starts (PROTOCOL.md §12.8).
//!
//! [`PairedDevicesFile`] is a text file that is only ever appended to: one
//! record per pairing, `\n<key hex> <unix seconds>[ <name>]`, written with
//! a single append and synced. Every record *starts* with its newline, so a
//! record torn by a crash stays alone on its line and the next one starts
//! clean. Appends need no lock: two processes adding at once both keep
//! their record. A later record for the same key updates its name and
//! time.
//!
//! Reading never rewrites the file and never drops a good record: a line
//! that can't be read is skipped and reported in [`PairedList::skipped`].
//!
//! Removal is appended too (WC-28): `\n-<key hex> <seconds>` removes the
//! key and owes its device a notice (`Goodbye` `UNPAIRED`, PROTOCOL.md
//! §12.10), `\n=<key hex> <seconds>` settles it (notice delivered, or the
//! removal came from that device). A later pairing record pairs the key
//! again. Renaming appends an ordinary record with the new name.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::wire::{clean_name, hex_byte, MAX_NAME};
use crate::identity::PublicKey;
use crate::transport::PinnedKeys;

/// File name of the paired-device list, beside the identity key.
pub const PAIRED_FILE: &str = "paired.txt";

/// How long a removed device is owed its unpair notice (§12.10). After
/// that the key is simply gone: the device's attempts fail in TLS.
pub const NOTICE_DAYS: u64 = 30;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A paired device: its pinned key and what we know to show for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    /// The key from the pairing connection's TLS handshake.
    pub key: PublicKey,
    /// Its name as it gave it, cleaned; for display only.
    pub name: Option<String>,
    /// When it was paired, in seconds since the Unix epoch.
    pub paired_at: u64,
}

impl PairedDevice {
    /// A device paired now.
    pub fn now(key: PublicKey, name: Option<String>) -> Self {
        PairedDevice {
            key,
            name: name.as_deref().and_then(clean_name),
            paired_at: unix_now(),
        }
    }
}

/// Where pairing records a newly paired device.
pub trait PairedStore: Send + Sync {
    /// Records `device` durably (it survives a crash right after).
    fn add(&self, device: &PairedDevice) -> io::Result<()>;
}

/// What a paired-device file holds.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PairedList {
    /// One entry per key, in the order first paired, with the latest name.
    pub devices: Vec<PairedDevice>,
    /// Lines that couldn't be read: 1-based line number and why.
    pub skipped: Vec<(usize, &'static str)>,
    /// Removed devices still owed their unpair notice, with the time of
    /// removal (seconds since the Unix epoch). See [`PairedList::owed`].
    pub revoked: Vec<(PublicKey, u64)>,
}

impl PairedList {
    /// Pins every device's key, and marks the keys still owed an unpair
    /// notice as revoked (notice only, never a session; §12.10).
    pub fn pin_all(&self, pins: &PinnedKeys) {
        for d in &self.devices {
            pins.pin(d.key);
        }
        for key in self.owed(unix_now()) {
            pins.revoke(key);
        }
    }

    /// Keys removed less than [`NOTICE_DAYS`] before `now` whose notice
    /// isn't settled yet.
    pub fn owed(&self, now: u64) -> Vec<PublicKey> {
        self.revoked
            .iter()
            .filter(|(_, at)| now.saturating_sub(*at) < NOTICE_DAYS * 24 * 3600)
            .map(|(key, _)| *key)
            .collect()
    }

    /// The paired device with `key`, if it is paired.
    pub fn get(&self, key: &PublicKey) -> Option<&PairedDevice> {
        self.devices.iter().find(|d| &d.key == key)
    }
}

/// The paired-device list in one append-only file.
#[derive(Debug, Clone)]
pub struct PairedDevicesFile {
    path: PathBuf,
}

impl PairedDevicesFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        PairedDevicesFile { path: path.into() }
    }

    /// `paired.txt` in `dir`, the folder of the identity key.
    pub fn in_dir(dir: impl AsRef<Path>) -> Self {
        Self::new(dir.as_ref().join(PAIRED_FILE))
    }

    /// `%LOCALAPPDATA%\WindConnect\paired.txt`, beside the identity key.
    #[cfg(windows)]
    pub fn in_user_profile() -> io::Result<Self> {
        let key = crate::identity::DpapiKeyStore::in_user_profile()?;
        let dir = key
            .path()
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no profile folder"))?;
        Ok(Self::in_dir(dir))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The devices in the file; an empty list when there is no file.
    pub fn load(&self) -> io::Result<PairedList> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(parse(&bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PairedList::default()),
            Err(e) => Err(e),
        }
    }

    /// Removes `key` and owes its device the unpair notice (§12.10). The
    /// caller also revokes it in the pinned keys ([`PinnedKeys::revoke`]).
    pub fn remove(&self, key: &PublicKey) -> io::Result<()> {
        self.append(&mark('-', key))
    }

    /// The unpair notice to `key` is delivered, or `key`'s device removed
    /// the pairing itself: nothing is owed and the key stays removed.
    pub fn settle(&self, key: &PublicKey) -> io::Result<()> {
        self.append(&mark('=', key))
    }

    /// Gives the paired device `key` the local name `name` (an alias shown
    /// on this device only; it is never sent). `InvalidInput` for a name
    /// that is empty once cleaned, `NotFound` for a key that isn't paired.
    pub fn rename(&self, key: &PublicKey, name: &str) -> io::Result<()> {
        let name = clean_name(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty name"))?;
        let list = self.load()?;
        let device = list
            .get(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not paired"))?;
        self.append(&record(&PairedDevice {
            key: *key,
            name: Some(name),
            paired_at: device.paired_at,
        }))
    }

    /// One append, synced (see the module docs).
    fn append(&self, line: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        // One write: appends are atomic with respect to other appenders.
        file.write_all(line.as_bytes())?;
        file.sync_all()
    }
}

impl PairedStore for PairedDevicesFile {
    fn add(&self, device: &PairedDevice) -> io::Result<()> {
        self.append(&record(device))
    }
}

/// `\n<mark><key hex> <seconds>`: removal (`-`) or settled notice (`=`).
fn mark(kind: char, key: &PublicKey) -> String {
    let mut line = format!("\n{kind}");
    for b in key.as_bytes() {
        line.push_str(&format!("{b:02x}"));
    }
    line.push(' ');
    line.push_str(&unix_now().to_string());
    line
}

/// `\n<key hex> <seconds>[ <name>]`, the name percent-encoded so that it
/// holds no space or line break.
fn record(device: &PairedDevice) -> String {
    let mut line = String::from("\n");
    for b in device.key.as_bytes() {
        line.push_str(&format!("{b:02x}"));
    }
    line.push(' ');
    line.push_str(&device.paired_at.to_string());
    if let Some(name) = device.name.as_deref().and_then(clean_name) {
        line.push(' ');
        for &b in name.as_bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                line.push(b as char);
            } else {
                line.push_str(&format!("%{b:02X}"));
            }
        }
    }
    line
}

/// Reads the bytes of a paired-device file (what [`PairedDevicesFile::load`]
/// does after reading it). Never fails: unreadable lines go to `skipped`.
pub fn parse_paired(bytes: &[u8]) -> PairedList {
    parse(bytes)
}

fn parse(bytes: &[u8]) -> PairedList {
    let mut list = PairedList::default();
    for (i, line) in bytes.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        match parse_entry(line) {
            Ok(Entry::Paired(device)) => {
                list.revoked.retain(|(k, _)| *k != device.key);
                match list.devices.iter_mut().find(|d| d.key == device.key) {
                    Some(known) => {
                        known.name = device.name;
                        known.paired_at = device.paired_at;
                    }
                    None => list.devices.push(device),
                }
            }
            Ok(Entry::Removed(key, at)) => {
                list.devices.retain(|d| d.key != key);
                list.revoked.retain(|(k, _)| *k != key);
                list.revoked.push((key, at));
            }
            Ok(Entry::Settled(key)) => {
                list.devices.retain(|d| d.key != key);
                list.revoked.retain(|(k, _)| *k != key);
            }
            Err(why) => list.skipped.push((i + 1, why)),
        }
    }
    list
}

enum Entry {
    Paired(PairedDevice),
    Removed(PublicKey, u64),
    Settled(PublicKey),
}

fn parse_entry(line: &[u8]) -> Result<Entry, &'static str> {
    match line.first() {
        Some(b'-') | Some(b'=') => {
            let text = std::str::from_utf8(&line[1..]).map_err(|_| "not UTF-8")?;
            let mut fields = text.split(' ');
            let key_hex = fields.next().ok_or("empty")?;
            let seconds = fields.next().ok_or("no time")?;
            if fields.next().is_some() {
                return Err("too many fields");
            }
            let key = parse_key(key_hex)?;
            let at = parse_seconds(seconds)?;
            Ok(if line[0] == b'-' {
                Entry::Removed(key, at)
            } else {
                Entry::Settled(key)
            })
        }
        _ => parse_line(line).map(Entry::Paired),
    }
}

fn parse_seconds(seconds: &str) -> Result<u64, &'static str> {
    if seconds.is_empty() || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return Err("bad time");
    }
    seconds.parse().map_err(|_| "bad time")
}

fn parse_key(key_hex: &str) -> Result<PublicKey, &'static str> {
    // Decoded byte by byte: a field of 64 bytes that isn't ASCII hex (a
    // damaged file) is a bad line, never a slice through a character.
    let hex = key_hex.as_bytes();
    if hex.len() != 64 {
        return Err("bad key");
    }
    let mut key = [0u8; 32];
    for (b, pair) in key.iter_mut().zip(hex.chunks_exact(2)) {
        *b = hex_byte(pair[0], pair[1]).ok_or("bad key")?;
    }
    PublicKey::from_bytes(&key).ok_or("bad key")
}

fn parse_line(line: &[u8]) -> Result<PairedDevice, &'static str> {
    let text = std::str::from_utf8(line).map_err(|_| "not UTF-8")?;
    let mut fields = text.split(' ');
    let key_hex = fields.next().ok_or("empty")?;
    let seconds = fields.next().ok_or("no time")?;
    let name = fields.next();
    if fields.next().is_some() {
        return Err("too many fields");
    }
    let key = parse_key(key_hex)?;
    let paired_at = parse_seconds(seconds)?;
    let name = match name {
        None => None,
        Some(raw) => {
            let decoded = percent_decode(raw).ok_or("bad name")?;
            if decoded.len() > MAX_NAME {
                return Err("name too long");
            }
            Some(
                clean_name(&String::from_utf8(decoded).map_err(|_| "bad name")?)
                    .ok_or("bad name")?,
            )
        }
    };
    Ok(PairedDevice {
        key,
        name,
        paired_at,
    })
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            out.push(hex_byte(*b.get(i + 1)?, *b.get(i + 2)?)?);
            i += 3;
        } else if b[i].is_ascii_graphic() {
            out.push(b[i]);
            i += 1;
        } else {
            return None;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    fn key() -> PublicKey {
        DeviceIdentity::generate().unwrap().public_key()
    }

    #[test]
    fn records_round_trip_with_odd_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path().join("WindConnect"));
        assert_eq!(store.load().unwrap(), PairedList::default());
        let a = PairedDevice::now(key(), Some("Pixel 9 % ż\nx".into()));
        let b = PairedDevice::now(key(), None);
        store.add(&a).unwrap();
        store.add(&b).unwrap();
        let list = store.load().unwrap();
        assert_eq!(list.skipped, vec![]);
        assert_eq!(list.devices.len(), 2);
        assert_eq!(list.devices[0].name.as_deref(), Some("Pixel 9 % żx"));
        assert_eq!(list.devices[1], b);
    }

    #[test]
    fn later_record_updates_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        let k = key();
        store
            .add(&PairedDevice::now(k, Some("old".into())))
            .unwrap();
        store
            .add(&PairedDevice::now(k, Some("new".into())))
            .unwrap();
        let list = store.load().unwrap();
        assert_eq!(list.devices.len(), 1);
        assert_eq!(list.devices[0].name.as_deref(), Some("new"));
    }

    #[test]
    fn bad_line_is_skipped_and_reported_and_nothing_else_lost() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        let (a, b) = (
            PairedDevice::now(key(), None),
            PairedDevice::now(key(), None),
        );
        store.add(&a).unwrap();
        let mut f = OpenOptions::new().append(true).open(store.path()).unwrap();
        f.write_all(b"\nnot a record at all").unwrap();
        drop(f);
        store.add(&b).unwrap();
        let before = fs::read(store.path()).unwrap();
        let list = store.load().unwrap();
        assert_eq!(list.devices, vec![a, b]);
        assert_eq!(list.skipped, vec![(3, "too many fields")]);
        assert_eq!(fs::read(store.path()).unwrap(), before, "never rewritten");
    }

    #[test]
    fn damaged_fields_are_skipped_never_a_panic() {
        let (a, b) = (
            PairedDevice::now(key(), None),
            PairedDevice::now(key(), None),
        );
        let good_a = record(&a);
        let good_b = record(&b);
        let hex: String = a
            .key
            .as_bytes()
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect();
        // 64 bytes, not ASCII: `a`, 31 × `é` (2 bytes each), `a`.
        let non_ascii = format!("\na{}a 5", "é".repeat(31));
        assert_eq!(non_ascii.len() - 3, 64);
        let damaged = [
            non_ascii,
            format!("\n+f{} 5", &hex[2..]),
            format!("\n{hex} +5"),
            format!("\n{hex} 5 %+f"),
            format!("\n{hex} 5 %e"),
            format!("\n{} 5", "g".repeat(64)),
        ];
        let mut file = good_a.clone();
        for d in &damaged {
            file.push_str(d);
        }
        file.push_str(&good_b);
        let list = parse_paired(file.as_bytes());
        assert_eq!(list.devices, vec![a, b]);
        assert_eq!(list.skipped.len(), damaged.len(), "{:?}", list.skipped);
    }

    #[test]
    fn torn_last_record_stays_alone_on_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        let a = PairedDevice::now(key(), Some("a".into()));
        store.add(&a).unwrap();
        // A crash in the middle of an append: half a record, no newline.
        let torn = &record(&PairedDevice::now(key(), None))[..30];
        let mut f = OpenOptions::new().append(true).open(store.path()).unwrap();
        f.write_all(torn.as_bytes()).unwrap();
        drop(f);
        let b = PairedDevice::now(key(), Some("b".into()));
        store.add(&b).unwrap();
        let list = store.load().unwrap();
        assert_eq!(list.devices, vec![a, b]);
        assert_eq!(list.skipped.len(), 1);
    }

    #[test]
    fn concurrent_adds_keep_every_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PAIRED_FILE);
        let keys: Vec<Vec<PublicKey>> = (0..2).map(|_| (0..50).map(|_| key()).collect()).collect();
        std::thread::scope(|s| {
            for mine in &keys {
                let store = PairedDevicesFile::new(&path);
                s.spawn(move || {
                    for k in mine {
                        store.add(&PairedDevice::now(*k, Some("x".into()))).unwrap();
                    }
                });
            }
        });
        let list = PairedDevicesFile::new(&path).load().unwrap();
        assert_eq!(list.skipped, vec![]);
        assert_eq!(list.devices.len(), 100);
        for k in keys.iter().flatten() {
            assert!(list.devices.iter().any(|d| d.key == *k));
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        store.add(&PairedDevice::now(key(), None)).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn removal_owes_a_notice_until_settled_and_pairing_again_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        let (a, b) = (key(), key());
        store.add(&PairedDevice::now(a, Some("A".into()))).unwrap();
        store.add(&PairedDevice::now(b, Some("B".into()))).unwrap();
        store.remove(&a).unwrap();
        let list = store.load().unwrap();
        assert_eq!(
            list.devices.iter().map(|d| d.key).collect::<Vec<_>>(),
            vec![b]
        );
        assert_eq!(
            list.revoked.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            vec![a]
        );
        assert_eq!(list.owed(unix_now()), vec![a]);
        store.settle(&a).unwrap();
        let list = store.load().unwrap();
        assert!(list.revoked.is_empty() && list.get(&a).is_none());
        store.remove(&b).unwrap();
        store
            .add(&PairedDevice::now(b, Some("again".into())))
            .unwrap();
        let list = store.load().unwrap();
        assert!(list.revoked.is_empty());
        assert_eq!(list.get(&b).unwrap().name.as_deref(), Some("again"));
        assert_eq!(list.skipped, vec![]);
    }

    #[test]
    fn a_notice_expires_after_notice_days() {
        let k = key();
        let mut list = PairedList::default();
        list.revoked.push((k, 1_000));
        assert_eq!(list.owed(1_000 + NOTICE_DAYS * 86_400 - 1), vec![k]);
        assert!(list.owed(1_000 + NOTICE_DAYS * 86_400).is_empty());
        let pins = PinnedKeys::new();
        list.pin_all(&pins);
        use crate::transport::PeerTrust;
        assert!(!pins.is_revoked(&k), "1970 + a bit is long expired");
    }

    #[test]
    fn rename_keeps_the_pairing_time_and_refuses_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let store = PairedDevicesFile::in_dir(dir.path());
        let k = key();
        let device = PairedDevice {
            key: k,
            name: Some("Pixel".into()),
            paired_at: 42,
        };
        store.add(&device).unwrap();
        store.rename(&k, "  Telefon Oli  ").unwrap();
        let list = store.load().unwrap();
        assert_eq!(list.get(&k).unwrap().paired_at, 42);
        assert_eq!(list.get(&k).unwrap().name.as_deref(), Some("Telefon Oli"));
        assert_eq!(
            store.rename(&k, " \n ").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            store.rename(&key(), "x").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn damaged_marks_are_skipped_not_applied() {
        let k = key();
        let mut hex = String::new();
        for b in k.as_bytes() {
            hex.push_str(&format!("{b:02x}"));
        }
        let text = format!(
            "\n{hex} 1 A\n-{hex}\n={} 5\n-{hex} 7 extra\n-{hex} x",
            &hex[..10]
        );
        let list = parse_paired(text.as_bytes());
        assert_eq!(list.devices.len(), 1, "no damaged mark removed the device");
        assert!(list.revoked.is_empty());
        assert_eq!(list.skipped.len(), 4);
    }
}
