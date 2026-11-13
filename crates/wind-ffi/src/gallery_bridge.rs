//! CCS/1 M:WN-58 the phone's photos for the PC's panel (gallery/1, §9.13): Kotlin reads MediaStore through
//! [`GallerySource`], the core answers the PC's `gallery` streams. RULE:`gallery/1` is announced only while the
//! person's setting is on and photo access is granted (Kotlin decides, [`crate::CoreClient::set_gallery_enabled`]).
//! An original comes as a file descriptor Kotlin detached for us: the core owns and closes it.
use std::sync::Arc;

use wind_core::gallery::{self, FileInfo, Item, Opened, Status};

#[derive(uniffi::Record, Clone, Debug)]
pub struct GalleryItem {
    pub id: u64,
    pub date_ms: u64,
    pub name: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub size: u64,
    pub screenshot: bool,
}

/// `status`: 0 ok, 1 denied (no access or turned off), 2 not found, 3 busy, 4 failed.
#[derive(uniffi::Record, Clone, Debug)]
pub struct GalleryItems {
    pub status: u8,
    pub items: Vec<GalleryItem>,
}

/// A JPEG of at most 64 KiB.
#[derive(uniffi::Record, Clone, Debug)]
pub struct GalleryThumb {
    pub status: u8,
    pub jpeg: Vec<u8>,
}

/// An opened original: `fd` is a detached, readable file descriptor of exactly `size` bytes (ownership passes to
/// the core); -1 with any status but 0.
#[derive(uniffi::Record, Clone, Debug)]
pub struct GalleryOriginal {
    pub status: u8,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub fd: i32,
}

/// WN-58: the phone's photos, called off the UI thread (may block on the content provider). Never throws: a failure
/// is status 4. Never re-enter the core from it.
#[uniffi::export(callback_interface)]
pub trait GallerySource: Send + Sync {
    /// Newest first, photos and screenshots, at most `limit`.
    fn recent(&self, limit: u32) -> GalleryItems;
    /// Its longer side at most `size` px.
    fn thumb(&self, id: u64, size: u32) -> GalleryThumb;
    fn original(&self, id: u64) -> GalleryOriginal;
}

pub(crate) struct PlatformGallery(pub Box<dyn GallerySource>);

fn result<T>(code: u8, value: T) -> Result<T, Status> {
    match Status::from_code(code.into()) {
        Status::Ok => Ok(value),
        s => Err(s),
    }
}

#[cfg(unix)]
fn reader(fd: i32) -> Option<Box<dyn std::io::Read + Send>> {
    use std::os::fd::FromRawFd;
    // SAFETY: Kotlin detached this descriptor for us (ParcelFileDescriptor.detachFd) and keeps no other owner.
    (fd >= 0).then(|| Box::new(unsafe { std::fs::File::from_raw_fd(fd) }) as Box<_>)
}

#[cfg(not(unix))]
fn reader(_fd: i32) -> Option<Box<dyn std::io::Read + Send>> {
    None
}

impl gallery::Source for PlatformGallery {
    fn recent(&self, limit: u32) -> Result<Vec<Item>, Status> {
        let answer = self.0.recent(limit);
        result(answer.status, ())?;
        Ok(answer
            .items
            .into_iter()
            .map(|i| Item {
                id: i.id,
                date_ms: i.date_ms,
                name: i.name,
                mime: i.mime,
                width: i.width,
                height: i.height,
                size: i.size,
                screenshot: i.screenshot,
            })
            .collect())
    }
    fn thumb(&self, id: u64, size: u32) -> Result<Vec<u8>, Status> {
        let answer = self.0.thumb(id, size);
        result(answer.status, answer.jpeg)
    }
    fn original(&self, id: u64) -> Result<Opened, Status> {
        let answer = self.0.original(id);
        // Take the descriptor first, so an error status never leaks it.
        let bytes = reader(answer.fd);
        result(answer.status, ())?;
        Ok(Opened {
            info: FileInfo {
                name: answer.name,
                mime: answer.mime,
                size: answer.size,
            },
            bytes: bytes.ok_or(Status::Failed)?,
        })
    }
}

pub(crate) type GalleryRoute = (
    Arc<gallery::Server>,
    tokio::sync::mpsc::Receiver<wind_core::session::GalleryStream>,
);
