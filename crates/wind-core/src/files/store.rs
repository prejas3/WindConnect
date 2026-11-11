//! Incoming files on disk until they are complete (PROTOCOL.md §9.1,
//! resume): `<partials>/<peer fingerprint>/<id>.part` with its `.state`
//! beside it. `<partials>` lies outside the inbox, so no path from a peer
//! reaches it (WN-38 review).
//!
//! CCS/1 RULE:a bit is durable only after its data: sync_data, then state tmp + rename.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime};

use super::path;
use super::wire::{self, Id, Offer};

const MAGIC: &[u8; 4] = b"WCP1";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What an `Offer` must match for its partial file to be resumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Meta {
    size: u64,
    chunk: u32,
    modified: Option<u64>,
}

impl Meta {
    fn of(offer: &Offer) -> Self {
        Meta {
            size: offer.size,
            chunk: offer.chunk,
            modified: offer.modified,
        }
    }

    fn encode(&self, have: &[u8]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&self.size.to_be_bytes());
        out.extend_from_slice(&self.chunk.to_be_bytes());
        out.extend_from_slice(&self.modified.unwrap_or(u64::MAX).to_be_bytes());
        out.extend_from_slice(have);
        out
    }

    fn decode(bytes: &[u8]) -> Option<(Self, Vec<u8>)> {
        let rest = bytes.strip_prefix(MAGIC)?;
        if rest.len() < 20 {
            return None;
        }
        let size = u64::from_be_bytes(rest[0..8].try_into().ok()?);
        let chunk = u32::from_be_bytes(rest[8..12].try_into().ok()?);
        let modified = u64::from_be_bytes(rest[12..20].try_into().ok()?);
        Some((
            Meta {
                size,
                chunk,
                modified: (modified != u64::MAX).then_some(modified),
            },
            rest[20..].to_vec(),
        ))
    }
}

/// A file being received.
pub struct Partial {
    /// Fingerprint of the sending device (its Mark-of-the-Web, WN-96).
    peer: [u8; 32],
    part: PathBuf,
    state: PathBuf,
    meta: Meta,
    chunks: u64,
    file: RwLock<Option<Arc<File>>>,
    /// Chunks written (verified), durable or not yet.
    written: Mutex<(Vec<u8>, u64)>,
    /// One flush at a time, so states are written in order.
    flushing: Mutex<()>,
}

impl Partial {
    /// Opens the partial file of `(peer, offer.id)`: resumed when its state
    /// matches the offer, else started anew. Returns it and its durable
    /// `have` bitmap.
    pub fn open(root: &Path, peer: &[u8; 32], offer: &Offer) -> io::Result<(Self, Vec<u8>)> {
        let dir = root.join(hex(peer));
        fs::create_dir_all(&dir)?;
        let stem = hex(&offer.id);
        let part = dir.join(format!("{stem}.part"));
        let state = dir.join(format!("{stem}.state"));
        let meta = Meta::of(offer);
        let chunks = offer.chunks();
        let resumed = fs::read(&state)
            .ok()
            .and_then(|b| Meta::decode(&b))
            .filter(|(m, have)| *m == meta && have.len() as u64 <= chunks.div_ceil(8))
            .map(|(_, have)| have);
        let file = match &resumed {
            Some(_) => OpenOptions::new().read(true).write(true).open(&part),
            None => {
                let _ = fs::remove_file(&state);
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&part)
            }
        };
        let file = match file {
            Ok(f) => f,
            // The state survived but its data didn't: start over.
            Err(e) if e.kind() == io::ErrorKind::NotFound && resumed.is_some() => {
                let _ = fs::remove_file(&state);
                return Self::open(root, peer, offer);
            }
            Err(e) => return Err(e),
        };
        file.set_len(offer.size)?;
        let have = resumed.unwrap_or_default();
        let count = (0..chunks).filter(|i| wire::has(&have, *i)).count() as u64;
        Ok((
            Partial {
                peer: *peer,
                part,
                state,
                meta,
                chunks,
                file: RwLock::new(Some(Arc::new(file))),
                written: Mutex::new((have.clone(), count)),
                flushing: Mutex::new(()),
            },
            have,
        ))
    }

    fn file(&self) -> io::Result<Arc<File>> {
        self.file
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| io::Error::other("partial file closed"))
    }

    /// Whether chunk `index` is written.
    pub fn has(&self, index: u64) -> bool {
        wire::has(
            &self.written.lock().unwrap_or_else(|e| e.into_inner()).0,
            index,
        )
    }

    /// Chunks written so far.
    pub fn written(&self) -> u64 {
        self.written.lock().unwrap_or_else(|e| e.into_inner()).1
    }

    pub fn chunks(&self) -> u64 {
        self.chunks
    }

    /// Writes a verified chunk (blocking) and marks it written. True if it
    /// was new.
    pub fn write_chunk(&self, index: u64, offset: u64, data: &[u8]) -> io::Result<bool> {
        if self.has(index) {
            return Ok(false);
        }
        write_at(&*self.file()?, data, offset)?;
        let mut w = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if wire::has(&w.0, index) {
            return Ok(false);
        }
        wire::set(&mut w.0, index);
        w.1 += 1;
        Ok(true)
    }

    /// Makes what is written durable, then records it (blocking).
    pub fn flush(&self) -> io::Result<()> {
        let _one = self.flushing.lock().unwrap_or_else(|e| e.into_inner());
        let have = self
            .written
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .clone();
        self.file()?.sync_data()?;
        let tmp = self.state.with_extension("state.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&self.meta.encode(&have))?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.state)
    }

    /// Moves the complete file to a free name for `parts` under `inbox`
    /// (blocking); returns where it went.
    pub fn finish(&self, inbox: &Path, parts: &[&str]) -> io::Result<PathBuf> {
        let mut file = self
            .file
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| io::Error::other("partial file closed"))?;
        // A late duplicate chunk may still hold the handle for a moment;
        // Windows renames only a closed file.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let file = loop {
            match Arc::try_unwrap(file) {
                Ok(f) => break f,
                Err(shared) if std::time::Instant::now() < deadline => {
                    file = shared;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return Err(io::Error::other("partial file still in use")),
            }
        };
        file.sync_all()?;
        drop(file);
        let (target, claimed) = path::claim(inbox, parts)?;
        drop(claimed);
        // Mark-of-the-Web (WN-96) before the file gets its name, so it is
        // never in the inbox unmarked (an NTFS rename keeps the streams).
        // Best effort, never fails the transfer: FAT32, exFAT and network
        // shares may have no streams.
        let _ = mark_of_the_web(&self.part, &self.peer);
        // Replaces only the empty file just claimed (create_new), never
        // anything else.
        if let Err(e) = fs::rename(&self.part, &target) {
            // Partials on another volume than the inbox: copy, then delete.
            let copied = fs::copy(&self.part, &target)
                .and_then(|_| OpenOptions::new().write(true).open(&target)?.sync_all());
            if copied.is_err() {
                let _ = fs::remove_file(&target);
                return Err(e);
            }
            let _ = fs::remove_file(&self.part);
            // The copy may not have carried the stream over.
            let _ = mark_of_the_web(&target, &self.peer);
        }
        let _ = fs::remove_file(&self.state);
        Ok(target)
    }

    /// Deletes the partial file and its state (cancelled).
    pub fn discard(&self) {
        self.file.write().unwrap_or_else(|e| e.into_inner()).take();
        let _ = fs::remove_file(&self.part);
        let _ = fs::remove_file(&self.state);
    }
}

/// Contents of the `Zone.Identifier` stream of a file from `peer`
/// (PROTOCOL.md §9.1): Internet zone, so SmartScreen and Office Protected
/// View treat it as downloaded.
pub fn zone_identifier(peer: &[u8; 32]) -> String {
    // Explicit escapes: the bytes must not depend on the checkout's line
    // endings (.gitattributes eol=lf).
    format!(
        "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=windconnect:{}\r\n",
        hex(peer)
    )
}

#[cfg(test)]
thread_local! {
    /// Makes `mark_of_the_web` fail as on a volume without streams.
    static NO_STREAMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Writes the NTFS stream `<file>:Zone.Identifier` of a file received from
/// `peer` (Windows; elsewhere nothing). Its error is the caller's to ignore.
pub fn mark_of_the_web(file: &Path, peer: &[u8; 32]) -> io::Result<()> {
    #[cfg(test)]
    if NO_STREAMS.with(|n| n.get()) {
        return Err(io::Error::from(io::ErrorKind::Unsupported));
    }
    #[cfg(windows)]
    {
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let mut f = File::create(PathBuf::from(stream))?;
        f.write_all(zone_identifier(peer).as_bytes())?;
        f.sync_all()
    }
    #[cfg(not(windows))]
    {
        let _ = (file, peer);
        Ok(())
    }
}

/// The partial files of `(peer, id)`, for deleting without opening.
pub fn discard(root: &Path, peer: &[u8; 32], id: &Id) {
    let dir = root.join(hex(peer));
    let stem = hex(id);
    let _ = fs::remove_file(dir.join(format!("{stem}.part")));
    let _ = fs::remove_file(dir.join(format!("{stem}.state")));
}

/// Deletes partial files unchanged for `max_age` (transfers never
/// resumed); returns how many transfers went.
pub fn clean(root: &Path, max_age: Duration) -> usize {
    let now = SystemTime::now();
    let mut removed = 0;
    let Ok(peers) = fs::read_dir(root) else {
        return 0;
    };
    for peer in peers.flatten() {
        let Ok(entries) = fs::read_dir(peer.path()) else {
            continue;
        };
        // Newest change per transfer (its .part, .state and .state.tmp).
        let mut newest: std::collections::BTreeMap<String, (SystemTime, Vec<PathBuf>)> =
            Default::default();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next().unwrap_or_default().to_owned();
            let changed = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let slot = newest
                .entry(stem)
                .or_insert((SystemTime::UNIX_EPOCH, Vec::new()));
            slot.0 = slot.0.max(changed);
            slot.1.push(entry.path());
        }
        for (_, (changed, files)) in newest {
            if now.duration_since(changed).unwrap_or_default() >= max_age {
                for f in files {
                    let _ = fs::remove_file(f);
                }
                removed += 1;
            }
        }
        let _ = fs::remove_dir(peer.path());
    }
    removed
}

/// Free bytes for a new file in `dir`.
pub fn free_space(dir: &Path) -> io::Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let mut free = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(free)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| io::Error::other("NUL in path"))?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        #[allow(clippy::unnecessary_cast)]
        Ok(st.f_bavail as u64 * st.f_frsize as u64)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = dir;
        Ok(u64::MAX)
    }
}

/// Reads exactly `buf.len()` bytes at `offset` (blocking).
pub fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < buf.len() {
            let n = std::os::windows::fs::FileExt::seek_read(
                file,
                &mut buf[done..],
                offset + done as u64,
            )?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, buf, offset);
        Err(io::Error::other("positional reads unsupported"))
    }
}

fn write_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::write_all_at(file, data, offset)
    }
    #[cfg(windows)]
    {
        let mut done = 0;
        while done < data.len() {
            let n = std::os::windows::fs::FileExt::seek_write(
                file,
                &data[done..],
                offset + done as u64,
            )?;
            if n == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            done += n;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, data, offset);
        Err(io::Error::other("positional writes unsupported"))
    }
}

/// Reads a whole small file; used by tests.
#[allow(dead_code)]
fn read_all(path: &Path) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    File::open(path)?.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(id: u8, size: u64, modified: Option<u64>) -> Offer {
        Offer {
            id: [id; 16],
            path: "x".into(),
            size,
            chunk: wire::MIN_CHUNK,
            streams: 1,
            modified,
        }
    }

    #[test]
    fn state_records_only_flushed_chunks_and_resumes_when_meta_matches() {
        let inbox = tempfile::tempdir().unwrap();
        let o = offer(1, 3 * u64::from(wire::MIN_CHUNK), Some(7));
        let (p, have) = Partial::open(inbox.path(), &[9; 32], &o).unwrap();
        assert!(have.is_empty());
        let data = vec![5u8; wire::MIN_CHUNK as usize];
        assert!(p.write_chunk(0, 0, &data).unwrap());
        assert!(!p.write_chunk(0, 0, &data).unwrap());
        p.flush().unwrap();
        p.write_chunk(2, 2 * u64::from(wire::MIN_CHUNK), &data)
            .unwrap();
        drop(p); // no flush: chunk 2 is not durable
        let (p, have) = Partial::open(inbox.path(), &[9; 32], &o).unwrap();
        assert_eq!(have, vec![0x80]);
        assert_eq!(p.written(), 1);
        drop(p);
        // Same size, another modification time: start over.
        let (_, have) = Partial::open(inbox.path(), &[9; 32], &offer(1, o.size, Some(8))).unwrap();
        assert!(have.is_empty());
    }

    #[test]
    fn partials_of_two_devices_with_one_id_stay_apart() {
        let (inbox, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let o = offer(3, u64::from(wire::MIN_CHUNK), None);
        let (a, _) = Partial::open(root.path(), &[1; 32], &o).unwrap();
        let (b, _) = Partial::open(root.path(), &[2; 32], &o).unwrap();
        a.write_chunk(0, 0, &vec![0xAA; wire::MIN_CHUNK as usize])
            .unwrap();
        b.write_chunk(0, 0, &vec![0xBB; wire::MIN_CHUNK as usize])
            .unwrap();
        let fa = a.finish(inbox.path(), &["a.bin"]).unwrap();
        let fb = b.finish(inbox.path(), &["b.bin"]).unwrap();
        assert!(read_all(&fa).unwrap().iter().all(|x| *x == 0xAA));
        assert!(read_all(&fb).unwrap().iter().all(|x| *x == 0xBB));
    }

    #[test]
    fn old_partials_are_cleaned() {
        let inbox = tempfile::tempdir().unwrap();
        let (p, _) = Partial::open(inbox.path(), &[1; 32], &offer(1, 10, None)).unwrap();
        p.flush().unwrap();
        drop(p);
        let (q, _) = Partial::open(inbox.path(), &[1; 32], &offer(2, 10, None)).unwrap();
        drop(q);
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        let dir = inbox.path().join(hex(&[1; 32]));
        for name in [hex(&[1; 16]) + ".part", hex(&[1; 16]) + ".state"] {
            File::options()
                .write(true)
                .open(dir.join(name))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        assert_eq!(clean(inbox.path(), Duration::from_secs(7 * 24 * 3600)), 1);
        assert!(!dir.join(hex(&[1; 16]) + ".part").exists());
        assert!(dir.join(hex(&[2; 16]) + ".part").exists());
    }

    #[test]
    fn zone_identifier_is_internet_zone_with_the_device() {
        let mut want = b"[ZoneTransfer]\x0d\x0aZoneId=3\x0d\x0aHostUrl=windconnect:".to_vec();
        want.extend_from_slice("ab".repeat(32).as_bytes());
        want.extend_from_slice(b"\x0d\x0a");
        assert_eq!(zone_identifier(&[0xAB; 32]).as_bytes(), want);
    }

    #[cfg(windows)]
    #[test]
    fn received_file_has_zone_identifier_stream() {
        let (inbox, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let o = offer(4, 10, None);
        let (p, _) = Partial::open(root.path(), &[7; 32], &o).unwrap();
        p.write_chunk(0, 0, b"0123456789").unwrap();
        let f = p.finish(inbox.path(), &["setup.exe"]).unwrap();
        assert_eq!(read_all(&f).unwrap(), b"0123456789");
        let mut stream = f.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let zone = read_all(Path::new(&stream)).unwrap();
        let mut want = b"[ZoneTransfer]\x0d\x0aZoneId=3\x0d\x0aHostUrl=windconnect:".to_vec();
        want.extend_from_slice("07".repeat(32).as_bytes());
        want.extend_from_slice(b"\x0d\x0a");
        assert_eq!(zone, want, "{:?}", String::from_utf8_lossy(&zone));
        // Nothing left behind in the partials.
        assert!(!root
            .path()
            .join(hex(&[7; 32]))
            .join(hex(&[4; 16]) + ".part")
            .exists());
        // The stream is not part of the file's data.
        assert_eq!(fs::metadata(&f).unwrap().len(), 10);
    }

    #[test]
    fn transfer_finishes_on_a_volume_without_streams() {
        let (inbox, root) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (p, _) = Partial::open(root.path(), &[7; 32], &offer(5, 3, None)).unwrap();
        p.write_chunk(0, 0, b"abc").unwrap();
        NO_STREAMS.with(|n| n.set(true));
        let f = p.finish(inbox.path(), &["a.txt"]);
        NO_STREAMS.with(|n| n.set(false));
        let f = f.unwrap();
        assert_eq!(read_all(&f).unwrap(), b"abc");
        assert!(mark_of_the_web(&f, &[7; 32]).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn mark_of_the_web_error_is_returned_not_hidden() {
        // A missing file: the stream can't be made, the error comes back.
        let dir = tempfile::tempdir().unwrap();
        assert!(mark_of_the_web(&dir.path().join("none").join("x"), &[1; 32]).is_err());
    }

    #[test]
    fn free_space_is_known() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).unwrap() > 0);
    }
}
