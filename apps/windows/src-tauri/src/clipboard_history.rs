//! CCS/1 M:clipboard history for the panel (WN-37): the last items that crossed between the devices, pins kept across restarts.
//! RULE:only items the phone Applied or that came from it are recorded; private (password) captures never reach here.
//! RULE:20 recent + up to 20 pinned; unpinned image bytes ≤ 64 MB, pinned ones too; a repeat moves to the top.
//! SEC:the history lives in memory; only pins go to disk, DPAPI-encrypted for the current user ([`save_pins`]).
//! SEC:the panel gets [`View`]s (a one-line preview or the thumbnail), never the full text or PNG.
use std::sync::Arc;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MAX_RECENT: usize = 20;
pub const MAX_PINNED: usize = 20;
/// Image bytes (PNG + thumbnail) the unpinned entries may hold, and the pinned ones.
pub const IMAGE_BUDGET: usize = 64_000_000;
/// Characters of text the panel shows.
pub const PREVIEW_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    ToPhone,
    FromPhone,
}

enum Content {
    Text(Zeroizing<String>),
    Image {
        thumbnail: Vec<u8>,
        /// None until a received image is complete.
        png: Option<Arc<Vec<u8>>>,
        width: u32,
        height: u32,
    },
}

struct Entry {
    id: u64,
    content: Content,
    direction: Direction,
    at_ms: u64,
    pinned: bool,
    hash: Option<[u8; 32]>,
}
impl Entry {
    fn image_bytes(&self) -> usize {
        match &self.content {
            Content::Text(_) => 0,
            Content::Image { thumbnail, png, .. } => {
                thumbnail.len() + png.as_ref().map_or(0, |p| p.len())
            }
        }
    }
}

/// What the panel shows for one entry.
#[derive(Clone, Debug, serde::Serialize)]
pub struct View {
    pub id: u64,
    pub kind: &'static str,
    pub direction: Direction,
    pub at: u64,
    pub pinned: bool,
    /// Text: one sanitised line, at most [`PREVIEW_CHARS`] characters.
    pub preview: Option<String>,
    /// Image: the thumbnail as a `data:image/png;base64,` URL.
    pub thumbnail: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Whether it can be copied (a received image: once complete).
    pub ready: bool,
}

/// An entry's content, for writing it back to the clipboard.
pub enum Item {
    Text(Zeroizing<String>),
    Image {
        png: Arc<Vec<u8>>,
        thumbnail: Vec<u8>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum PinError {
    Missing,
    /// A received image still arriving.
    NotReady,
    Full,
    TooLarge,
}

#[derive(Default)]
pub struct History {
    /// Newest first.
    entries: Vec<Entry>,
    next_id: u64,
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl History {
    fn add(
        &mut self,
        content: Content,
        hash: Option<[u8; 32]>,
        direction: Direction,
        at_ms: u64,
    ) -> u64 {
        if let Some(h) = hash {
            if let Some(i) = self.entries.iter().position(|e| e.hash == Some(h)) {
                if self.entries[i].pinned {
                    // Already kept at the top; nothing new to show.
                    return self.entries[i].id;
                }
                self.entries.remove(i);
            }
        }
        self.next_id += 1;
        let id = self.next_id;
        self.entries.insert(
            0,
            Entry {
                id,
                content,
                direction,
                at_ms,
                pinned: false,
                hash,
            },
        );
        self.trim(id);
        id
    }
    pub fn record_text(&mut self, text: &str, direction: Direction, at_ms: u64) -> u64 {
        let h = hash(text.as_bytes());
        self.add(
            Content::Text(Zeroizing::new(text.to_owned())),
            Some(h),
            direction,
            at_ms,
        )
    }
    /// An image; a received one may come without its PNG yet ([`History::complete_image`]).
    pub fn record_image(
        &mut self,
        thumbnail: Vec<u8>,
        png: Option<Arc<Vec<u8>>>,
        width: u32,
        height: u32,
        direction: Direction,
        at_ms: u64,
    ) -> u64 {
        let h = png.as_deref().map(|p| hash(p));
        self.add(
            Content::Image {
                thumbnail,
                png,
                width,
                height,
            },
            h,
            direction,
            at_ms,
        )
    }
    /// A received image is complete: its PNG, and an older copy of it goes.
    pub fn complete_image(&mut self, id: u64, full: Arc<Vec<u8>>) {
        let h = hash(&full);
        let Some(i) = self.entries.iter().position(|e| e.id == id) else {
            return;
        };
        if let Content::Image { png, .. } = &mut self.entries[i].content {
            *png = Some(full);
        }
        self.entries[i].hash = Some(h);
        if let Some(older) = self
            .entries
            .iter()
            .position(|e| e.id != id && e.hash == Some(h) && !e.pinned)
        {
            self.entries.remove(older);
        }
        self.trim(id);
    }
    /// Keeps the limits; never drops `keep` (the entry just added or completed).
    fn trim(&mut self, keep: u64) {
        while self.entries.iter().filter(|e| !e.pinned).count() > MAX_RECENT {
            let Some(i) = self.entries.iter().rposition(|e| !e.pinned && e.id != keep) else {
                break;
            };
            self.entries.remove(i);
        }
        loop {
            let bytes: usize = self
                .entries
                .iter()
                .filter(|e| !e.pinned)
                .map(Entry::image_bytes)
                .sum();
            if bytes <= IMAGE_BUDGET {
                break;
            }
            let Some(i) = self
                .entries
                .iter()
                .rposition(|e| !e.pinned && e.id != keep && e.image_bytes() > 0)
            else {
                break;
            };
            self.entries.remove(i);
        }
    }
    pub fn remove(&mut self, id: u64) -> Option<bool> {
        let i = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(i).pinned)
    }
    /// Removes the unpinned entries.
    pub fn clear(&mut self) {
        self.entries.retain(|e| e.pinned);
    }
    pub fn pin(&mut self, id: u64, pinned: bool) -> Result<(), PinError> {
        let i = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(PinError::Missing)?;
        if self.entries[i].pinned == pinned {
            return Ok(());
        }
        if pinned {
            if matches!(&self.entries[i].content, Content::Image { png: None, .. }) {
                return Err(PinError::NotReady);
            }
            let held: Vec<&Entry> = self.entries.iter().filter(|e| e.pinned).collect();
            if held.len() >= MAX_PINNED {
                return Err(PinError::Full);
            }
            let bytes: usize = held.iter().map(|e| e.image_bytes()).sum();
            if bytes + self.entries[i].image_bytes() > IMAGE_BUDGET {
                return Err(PinError::TooLarge);
            }
        }
        self.entries[i].pinned = pinned;
        if !pinned {
            self.trim(id);
        }
        Ok(())
    }
    pub fn get(&self, id: u64) -> Option<(Item, Direction)> {
        let entry = self.entries.iter().find(|e| e.id == id)?;
        let item = match &entry.content {
            Content::Text(text) => Item::Text(text.clone()),
            Content::Image {
                thumbnail,
                png,
                width,
                height,
            } => Item::Image {
                png: png.clone()?,
                thumbnail: thumbnail.clone(),
                width: *width,
                height: *height,
            },
        };
        Some((item, entry.direction))
    }
    /// Pinned first (newest first), then the recent ones.
    pub fn views(&self) -> Vec<View> {
        let order = self
            .entries
            .iter()
            .filter(|e| e.pinned)
            .chain(self.entries.iter().filter(|e| !e.pinned));
        order
            .map(|e| match &e.content {
                Content::Text(text) => View {
                    id: e.id,
                    kind: "text",
                    direction: e.direction,
                    at: e.at_ms,
                    pinned: e.pinned,
                    preview: Some(preview(text)),
                    thumbnail: None,
                    width: None,
                    height: None,
                    ready: true,
                },
                Content::Image {
                    thumbnail,
                    png,
                    width,
                    height,
                } => View {
                    id: e.id,
                    kind: "image",
                    direction: e.direction,
                    at: e.at_ms,
                    pinned: e.pinned,
                    preview: None,
                    thumbnail: Some(format!("data:image/png;base64,{}", base64(thumbnail))),
                    width: Some(*width),
                    height: Some(*height),
                    ready: png.is_some(),
                },
            })
            .collect()
    }

    /// The pins as plain bytes (to be encrypted): see [`PINS_FORMAT`].
    pub fn encode_pins(&self) -> Zeroizing<Vec<u8>> {
        let pinned: Vec<&Entry> = self.entries.iter().filter(|e| e.pinned).collect();
        let mut out = Zeroizing::new(Vec::new());
        out.extend_from_slice(&(pinned.len() as u32).to_le_bytes());
        // Oldest first, so loading in order rebuilds newest-first.
        for e in pinned.iter().rev() {
            match &e.content {
                Content::Text(text) => {
                    out.push(0);
                    out.push(e.direction as u8);
                    out.extend_from_slice(&e.at_ms.to_le_bytes());
                    put_bytes(&mut out, text.as_bytes());
                }
                Content::Image {
                    thumbnail,
                    png,
                    width,
                    height,
                } => {
                    let Some(png) = png else { continue };
                    out.push(1);
                    out.push(e.direction as u8);
                    out.extend_from_slice(&e.at_ms.to_le_bytes());
                    out.extend_from_slice(&width.to_le_bytes());
                    out.extend_from_slice(&height.to_le_bytes());
                    put_bytes(&mut out, thumbnail);
                    put_bytes(&mut out, png);
                }
            }
        }
        out
    }
    /// Restores pins written by [`History::encode_pins`]; anything malformed is an error and changes nothing.
    pub fn load_pins(&mut self, plain: &[u8]) -> Result<(), &'static str> {
        let mut r = Reader(plain);
        let count = r.u32()? as usize;
        if count > MAX_PINNED {
            return Err("too many pins");
        }
        let mut loaded = History {
            entries: Vec::new(),
            next_id: self.next_id,
        };
        for _ in 0..count {
            let kind = r.u8()?;
            let direction = match r.u8()? {
                0 => Direction::ToPhone,
                1 => Direction::FromPhone,
                _ => return Err("direction"),
            };
            let at_ms = r.u64()?;
            let id = match kind {
                0 => {
                    let bytes = r.bytes(wind_core::clipboard::MAX_TEXT)?;
                    let text = std::str::from_utf8(bytes).map_err(|_| "text")?;
                    loaded.record_text(text, direction, at_ms)
                }
                1 => {
                    let (width, height) = (r.u32()?, r.u32()?);
                    let thumbnail = r.bytes(wind_core::clipboard::MAX_THUMBNAIL)?.to_vec();
                    let png = r.bytes(wind_core::clipboard::MAX_IMAGE as usize)?.to_vec();
                    loaded.record_image(
                        thumbnail,
                        Some(Arc::new(png)),
                        width,
                        height,
                        direction,
                        at_ms,
                    )
                }
                _ => return Err("kind"),
            };
            loaded.pin(id, true).map_err(|_| "pin limits")?;
        }
        if !r.0.is_empty() {
            return Err("trailing bytes");
        }
        self.next_id = loaded.next_id;
        self.entries.retain(|e| !e.pinned);
        let mut entries = loaded.entries;
        entries.append(&mut self.entries);
        self.entries = entries;
        Ok(())
    }
}

/// Pins file: magic `WCPN`, format byte 1, then DPAPI (current user, own
/// purpose) over: u32 count, then per pin (oldest first) u8 kind (0 text,
/// 1 image), u8 direction, u64 time (ms); text: u32 length + UTF-8; image:
/// u32 width, u32 height, u32 length + thumbnail PNG, u32 length + PNG.
/// Little-endian.
pub const PINS_FORMAT: u8 = 1;
const PINS_MAGIC: &[u8; 4] = b"WCPN";
const PINS_PURPOSE: &[u8] = b"WindConnect clipboard pins v1";

/// Writes the pins (atomically: temporary file, then rename); no pins → the file goes.
pub fn save_pins(path: &std::path::Path, plain: &[u8]) -> std::io::Result<()> {
    if plain.len() <= 4 && plain.iter().all(|b| *b == 0) {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let blob = wind_core::identity::dpapi::protect_with(plain, PINS_PURPOSE)?;
    let mut file = Vec::with_capacity(blob.len() + 5);
    file.extend_from_slice(PINS_MAGIC);
    file.push(PINS_FORMAT);
    file.extend_from_slice(&blob);
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, &file)?;
    std::fs::rename(&temporary, path)
}

/// Reads the pins file's plain bytes: None if there is none. Another user's,
/// a corrupt or a newer file is an error; the caller starts without pins and
/// leaves the file as it is.
pub fn read_pins(path: &std::path::Path) -> std::io::Result<Option<Zeroizing<Vec<u8>>>> {
    let file = match std::fs::read(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let blob = file
        .strip_prefix(&PINS_MAGIC[..])
        .and_then(|rest| rest.strip_prefix(&[PINS_FORMAT]))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown pins format")
        })?;
    wind_core::identity::dpapi::unprotect_with(blob, PINS_PURPOSE).map(Some)
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], &'static str> {
        if self.0.len() < n {
            return Err("truncated");
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, &'static str> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self, max: usize) -> Result<&'a [u8], &'static str> {
        let n = self.u32()? as usize;
        if n == 0 || n > max {
            return Err("length");
        }
        self.take(n)
    }
}

/// Characters that would let a preview fake its look: bidi embeddings,
/// overrides and isolates, zero-width and direction marks.
fn deceptive(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}
/// One line for the panel: control, bidi and zero-width characters and runs
/// of whitespace become one space, at most [`PREVIEW_CHARS`] characters, "…" when cut.
pub fn preview(text: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    // Starting as if after a space drops leading blanks.
    let mut space = true;
    for c in text.chars() {
        let c = if c.is_control() || c.is_whitespace() || deceptive(c) {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if space {
                continue;
            }
            space = true;
        } else {
            space = false;
        }
        if count == PREVIEW_CHARS {
            out.truncate(out.trim_end().len());
            out.push('…');
            return out;
        }
        out.push(c);
        count += 1;
    }
    out.truncate(out.trim_end().len());
    out
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: usize, seed: u8) -> Arc<Vec<u8>> {
        Arc::new(vec![seed; bytes])
    }

    #[test]
    fn newest_first_twenty_recent_and_a_repeat_moves_up() {
        let mut h = History::default();
        let first = h.record_text("a", Direction::ToPhone, 1);
        for i in 0..20 {
            h.record_text(&format!("t{i}"), Direction::FromPhone, 2 + i);
        }
        let views = h.views();
        assert_eq!(views.len(), MAX_RECENT);
        assert!(views.iter().all(|v| v.id != first), "the oldest went");
        assert_eq!(views[0].preview.as_deref(), Some("t19"));
        // A repeat: one entry, on top, with its new direction.
        h.record_text("t5", Direction::ToPhone, 100);
        let views = h.views();
        assert_eq!(views.len(), MAX_RECENT);
        assert_eq!(views[0].preview.as_deref(), Some("t5"));
        assert_eq!(views[0].direction, Direction::ToPhone);
        assert_eq!(
            views
                .iter()
                .filter(|v| v.preview.as_deref() == Some("t5"))
                .count(),
            1
        );
    }

    #[test]
    fn pins_stay_on_top_and_outside_the_twenty() {
        let mut h = History::default();
        let pinned = h.record_text("keep", Direction::ToPhone, 1);
        h.pin(pinned, true).unwrap();
        for i in 0..25 {
            h.record_text(&format!("t{i}"), Direction::FromPhone, 2 + i);
        }
        let views = h.views();
        assert_eq!(views.len(), MAX_RECENT + 1);
        assert_eq!(views[0].id, pinned);
        assert!(views[0].pinned);
        // Repeating a pinned text keeps the one pinned entry.
        assert_eq!(h.record_text("keep", Direction::FromPhone, 99), pinned);
        assert_eq!(h.views().len(), MAX_RECENT + 1);
        h.clear();
        assert_eq!(h.views().len(), 1, "clear keeps pins");
        h.pin(pinned, false).unwrap();
        assert_eq!(h.remove(pinned), Some(false));
        assert!(h.views().is_empty());
    }

    #[test]
    fn pin_limits() {
        let mut h = History::default();
        for i in 0..MAX_PINNED as u64 {
            let id = h.record_text(&format!("p{i}"), Direction::ToPhone, i);
            h.pin(id, true).unwrap();
        }
        let one_more = h.record_text("one more", Direction::ToPhone, 99);
        assert_eq!(h.pin(one_more, true), Err(PinError::Full));
        assert_eq!(h.pin(12345, true), Err(PinError::Missing));
        // Images: within the pinned budget only, and only once complete.
        let mut h = History::default();
        let big = h.record_image(
            vec![1],
            Some(image(40_000_000, 1)),
            10,
            10,
            Direction::ToPhone,
            1,
        );
        h.pin(big, true).unwrap();
        let other = h.record_image(
            vec![2],
            Some(image(30_000_000, 2)),
            10,
            10,
            Direction::ToPhone,
            2,
        );
        assert_eq!(h.pin(other, true), Err(PinError::TooLarge));
        let arriving = h.record_image(vec![3], None, 10, 10, Direction::FromPhone, 3);
        assert_eq!(h.pin(arriving, true), Err(PinError::NotReady));
        assert!(h.get(arriving).is_none(), "not copyable until complete");
        assert!(!h.views().iter().find(|v| v.id == arriving).unwrap().ready);
    }

    #[test]
    fn unpinned_images_share_a_memory_budget() {
        let mut h = History::default();
        let a = h.record_image(
            vec![1],
            Some(image(30_000_000, 1)),
            1,
            1,
            Direction::ToPhone,
            1,
        );
        let b = h.record_image(
            vec![2],
            Some(image(30_000_000, 2)),
            1,
            1,
            Direction::ToPhone,
            2,
        );
        let text = h.record_text("text", Direction::ToPhone, 3);
        let c = h.record_image(vec![3], None, 1, 1, Direction::FromPhone, 4);
        h.complete_image(c, image(30_000_000, 3));
        let ids: Vec<u64> = h.views().iter().map(|v| v.id).collect();
        assert_eq!(
            ids,
            vec![c, text, b],
            "the oldest image went, text stays: {a}"
        );
        // A completed image equal to an older one replaces it.
        let d = h.record_image(vec![4], None, 1, 1, Direction::FromPhone, 5);
        h.complete_image(d, image(30_000_000, 2));
        let ids: Vec<u64> = h.views().iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![d, c, text]);
    }

    #[test]
    fn pins_encode_and_load_and_reject_garbage() {
        let mut h = History::default();
        let t = h.record_text("zażółć\ngęślą", Direction::FromPhone, 7);
        let i = h.record_image(vec![9, 8], Some(image(10, 5)), 3, 4, Direction::ToPhone, 8);
        h.record_text("not pinned", Direction::ToPhone, 9);
        h.pin(t, true).unwrap();
        h.pin(i, true).unwrap();
        let plain = h.encode_pins();

        let mut loaded = History::default();
        loaded.record_text("recent", Direction::ToPhone, 1);
        loaded.load_pins(&plain).unwrap();
        let views = loaded.views();
        assert_eq!(views.len(), 3);
        assert!(views[0].pinned && views[1].pinned && !views[2].pinned);
        assert_eq!(views[0].kind, "image");
        assert_eq!(
            (views[0].width, views[0].height, views[0].at),
            (Some(3), Some(4), 8)
        );
        assert_eq!(views[1].preview.as_deref(), Some("zażółć gęślą"));
        assert_eq!(views[1].direction, Direction::FromPhone);
        let Some((Item::Text(text), Direction::FromPhone)) = loaded.get(views[1].id) else {
            panic!()
        };
        assert_eq!(text.as_str(), "zażółć\ngęślą");
        let Some((Item::Image { png, thumbnail, .. }, _)) = loaded.get(views[0].id) else {
            panic!()
        };
        assert_eq!((png.len(), thumbnail), (10, vec![9, 8]));

        for bad in [
            &plain[..plain.len() - 1],
            &[],
            &[1, 0, 0, 0, 7][..],
            &[0xff, 0, 0, 0][..],
        ] {
            let mut h = History::default();
            assert!(h.load_pins(bad).is_err());
            assert!(h.views().is_empty());
        }
        let mut extra = plain.to_vec();
        extra.push(0);
        assert!(History::default().load_pins(&extra).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn pins_file_is_encrypted_and_bound_to_its_purpose() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard-pins.bin");
        let mut h = History::default();
        let t = h.record_text("hasło-nie-tu WN37 pin", Direction::ToPhone, 1);
        h.pin(t, true).unwrap();
        save_pins(&path, &h.encode_pins()).unwrap();
        let file = std::fs::read(&path).unwrap();
        assert!(file.starts_with(b"WCPN\x01"));
        assert!(!file.windows(4).any(|w| w == b"WN37"), "plain text on disk");
        let plain = read_pins(&path).unwrap().unwrap();
        let mut loaded = History::default();
        loaded.load_pins(&plain).unwrap();
        assert_eq!(
            loaded.views()[0].preview.as_deref(),
            Some("hasło-nie-tu WN37 pin")
        );
        // The identity key's purpose can't open it, nor can a broken file be read.
        assert!(wind_core::identity::dpapi::unprotect(&file[5..]).is_err());
        std::fs::write(&path, b"WCPN\x01garbage").unwrap();
        assert!(read_pins(&path).is_err());
        std::fs::write(&path, b"WCPN\x02").unwrap();
        assert!(read_pins(&path).is_err());
        // No pins: the file goes.
        save_pins(&path, &History::default().encode_pins()).unwrap();
        assert!(!path.exists());
        assert!(read_pins(&path).unwrap().is_none());
    }

    #[test]
    fn preview_is_one_short_line() {
        assert_eq!(preview("  a\r\n\tb\u{0}c  "), "a b c");
        // Bidi overrides, isolates, zero-width: can't make "exe.txt" look like "txt.exe".
        assert_eq!(
            preview("\u{202E}a\u{202E}txt.exe\u{202C}b\u{200B}c\u{2066}d\u{FEFF}"),
            "a txt.exe b c d"
        );
        let long = "x".repeat(200);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS + 1);
        assert!(p.ends_with('…'));
        assert_eq!(
            preview(&"y".repeat(PREVIEW_CHARS)),
            "y".repeat(PREVIEW_CHARS)
        );
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }
}
