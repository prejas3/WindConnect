//! CCS/1 M:wind-ffi | PURPOSE:UniFFI boundary for Android
//! SEC:identity storage errors never generate replacement keys; explicit pinning, mTLS.

use std::{fmt, io, sync::Arc, time::Duration};
use wind_core::{
    identity::{DeviceIdentity, KeyStore, PublicKey},
    network::{DiscoveryOptions, DiscoverySnapshot, NetworkChanges, NetworkDiscovery},
    protocol::{DeviceKind, ErrorCode},
    relay::RelayLink,
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
use zeroize::{Zeroize, Zeroizing};

uniffi::setup_scaffolding!();
mod file_transfer;
mod gallery_bridge;
mod phone_actions;
mod phone_extras;
mod sms_bridge;
mod touchpad;
pub use file_transfer::{
    FileCommitResult, FileCommitStatus, FileDestination, FileTransfer, IncomingControl,
    IncomingFile,
};
pub use gallery_bridge::{GalleryItem, GalleryItems, GalleryOriginal, GallerySource, GalleryThumb};
pub use phone_actions::UrlOpener;
pub use phone_extras::{CallActionSink, CallerInfo, MissedCall, VolumeSink};
pub use sms_bridge::{
    SmsMessage, SmsMessages, SmsOutgoing, SmsPart, SmsPartInfo, SmsPerson, SmsPhoto, SmsSim,
    SmsSims, SmsSource, SmsThread, SmsThreads,
};
pub use touchpad::{TouchpadCapabilities, TouchpadCommand, TouchpadSession};

/// With the relay on, the address gets this long first (a LAN answers in
/// milliseconds; the relay is the fallback).
const LAN_FIRST: Duration = Duration::from_secs(2);

#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    InvalidInput { reason: String },
    Storage { reason: String },
    Transport { reason: String },
    UntrustedPeer { reason: String },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::InvalidInput { reason }
            | Self::Storage { reason }
            | Self::Transport { reason }
            | Self::UntrustedPeer { reason } => reason,
        };
        f.write_str(reason)
    }
}

impl std::error::Error for CoreError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for CoreError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Storage {
            reason: error.to_string(),
        }
    }
}

/// CCS/1 IO:encrypted platform store <-> PKCS#8 PEM bytes
/// RULE:store_new is atomic create-only; false means load the race winner.
#[uniffi::export(callback_interface)]
pub trait IdentityStorage: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>, CoreError>;
    fn store_new(&self, pem: Vec<u8>) -> Result<bool, CoreError>;
}

struct PlatformStore(Box<dyn IdentityStorage>);

/// WN-50 (§9.5): the phone's ring. `ring` starts the sound and returns true once it is playing
/// (it may block briefly); `stop` stops it. Called off the UI thread; never re-enter the core from them.
#[uniffi::export(callback_interface)]
pub trait FindSink: Send + Sync {
    fn ring(&self) -> bool;
    fn stop(&self);
}

/// WN-46 (§9.6): a command from the PC for the phone's player, already checked against the
/// actions the last `send_media` allowed. `position_ms` only with "seek".
#[uniffi::export(callback_interface)]
pub trait MediaSink: Send + Sync {
    fn command(&self, action: String, position_ms: Option<u64>);
}

/// The phone's player as the PC shows it (§9.6 `Now`).
#[derive(uniffi::Record, Clone, Debug)]
pub struct MediaNow {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub app: String,
    pub duration_ms: Option<u64>,
    pub position_ms: u64,
    pub playing: bool,
    pub speed_permille: u64,
    /// Bits: 1 play, 2 pause, 4 next, 8 previous, 16 seek.
    pub actions: u64,
}

/// A notification on the phone, normalised (§9.7 `Posted`; WN-42).
#[derive(uniffi::Record, Clone, Debug)]
pub struct PhoneNotification {
    /// 16 bytes, stable per notification (a salted hash of its key).
    pub id: Vec<u8>,
    pub app: String,
    pub package_name: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub time_ms: u64,
    /// It shows a progress bar: only its start and end go out.
    pub progress: bool,
    /// The label of its reply action, when it can be replied to (WN-44).
    pub reply: Option<String>,
    /// It doesn't interrupt on the phone (Do not disturb): quiet on the PC (WN-45).
    pub quiet: bool,
}

/// WN-44 (§9.8): the PC's dismissals and replies for notifications this phone sent in the session. The phone checks the id
/// itself (only its own, from this session); a reply's outcome goes back with [`CoreConnection::notification_replied`].
#[uniffi::export(callback_interface)]
pub trait NotifyActionSink: Send + Sync {
    fn dismiss(&self, id: Vec<u8>);
    fn reply(&self, id: Vec<u8>, text: String);
}

async fn serve_notify_actions(
    sink: Arc<dyn NotifyActionSink>,
    mut queue: tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
) {
    while let Some(message) = queue.recv().await {
        let Some(action) = wind_core::notify::Action::decode(&message.message) else {
            continue;
        };
        let sink = sink.clone();
        // A platform callback may block briefly; off the runtime's workers.
        let _ = tokio::task::spawn_blocking(move || match action {
            wind_core::notify::Action::Dismiss { id } => sink.dismiss(id.to_vec()),
            wind_core::notify::Action::Reply { id, text } => sink.reply(id.to_vec(), text),
        })
        .await;
    }
}

type NotifyActionRoute = (
    Arc<dyn NotifyActionSink>,
    tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
);

fn notification_id(id: &[u8]) -> Result<[u8; 16], CoreError> {
    id.try_into().map_err(|_| CoreError::InvalidInput {
        reason: format!("notification id of {} bytes, not 16", id.len()),
    })
}

fn action_name(action: wind_core::media::Action) -> (&'static str, Option<u64>) {
    use wind_core::media::Action;
    match action {
        Action::Play => ("play", None),
        Action::Pause => ("pause", None),
        Action::Toggle => ("toggle", None),
        Action::Next => ("next", None),
        Action::Previous => ("previous", None),
        Action::Seek(at) => ("seek", Some(at)),
    }
}

/// Hands the PC's commands to the sink, dropping what the last sent `Now` didn't allow.
async fn serve_media(
    sink: Arc<dyn MediaSink>,
    allowed: Arc<std::sync::atomic::AtomicU64>,
    mut queue: tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
) {
    while let Some(message) = queue.recv().await {
        let Some(action) = wind_core::media::Action::decode(&message.message) else {
            continue;
        };
        if !action.allowed_by(allowed.load(std::sync::atomic::Ordering::SeqCst)) {
            continue;
        }
        let sink = sink.clone();
        let (name, position) = action_name(action);
        // A platform callback may block briefly; off the runtime's workers.
        let _ = tokio::task::spawn_blocking(move || sink.command(name.into(), position)).await;
    }
}

/// WN-47 (§9.6): the PC's player for the phone. `art` is the image a `Now` points at (JPEG/PNG,
/// checked by magic bytes), resolved here by its key; texts are the PC's, still to be shown as text only.
#[uniffi::export(callback_interface)]
pub trait PcMediaSink: Send + Sync {
    fn now(&self, now: MediaNow, art: Option<Vec<u8>>);
    fn gone(&self);
}

/// Folds the PC's Now/Art/Gone for the sink; ends with the session (a last `gone`).
async fn serve_pc_media(
    sink: Arc<dyn PcMediaSink>,
    mut queue: tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
) {
    use wind_core::media::Message;
    let mut art: Option<([u8; 8], Vec<u8>)> = None;
    while let Some(message) = queue.recv().await {
        let sink = sink.clone();
        match Message::decode(&message.message) {
            Some(Message::Art { key, image }) => art = Some((key, image)),
            Some(Message::Now(n)) => {
                let image = n.art.and_then(|k| {
                    art.as_ref()
                        .filter(|(key, _)| *key == k)
                        .map(|(_, i)| i.clone())
                });
                let now = MediaNow {
                    title: n.title,
                    artist: n.artist,
                    album: n.album,
                    app: n.app,
                    duration_ms: n.duration_ms,
                    position_ms: n.position_ms,
                    playing: n.playing,
                    speed_permille: n.speed_permille,
                    actions: n.actions,
                };
                let _ = tokio::task::spawn_blocking(move || sink.now(now, image)).await;
            }
            Some(Message::Gone) => {
                let _ = tokio::task::spawn_blocking(move || sink.gone()).await;
            }
            None => {}
        }
    }
    let _ = tokio::task::spawn_blocking(move || sink.gone()).await;
}

type PcMediaRoute = (
    Arc<dyn PcMediaSink>,
    tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
);

type MediaRoute = (
    Arc<dyn MediaSink>,
    Arc<std::sync::atomic::AtomicU64>,
    tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>,
);

struct PlatformFind(Box<dyn FindSink>);
impl wind_core::find::Sink for PlatformFind {
    fn ring(&self) -> bool {
        self.0.ring()
    }
    fn stop(&self) {
        self.0.stop()
    }
}

/// CCS/1 RULE:callback writes OS clipboard synchronously; true only after success. Never re-enter clipboard send synchronously.
#[uniffi::export(callback_interface)]
pub trait ClipboardSink: Send + Sync {
    fn apply_text(&self, text: String) -> bool;
    /// WN-36: put `image` on the OS clipboard now (pasteable); its PNG comes
    /// through `ClipboardImage::read`. True only once the OS took it.
    fn apply_image(&self, image: Arc<ClipboardImage>) -> bool;
}
struct PlatformClipboard(Box<dyn ClipboardSink>);
impl wind_core::clipboard::Sink for PlatformClipboard {
    fn apply(&self, text: &str) -> bool {
        self.0.apply_text(text.to_owned())
    }
    fn apply_image(&self, image: Arc<wind_core::clipboard::IncomingImage>) -> bool {
        self.0.apply_image(Arc::new(ClipboardImage(image)))
    }
}

/// A received clipboard image (WN-36): the thumbnail now, the PNG as it
/// arrives. Memory only; a newer clipboard item fails an unfinished one.
#[derive(uniffi::Object)]
pub struct ClipboardImage(Arc<wind_core::clipboard::IncomingImage>);

#[uniffi::export]
impl ClipboardImage {
    /// Hex of its 16-byte id (names the content URI).
    pub fn id(&self) -> String {
        self.0.id().iter().map(|b| format!("{b:02x}")).collect()
    }
    pub fn size(&self) -> u64 {
        self.0.size()
    }
    pub fn width(&self) -> u64 {
        self.0.width()
    }
    pub fn height(&self) -> u64 {
        self.0.height()
    }
    /// The thumbnail PNG: for previews only, never pasted as the image.
    pub fn thumbnail(&self) -> Vec<u8> {
        self.0.thumbnail().to_vec()
    }
    pub fn is_complete(&self) -> bool {
        self.0.is_complete()
    }
    /// Blocks until bytes from `offset` are there (up to `max`); empty at
    /// the end. Fails when replaced, broken off, or after 10 s with no bytes.
    /// CCS/1 RULE:call from a worker thread (a paste's pipe), never the main thread.
    pub fn read(&self, offset: u64, max: u32) -> Result<Vec<u8>, CoreError> {
        self.0
            .read_at(offset, max as usize)
            .map_err(|e| CoreError::Transport {
                reason: match e {
                    wind_core::clipboard::ImageError::Failed => "image replaced or broken off",
                    wind_core::clipboard::ImageError::Stalled => "image transfer stalled",
                }
                .into(),
            })
    }
}

/// Told once the peer has the image on its clipboard (pasteable), before
/// the whole PNG is there.
#[uniffi::export(callback_interface)]
pub trait ClipboardImageProgress: Send + Sync {
    fn applied(&self);
}

/// How a clipboard image send ended: `status` as for text, plus
/// "too-large", "peer-limit" (with `peer_limit` bytes), "unsupported",
/// "replaced".
#[derive(uniffi::Record)]
pub struct ClipboardSendResult {
    pub status: String,
    pub peer_limit: Option<u64>,
}

impl KeyStore for PlatformStore {
    fn load(&self) -> io::Result<Option<Zeroizing<String>>> {
        let Some(mut bytes) = self.0.load().map_err(io::Error::other)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map(|s| Zeroizing::new(s.to_owned()));
        bytes.zeroize();
        text.map(Some)
            .map_err(|_| io::Error::other("identity PEM is not UTF-8"))
    }

    fn store_new(&self, pem: &str) -> io::Result<bool> {
        self.0
            .store_new(pem.as_bytes().to_vec())
            .map_err(io::Error::other)
    }
}

fn public_key(bytes: &[u8]) -> Result<PublicKey, CoreError> {
    let bytes = bytes.try_into().map_err(|_| CoreError::InvalidInput {
        reason: "Ed25519 public key must contain 32 bytes".into(),
    })?;
    PublicKey::from_bytes(bytes).ok_or_else(|| CoreError::InvalidInput {
        reason: "invalid Ed25519 public key".into(),
    })
}

fn session_error(error: wind_core::session::SessionError) -> CoreError {
    if error.is_untrusted_peer() {
        CoreError::UntrustedPeer {
            reason: error.to_string(),
        }
    } else {
        CoreError::Transport {
            reason: error.to_string(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct IdentityInfo {
    pub public_key: Vec<u8>,
    pub fingerprint: String,
    pub display_fingerprint: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PeerInfo {
    pub public_key: Vec<u8>,
    pub name: String,
    pub fingerprint: String,
}

/// A row of the paired-device list (WC-28).
#[derive(uniffi::Record, Clone, Debug)]
pub struct PairedEntry {
    pub public_key: Vec<u8>,
    /// The local name (alias), or the name it gave when pairing.
    pub name: String,
    pub fingerprint: String,
    /// Seconds since the Unix epoch: when paired, or (when `owed`) removed.
    pub paired_at: u64,
    /// Removed here; its device is still owed the unpair notice (§12.10),
    /// delivered by [`CoreClient::deliver_notice`] when it is seen.
    pub owed: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PairedLoadResult {
    pub devices: Vec<PeerInfo>,
    pub skipped_records: u64,
}

#[derive(uniffi::Object)]
pub struct CoreClient {
    files: Arc<file_transfer::FileBridge>,
    clipboard: Arc<wind_core::clipboard::Clipboard>,
    /// Signs pairing and relay authentication (PROTOCOL.md §13.2).
    device_identity: Arc<DeviceIdentity>,
    endpoint: Endpoint,
    /// Relay base URL; `None` (the default, D3) never touches the relay.
    relay: std::sync::Mutex<Option<String>>,
    trust: Arc<PinnedKeys>,
    identity: IdentityInfo,
    local: LocalInfo,
    bind_ip: std::net::IpAddr,
    /// The port the platform asked for; 0 lets the system pick.
    bind_port: u16,
    network: Arc<NetworkChanges>,
    discovery: tokio::sync::Mutex<Option<NetworkDiscovery>>,
    discovered: tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<DiscoverySnapshot>>>,
    /// WN-50: one ringer for the client, so a ring outlives its session.
    ringer: std::sync::Mutex<Option<Arc<wind_core::find::Ringer>>>,
    /// The person allows the computer to ring (announce `find/1`).
    find_enabled: std::sync::atomic::AtomicBool,
    /// WN-46: the phone's player for the PC's commands, and whether it may be controlled.
    media: std::sync::Mutex<Option<Arc<dyn MediaSink>>>,
    media_enabled: std::sync::atomic::AtomicBool,
    /// WN-47: the PC's player shown here, and whether the person wants it.
    pc_media: std::sync::Mutex<Option<Arc<dyn PcMediaSink>>>,
    pc_media_enabled: std::sync::atomic::AtomicBool,
    /// WN-44: the PC's dismissals and replies, and whether the phone takes them (notifications on, access granted).
    notify_actions: std::sync::Mutex<Option<Arc<dyn NotifyActionSink>>>,
    notify_actions_enabled: std::sync::atomic::AtomicBool,
    /// WN-48: the PC's volume and its requests for the phone's; whether the phone takes them ("Głośność z komputera").
    volume: std::sync::Mutex<Option<Arc<dyn VolumeSink>>>,
    volume_enabled: std::sync::atomic::AtomicBool,
    /// WN-100: where the PC's links open, and "Otwieraj linki z komputera" (read for every request).
    url_opener: std::sync::Mutex<Option<Arc<dyn UrlOpener>>>,
    open_links: Arc<std::sync::atomic::AtomicBool>,
    /// WN-57: the PC's Reject/Silence; whether the phone takes them ("Połączenia na komputerze").
    call_actions: std::sync::Mutex<Option<Arc<dyn CallActionSink>>>,
    call_actions_enabled: std::sync::atomic::AtomicBool,
    /// WN-55: one SMS server for the client (its watches end with each session).
    sms: std::sync::Mutex<Option<Arc<wind_core::sms::Server>>>,
    /// The person's setting and READ_SMS (announce `sms/1`).
    sms_enabled: std::sync::atomic::AtomicBool,
    /// WN-58: one gallery server for the client (its watches end with each session).
    gallery: std::sync::Mutex<Option<Arc<wind_core::gallery::Server>>>,
    /// The person's setting and photo access (announce `gallery/1`).
    gallery_enabled: std::sync::atomic::AtomicBool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DiscoveredDevice {
    pub fingerprint: String,
    pub addresses: Vec<String>,
    /// TXT `pair` (§12.4): the SAS pairing port while its window is open.
    pub pairing_port: Option<u16>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct NetworkSnapshot {
    pub generation: u64,
    pub peers: Vec<DiscoveredDevice>,
    pub error: Option<String>,
}

/// CCS/1 FLOW:Kotlin IO coroutine -> load/create identity -> Tokio QUIC endpoint
/// DEC:UniFFI's Tokio compatibility runtime drives every exported async operation.
#[uniffi::export(async_runtime = "tokio")]
pub async fn open_client(
    storage: Box<dyn IdentityStorage>,
    name: String,
    bind_address: String,
) -> Result<Arc<CoreClient>, CoreError> {
    if name.is_empty() || name.len() > 64 {
        return Err(CoreError::InvalidInput {
            reason: "device name must be 1–64 UTF-8 bytes".into(),
        });
    }
    let address: std::net::SocketAddr =
        bind_address.parse().map_err(|_| CoreError::InvalidInput {
            reason: "bind address must be an IP address and port".into(),
        })?;
    let identity = DeviceIdentity::load_or_create(&PlatformStore(storage)).map_err(|e| {
        CoreError::Storage {
            reason: e.to_string(),
        }
    })?;
    let trust = Arc::new(PinnedKeys::new());
    let endpoint =
        Endpoint::bind(address, &identity, trust.clone()).map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })?;
    Ok(Arc::new(CoreClient {
        files: Arc::new(file_transfer::FileBridge::default()),
        clipboard: Arc::new(wind_core::clipboard::Clipboard::default()),
        endpoint,
        relay: std::sync::Mutex::new(None),
        trust,
        bind_ip: address.ip(),
        bind_port: address.port(),
        network: Arc::new(NetworkChanges::default()),
        discovery: tokio::sync::Mutex::new(None),
        discovered: tokio::sync::Mutex::new(None),
        ringer: std::sync::Mutex::new(None),
        find_enabled: std::sync::atomic::AtomicBool::new(true),
        media: std::sync::Mutex::new(None),
        media_enabled: std::sync::atomic::AtomicBool::new(false),
        pc_media: std::sync::Mutex::new(None),
        pc_media_enabled: std::sync::atomic::AtomicBool::new(true),
        notify_actions: std::sync::Mutex::new(None),
        notify_actions_enabled: std::sync::atomic::AtomicBool::new(false),
        volume: std::sync::Mutex::new(None),
        volume_enabled: std::sync::atomic::AtomicBool::new(false),
        url_opener: std::sync::Mutex::new(None),
        open_links: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        call_actions: std::sync::Mutex::new(None),
        call_actions_enabled: std::sync::atomic::AtomicBool::new(false),
        sms: std::sync::Mutex::new(None),
        sms_enabled: std::sync::atomic::AtomicBool::new(false),
        gallery: std::sync::Mutex::new(None),
        gallery_enabled: std::sync::atomic::AtomicBool::new(false),
        identity: IdentityInfo {
            public_key: identity.public_key().as_bytes().to_vec(),
            fingerprint: identity.fingerprint().to_hex(),
            display_fingerprint: identity.fingerprint().to_string(),
        },
        local: LocalInfo {
            name,
            kind: DeviceKind::Phone,
            app: Some(wind_core::VERSION.into()),
        },
        device_identity: Arc::new(identity),
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreClient {
    pub fn set_clipboard_sink(&self, sink: Box<dyn ClipboardSink>) {
        self.clipboard.set_sink(Arc::new(PlatformClipboard(sink)));
    }
    /// WN-50: the phone can ring; without a sink `find/1` is never announced.
    pub fn set_find_sink(&self, sink: Box<dyn FindSink>) {
        *self.ringer.lock().unwrap() =
            Some(wind_core::find::Ringer::new(Arc::new(PlatformFind(sink))));
    }
    /// Whether new sessions announce `find/1`; a live one changes with
    /// [`CoreConnection::set_find_accepted`].
    pub fn set_find_enabled(&self, enabled: bool) {
        self.find_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-55: the phone's SMS store; without it `sms/1` is never announced.
    pub fn set_sms_source(&self, source: Box<dyn SmsSource>) {
        *self.sms.lock().unwrap() = Some(wind_core::sms::Server::new(Arc::new(
            sms_bridge::PlatformSms(source),
        )));
    }
    /// Whether new sessions announce `sms/1` (the setting on and READ_SMS granted); a live one changes with
    /// [`CoreConnection::set_sms_accepted`].
    pub fn set_sms_enabled(&self, enabled: bool) {
        self.sms_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-56: a send's state from the platform (sent and delivery reports): 0 queued, 1 sent, 2 delivered, 3 failed.
    pub fn sms_send_state(&self, token: u64, state: u8) {
        if let Some(server) = self.sms.lock().unwrap().clone() {
            server.send_state(token, sms_bridge::send_state(state));
        }
    }
    /// SMS changed on the phone (a content observer): the PC's watches get `Changed`.
    pub fn sms_changed(&self) {
        if let Some(server) = self.sms.lock().unwrap().clone() {
            server.changed();
        }
    }
    /// WN-58: the phone's photos; without them `gallery/1` is never announced.
    pub fn set_gallery_source(&self, source: Box<dyn GallerySource>) {
        *self.gallery.lock().unwrap() = Some(wind_core::gallery::Server::new(Arc::new(
            gallery_bridge::PlatformGallery(source),
        )));
    }
    /// Whether new sessions announce `gallery/1` (the setting on and photo access granted); a live one changes
    /// with [`CoreConnection::set_gallery_accepted`].
    pub fn set_gallery_enabled(&self, enabled: bool) {
        self.gallery_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// Photos changed on the phone (a MediaStore observer): the PC's watches get `Changed`.
    pub fn gallery_changed(&self) {
        if let Some(server) = self.gallery.lock().unwrap().clone() {
            server.changed();
        }
    }
    /// The person stopped the ring on the phone (the PC gets state 1).
    pub fn stop_ringing(&self) {
        if let Some(ringer) = self.ringer.lock().unwrap().clone() {
            ringer.stop_here();
        }
    }
    /// WN-46: where the PC's media commands go.
    pub fn set_media_sink(&self, sink: Box<dyn MediaSink>) {
        *self.media.lock().unwrap() = Some(Arc::from(sink));
    }
    /// Whether new sessions announce `media-control/1` (access granted and the setting on); a live
    /// one changes with [`CoreConnection::set_media_control_accepted`].
    pub fn set_media_enabled(&self, enabled: bool) {
        self.media_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-48: where the PC's volume and its requests go.
    pub fn set_volume_sink(&self, sink: Box<dyn VolumeSink>) {
        *self.volume.lock().unwrap() = Some(Arc::from(sink));
    }
    /// Whether new sessions announce `volume/1`; a live one changes with [`CoreConnection::set_volume_accepted`].
    pub fn set_volume_enabled(&self, enabled: bool) {
        self.volume_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-100: where links from the PC open; without it the phone announces no `actions/1`.
    pub fn set_url_opener(&self, opener: Box<dyn UrlOpener>) {
        *self.url_opener.lock().unwrap() = Some(Arc::from(opener));
    }
    /// "Otwieraj linki z komputera": off, the PC's `open-url` is refused (status 1), read for every request.
    pub fn set_open_links(&self, allowed: bool) {
        self.open_links
            .store(allowed, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-57: where the PC's Reject/Silence go; without it `call-action/1` is never announced.
    pub fn set_call_action_sink(&self, sink: Box<dyn CallActionSink>) {
        *self.call_actions.lock().unwrap() = Some(Arc::from(sink));
    }
    /// Whether new sessions announce `call-action/1`; a live one changes with
    /// [`CoreConnection::set_call_actions_accepted`].
    pub fn set_call_actions_enabled(&self, enabled: bool) {
        self.call_actions_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-44: where the PC's dismissals and replies go.
    pub fn set_notify_action_sink(&self, sink: Box<dyn NotifyActionSink>) {
        *self.notify_actions.lock().unwrap() = Some(Arc::from(sink));
    }
    /// Whether new sessions announce `notify-action/1`; a live one changes with
    /// [`CoreConnection::set_notify_actions_accepted`].
    pub fn set_notify_actions_enabled(&self, enabled: bool) {
        self.notify_actions_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    /// WN-47: where the PC's player goes.
    pub fn set_pc_media_sink(&self, sink: Box<dyn PcMediaSink>) {
        *self.pc_media.lock().unwrap() = Some(Arc::from(sink));
    }
    /// Whether new sessions announce `media/1` ("Pokaż odtwarzacz komputera"); a live one changes
    /// with [`CoreConnection::set_pc_media_accepted`].
    pub fn set_pc_media_enabled(&self, enabled: bool) {
        self.pc_media_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }
    pub fn is_ringing(&self) -> bool {
        self.ringer
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.is_ringing())
    }
    pub fn set_clipboard_enabled(&self, enabled: bool) {
        self.clipboard.set_enabled(enabled);
    }
    /// The largest image PNG sent or taken, in bytes (the user's setting).
    pub fn set_clipboard_image_limit(&self, bytes: u64) {
        self.clipboard.set_image_limit(bytes);
    }
    /// CCS/1 SEC:call only after explicit confirmation; parsed offer is single-use and never exports its secret.
    pub async fn pair_qr(
        &self,
        offer: Arc<QrOffer>,
        directory: String,
    ) -> Result<PeerInfo, CoreError> {
        let offer = offer
            .offer
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| CoreError::InvalidInput {
                reason: "pairing offer already consumed".into(),
            })?;
        let store = wind_core::pairing::PairedDevicesFile::in_dir(directory);
        let device = tokio::time::timeout(
            Duration::from_secs(30),
            wind_core::pairing::pair_with_offer(
                &self.device_identity,
                &offer,
                Some(&self.local.name),
                &store,
                &self.trust,
                &wind_core::pairing::PairingConfig::default(),
            ),
        )
        .await
        .map_err(|_| CoreError::Transport {
            reason: "pairing timed out".into(),
        })?
        .map_err(|_| CoreError::Transport {
            reason: "pairing refused or expired; scan a new QR code".into(),
        })?;
        Ok(PeerInfo {
            public_key: device.key.as_bytes().to_vec(),
            fingerprint: device.key.fingerprint().to_hex(),
            name: device.name.unwrap_or_else(|| "Komputer".into()),
        })
    }

    /// Starts pairing by code comparison (PROTOCOL.md §12.6) with a
    /// discovered computer whose SAS window is open (`pairing_port`). Nothing
    /// is trusted yet: [`SasPairing::code`] gives the code to show, and only
    /// [`SasPairing::decide`] with `true`, matched by the computer's own
    /// acceptance, records and pins its key.
    /// CCS/1 SEC:the address is an untrusted mDNS hint; the code comparison authenticates (§12.4, §12.6).
    pub async fn start_sas(
        &self,
        device: DiscoveredDevice,
        directory: String,
    ) -> Result<Arc<SasPairing>, CoreError> {
        let port = device.pairing_port.ok_or_else(|| CoreError::InvalidInput {
            reason: "the computer has no pairing window open".into(),
        })?;
        let ips: Vec<std::net::IpAddr> = device
            .addresses
            .iter()
            .filter_map(|a| a.parse::<std::net::SocketAddr>().ok().map(|s| s.ip()))
            .collect();
        let ip = ips
            .iter()
            .find(|ip| ip.is_ipv4())
            .or(ips.first())
            .copied()
            .ok_or_else(|| CoreError::InvalidInput {
                reason: "the computer has no address".into(),
            })?;
        let address = std::net::SocketAddr::new(ip, port);
        let (prompt_tx, prompt_rx) = tokio::sync::oneshot::channel();
        let identity = self.device_identity.clone();
        let trust = self.trust.clone();
        let name = self.local.name.clone();
        let task = tokio::spawn(async move {
            let store = wind_core::pairing::PairedDevicesFile::in_dir(directory);
            wind_core::pairing::pair_with_sas(
                &identity,
                address,
                Some(&name),
                &store,
                &trust,
                &wind_core::pairing::PairingConfig::default(),
                move |prompt| {
                    let _ = prompt_tx.send(prompt);
                },
            )
            .await
        });
        Ok(Arc::new(SasPairing {
            prompt: tokio::sync::Mutex::new(Some(prompt_rx)),
            pending: std::sync::Mutex::new(None),
            task: tokio::sync::Mutex::new(Some(task)),
        }))
    }

    /// Every paired device and every one still owed its unpair notice
    /// (WC-28), and pins them as [`load_paired`](Self::load_paired) does.
    pub fn paired_list(&self, directory: String) -> Result<Vec<PairedEntry>, CoreError> {
        let list = wind_core::pairing::PairedDevicesFile::in_dir(directory)
            .load()
            .map_err(|_| CoreError::Storage {
                reason: "paired devices could not be read".into(),
            })?;
        list.pin_all(&self.trust);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let owed = list.owed(now);
        let mut entries: Vec<PairedEntry> = list
            .devices
            .iter()
            .map(|device| PairedEntry {
                public_key: device.key.as_bytes().to_vec(),
                fingerprint: device.key.fingerprint().to_hex(),
                name: device.name.clone().unwrap_or_else(|| "Komputer".into()),
                paired_at: device.paired_at,
                owed: false,
            })
            .collect();
        entries.extend(
            list.revoked
                .iter()
                .filter(|(k, _)| owed.contains(k))
                .map(|(key, at)| PairedEntry {
                    public_key: key.as_bytes().to_vec(),
                    fingerprint: key.fingerprint().to_hex(),
                    name: "Komputer".into(),
                    paired_at: *at,
                    owed: true,
                }),
        );
        Ok(entries)
    }

    /// Gives the paired device `key` the local name `name` (never sent).
    pub fn rename_paired(
        &self,
        directory: String,
        key: Vec<u8>,
        name: String,
    ) -> Result<(), CoreError> {
        wind_core::pairing::PairedDevicesFile::in_dir(directory)
            .rename(&public_key(&key)?, &name)
            .map_err(storage_error)
    }

    /// Removes the pairing with `key` while it isn't connected (§12.10):
    /// its key is revoked at once and the notice is owed until
    /// [`deliver_notice`](Self::deliver_notice) reaches it. With a
    /// connection to it, use [`CoreConnection::unpair`].
    pub async fn unpair(&self, directory: String, key: Vec<u8>) -> Result<(), CoreError> {
        wind_core::pairing::Unpairing::new(
            wind_core::pairing::PairedDevicesFile::in_dir(directory),
            self.trust.clone(),
            self.endpoint.clone(),
        )
        .unpair(public_key(&key)?, None)
        .await
        .map(|_| ())
        .map_err(storage_error)
    }

    /// Dials `key` (owed a notice) at `address` once, without 0-RTT, and
    /// sends `Goodbye UNPAIRED`. True when delivered: then nothing is owed.
    /// CCS/1 SEC:the dial opens no channel; the address is an untrusted hint, TLS proves the key.
    pub async fn deliver_notice(
        &self,
        directory: String,
        address: String,
        key: Vec<u8>,
    ) -> Result<bool, CoreError> {
        let address = address.parse().map_err(|_| CoreError::InvalidInput {
            reason: "peer address must be an IP address and port".into(),
        })?;
        let peer = public_key(&key)?;
        if !wind_core::transport::PeerTrust::is_revoked(self.trust.as_ref(), &peer) {
            return Err(CoreError::InvalidInput {
                reason: "no unpair notice is owed to this device".into(),
            });
        }
        let unpairing = wind_core::pairing::Unpairing::new(
            wind_core::pairing::PairedDevicesFile::in_dir(directory),
            self.trust.clone(),
            self.endpoint.clone(),
        );
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            Session::dial(
                &self.endpoint,
                address,
                peer,
                self.local.clone(),
                Router::new(),
            ),
        )
        .await
        .map_err(|_| CoreError::Transport {
            reason: "the computer did not answer".into(),
        })?;
        match result {
            Ok((session, _)) => {
                // Cannot happen for a revoked key (notice mode); close anyway.
                session.connection().close(ErrorCode::Unpaired, b"");
                Ok(false)
            }
            Err(error) => {
                let delivered = matches!(
                    error,
                    wind_core::session::SessionError::Unpaired {
                        delivered: true,
                        ..
                    }
                );
                if unpairing.notice_result(&error).map_err(storage_error)? {
                    Ok(delivered)
                } else {
                    Err(session_error(error))
                }
            }
        }
    }

    pub fn load_paired(&self, directory: String) -> Result<PairedLoadResult, CoreError> {
        let list = wind_core::pairing::PairedDevicesFile::in_dir(directory)
            .load()
            .map_err(|_| CoreError::Storage {
                reason: "paired devices could not be read".into(),
            })?;
        // CCS/1 RULE:damaged records are reported separately; valid records remain trusted (WN-19).
        let skipped_records = list.skipped.len() as u64;
        list.pin_all(&self.trust);
        let devices = list
            .devices
            .into_iter()
            .map(|device| PeerInfo {
                public_key: device.key.as_bytes().to_vec(),
                fingerprint: device.key.fingerprint().to_hex(),
                name: device.name.unwrap_or_else(|| "Komputer".into()),
            })
            .collect();
        Ok(PairedLoadResult {
            devices,
            skipped_records,
        })
    }
    pub async fn start_discovery(&self) -> Result<(), CoreError> {
        let mut discovery = self.discovery.lock().await;
        if discovery.is_none() {
            let key = public_key(&self.identity.public_key)?;
            // Paired devices always get a place in the broadcast cache (WN-87).
            let options = DiscoveryOptions {
                paired: Some(self.trust.clone()),
                ..Default::default()
            };
            let watcher = NetworkDiscovery::start_with_options(
                key,
                self.endpoint.clone(),
                self.network.clone(),
                options,
            )
            .map_err(|error| CoreError::Transport {
                reason: error.to_string(),
            })?;
            *self.discovered.lock().await = Some(watcher.subscribe());
            *discovery = Some(watcher);
        }
        Ok(())
    }

    /// Platform callbacks call this off their callback thread. Does not acquire discovery locks.
    /// CCS/1 DEC:a fixed port (firewall rules, a known address) survives: the socket stays, connections still probe the new path.
    pub async fn network_changed(&self) -> Result<(), CoreError> {
        let result = if self.bind_port == 0 {
            self.endpoint
                .rebind(std::net::SocketAddr::new(self.bind_ip, 0))
        } else {
            self.endpoint.refresh_path()
        };
        self.network.notify();
        result.map(|_| ()).map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })
    }

    pub async fn next_network_snapshot(&self) -> Result<NetworkSnapshot, CoreError> {
        let mut receiver = self.discovered.lock().await;
        let receiver = receiver.as_mut().ok_or_else(|| CoreError::Transport {
            reason: "discovery is not started".into(),
        })?;
        receiver.changed().await.map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })?;
        let state = receiver.borrow_and_update().clone();
        Ok(NetworkSnapshot {
            generation: state.generation,
            error: state.error,
            peers: state
                .peers
                .into_iter()
                .map(|peer| DiscoveredDevice {
                    fingerprint: peer
                        .fingerprint
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                    addresses: peer.addresses.into_iter().map(|a| a.to_string()).collect(),
                    pairing_port: peer.pairing_port,
                })
                .collect(),
        })
    }
    pub fn identity(&self) -> IdentityInfo {
        self.identity.clone()
    }

    pub fn local_address(&self) -> Result<String, CoreError> {
        self.endpoint
            .local_addr()
            .map(|a| a.to_string())
            .map_err(|e| CoreError::Transport {
                reason: e.to_string(),
            })
    }

    /// Explicit caller authorization only; never call this from discovery.
    pub fn pin_peer(&self, key: Vec<u8>) -> Result<(), CoreError> {
        self.trust.pin(public_key(&key)?);
        Ok(())
    }

    /// Turns the relay on with its base URL (`wss://…`, see
    /// `wind_core::relay::check_url`) or off with `None`. Off by default (D3);
    /// takes effect from the next connection.
    pub fn set_relay(&self, url: Option<String>) -> Result<(), CoreError> {
        if let Some(url) = &url {
            wind_core::relay::check_url(url).map_err(|e| CoreError::InvalidInput {
                reason: e.to_string(),
            })?;
        }
        *self.relay.lock().unwrap_or_else(|e| e.into_inner()) = url;
        Ok(())
    }

    pub fn relay_url(&self) -> Option<String> {
        self.relay.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// CCS/1 FLOW:address (LAN_FIRST when the relay is on) -> relay (§13); route() tells which
    pub async fn connect(
        &self,
        address: String,
        key: Vec<u8>,
    ) -> Result<Arc<CoreConnection>, CoreError> {
        let relay = self.relay_url();
        let wait = if relay.is_some() {
            LAN_FIRST
        } else {
            Duration::from_secs(10)
        };
        let direct = self.dial_direct(&address, &key, wait).await;
        let (Err(error), Some(relay)) = (&direct, relay) else {
            return direct;
        };
        if matches!(
            error,
            CoreError::UntrustedPeer { .. } | CoreError::InvalidInput { .. }
        ) {
            return direct;
        }
        let peer = public_key(&key)?;
        let link = RelayLink::connect(&relay, &self.device_identity, peer, self.endpoint.clone())
            .await
            .map_err(|e| CoreError::Transport {
                reason: e.to_string(),
            })?;
        let mut router = Router::new();
        let file_input = router.route_files(1, 32);
        router.start_disabled(wind_core::protocol::Channel::Files);
        let clipboard_input = wind_core::clipboard::Clipboard::route(&mut router);
        let find = self.route_find(&mut router);
        let media = self.route_media(&mut router);
        let pc_media = self.route_pc_media(&mut router);
        let notify_actions = self.route_notify_actions(&mut router);
        let volume = self.route_volume(&mut router);
        let sms = self.route_sms(&mut router);
        let gallery = self.route_gallery(&mut router);
        let call_actions = self.route_call_actions(&mut router);
        let actions = self.route_actions(&mut router);
        let (session, events) = tokio::time::timeout(
            Duration::from_secs(10),
            Session::dial(
                &self.endpoint,
                link.addr(),
                peer,
                self.local.clone(),
                router,
            ),
        )
        .await
        .map_err(|_| CoreError::Transport {
            reason: "the peer did not answer through the relay".into(),
        })?
        .map_err(session_error)?;
        self.serve_pc_media(pc_media);
        self.serve_find(find);
        self.serve_sms(sms);
        self.serve_gallery(gallery);
        self.serve_call_actions(call_actions);
        self.serve_actions(&session, actions);
        self.serve_notify_actions(notify_actions);
        let media_allowed = self.serve_media(media);
        let file_caps = file_transfer::FileCapabilities::new(&session, events);
        let extras = self.start_extras(&session, &file_caps, volume);
        Ok(Arc::new(CoreConnection {
            touchpad_owner: Arc::new(tokio::sync::Mutex::new(0)),
            file_caps,
            extras,
            files_active: std::sync::atomic::AtomicBool::new(false),
            files: self.files.clone(),
            file_input: std::sync::Mutex::new(Some(file_input)),
            clipboard: std::sync::Mutex::new(None),
            clipboard_shared: self.clipboard.clone(),
            battery: wind_core::battery::Sender::default(),
            media: wind_core::media::Sender::default(),
            media_allowed,
            notify: wind_core::notify::Sender::new(session.clone()),
            clipboard_input: std::sync::Mutex::new(Some(clipboard_input)),
            session,
            relay: Some(link),
            trust: self.trust.clone(),
            endpoint: self.endpoint.clone(),
        }))
    }

    /// Only the address, for LAN_FIRST: e.g. to come back from the relay
    /// once the network changed.
    pub async fn connect_direct(
        &self,
        address: String,
        key: Vec<u8>,
    ) -> Result<Arc<CoreConnection>, CoreError> {
        self.dial_direct(&address, &key, LAN_FIRST).await
    }

    pub async fn shutdown(&self) {
        if let Some(mut discovery) = self.discovery.lock().await.take() {
            discovery.shutdown().await;
        }
        self.endpoint.close();
        self.endpoint.wait_idle().await;
    }
}

impl CoreClient {
    /// WN-50: routes `find` when the phone can ring; withdrawn at the start
    /// while the person doesn't allow it, so the setting can turn it on live.
    fn route_find(
        &self,
        router: &mut Router,
    ) -> Option<(
        Arc<wind_core::find::Ringer>,
        tokio::sync::mpsc::Receiver<wind_core::session::FindStream>,
    )> {
        let ringer = self.ringer.lock().unwrap().clone()?;
        let streams = router.route_find(4);
        if !self.find_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            router.start_disabled(wind_core::protocol::Channel::Find);
        }
        Some((ringer, streams))
    }
    /// WN-46: routes `media-control` when there is a sink; withdrawn at the start unless allowed.
    fn route_media(&self, router: &mut Router) -> Option<MediaRoute> {
        let sink = self.media.lock().unwrap().clone()?;
        let queue = wind_core::media::route_control(router);
        if !self.media_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            router.start_disabled(wind_core::protocol::Channel::MediaControl);
        }
        Some((sink, Arc::new(std::sync::atomic::AtomicU64::new(0)), queue))
    }
    fn serve_media(&self, media: Option<MediaRoute>) -> Arc<std::sync::atomic::AtomicU64> {
        match media {
            Some((sink, allowed, queue)) => {
                tokio::spawn(serve_media(sink, allowed.clone(), queue));
                allowed
            }
            None => Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
    /// WN-47: routes `media` (the PC's player) when there is a sink; withdrawn at the start unless wanted.
    fn route_pc_media(&self, router: &mut Router) -> Option<PcMediaRoute> {
        let sink = self.pc_media.lock().unwrap().clone()?;
        let queue = wind_core::media::route_player(router);
        if !self
            .pc_media_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            router.start_disabled(wind_core::protocol::Channel::Media);
        }
        Some((sink, queue))
    }
    /// WN-44: routes `notify-action` when there is a sink; withdrawn at the start unless notifications are on.
    fn route_notify_actions(&self, router: &mut Router) -> Option<NotifyActionRoute> {
        let sink = self.notify_actions.lock().unwrap().clone()?;
        let queue = wind_core::notify::route_actions(router);
        if !self
            .notify_actions_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            router.start_disabled(wind_core::protocol::Channel::NotifyAction);
        }
        Some((sink, queue))
    }
    /// WN-48: routes `volume` when there is a sink; withdrawn at the start unless "Głośność z komputera" is on.
    fn route_volume(&self, router: &mut Router) -> Option<phone_extras::VolumeRoute> {
        let sink = self.volume.lock().unwrap().clone()?;
        let queue = wind_core::volume::route(router);
        if !self
            .volume_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            router.start_disabled(wind_core::protocol::Channel::Volume);
        }
        Some((sink, queue))
    }
    fn start_extras(
        &self,
        session: &Session,
        caps: &file_transfer::FileCapabilities,
        volume: Option<phone_extras::VolumeRoute>,
    ) -> phone_extras::PhoneExtras {
        phone_extras::PhoneExtras::start(
            session.clone(),
            caps.peer_caps(),
            volume,
            self.volume_enabled
                .load(std::sync::atomic::Ordering::SeqCst),
        )
    }
    fn serve_notify_actions(&self, route: Option<NotifyActionRoute>) {
        if let Some((sink, queue)) = route {
            tokio::spawn(serve_notify_actions(sink, queue));
        }
    }
    fn serve_pc_media(&self, route: Option<PcMediaRoute>) {
        if let Some((sink, queue)) = route {
            tokio::spawn(serve_pc_media(sink, queue));
        }
    }
    /// WN-55: routes `sms` when there is a source; withdrawn at the start unless allowed and granted.
    fn route_sms(&self, router: &mut Router) -> Option<sms_bridge::SmsRoute> {
        let server = self.sms.lock().unwrap().clone()?;
        let streams = router.route_sms(8);
        if !self.sms_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            router.start_disabled(wind_core::protocol::Channel::Sms);
        }
        Some((server, streams))
    }
    /// WN-58: routes `gallery` when there is a source; withdrawn at the start unless allowed and granted.
    fn route_gallery(&self, router: &mut Router) -> Option<gallery_bridge::GalleryRoute> {
        let server = self.gallery.lock().unwrap().clone()?;
        let streams = router.route_gallery(8);
        if !self
            .gallery_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            router.start_disabled(wind_core::protocol::Channel::Gallery);
        }
        Some((server, streams))
    }
    fn serve_gallery(&self, route: Option<gallery_bridge::GalleryRoute>) {
        if let Some((server, streams)) = route {
            tokio::spawn(server.serve(streams));
        }
    }
    /// WN-57: routes `call-action` when there is a sink; withdrawn at the start unless "Połączenia na komputerze" is on.
    fn route_call_actions(&self, router: &mut Router) -> Option<phone_extras::CallActionRoute> {
        let sink = self.call_actions.lock().unwrap().clone()?;
        let queue = wind_core::call::route_actions(router);
        if !self
            .call_actions_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            router.start_disabled(wind_core::protocol::Channel::CallAction);
        }
        Some((sink, queue))
    }
    /// WN-100: routes `actions` when there is a URL opener (announced even with links off: the PC then gets "refused").
    fn route_actions(&self, router: &mut Router) -> Option<phone_actions::ActionRoute> {
        let opener = self.url_opener.lock().unwrap().clone()?;
        Some((opener, router.route_actions(4)))
    }
    fn serve_actions(&self, session: &Session, route: Option<phone_actions::ActionRoute>) {
        if let Some((opener, streams)) = route {
            let host = wind_core::actions::ActionHost::new(
                Arc::new(phone_actions::PhonePolicy(self.open_links.clone())),
                Arc::new(phone_actions::PhoneExecutor(opener)),
                self.trust.clone(),
            );
            tokio::spawn(host.serve(session.clone(), streams));
        }
    }
    fn serve_call_actions(&self, route: Option<phone_extras::CallActionRoute>) {
        if let Some((sink, queue)) = route {
            tokio::spawn(phone_extras::serve_call_actions(sink, queue));
        }
    }
    fn serve_sms(&self, route: Option<sms_bridge::SmsRoute>) {
        if let Some((server, streams)) = route {
            tokio::spawn(server.serve(streams));
        }
    }
    fn serve_find(
        &self,
        find: Option<(
            Arc<wind_core::find::Ringer>,
            tokio::sync::mpsc::Receiver<wind_core::session::FindStream>,
        )>,
    ) {
        if let Some((ringer, streams)) = find {
            tokio::spawn(ringer.serve(streams));
        }
    }
    async fn dial_direct(
        &self,
        address: &str,
        key: &[u8],
        wait: Duration,
    ) -> Result<Arc<CoreConnection>, CoreError> {
        let address = address.parse().map_err(|_| CoreError::InvalidInput {
            reason: "peer address must be an IP address and port".into(),
        })?;
        let mut changes = self.network.subscribe();
        let ((session, events),(clipboard_input, find, media, pc_media, notify_actions, volume, sms, gallery, call_actions, actions, file_input)) = loop {
            let mut router=Router::new();
            let file_input=router.route_files(1,32);
            router.start_disabled(wind_core::protocol::Channel::Files);
            let clipboard_input=wind_core::clipboard::Clipboard::route(&mut router);
            let find = self.route_find(&mut router);
            let media = self.route_media(&mut router);
            let pc_media = self.route_pc_media(&mut router);
            let notify_actions = self.route_notify_actions(&mut router);
            let volume = self.route_volume(&mut router);
            let sms = self.route_sms(&mut router);
            let gallery = self.route_gallery(&mut router);
            let call_actions = self.route_call_actions(&mut router);
            let actions = self.route_actions(&mut router);
            let attempt = tokio::time::timeout(
            wait,
            Session::dial(
                &self.endpoint,
                address,
                public_key(key)?,
                self.local.clone(),
                router,
            ),
        );
            tokio::select! {
                result = attempt => break result.map(|r|r.map(|s|(s,(clipboard_input, find, media, pc_media, notify_actions, volume, sms, gallery, call_actions, actions, file_input)))),
                changed = changes.changed() => {
                    if changed.is_err() { return Err(CoreError::Transport { reason: "network observer stopped".into() }); }
                }
            }
        }
        .map_err(|_| CoreError::Transport {
            reason: "connection timed out".into(),
        })?
        .map_err(session_error)?;
        self.serve_pc_media(pc_media);
        self.serve_find(find);
        self.serve_sms(sms);
        self.serve_gallery(gallery);
        self.serve_call_actions(call_actions);
        self.serve_actions(&session, actions);
        self.serve_notify_actions(notify_actions);
        let media_allowed = self.serve_media(media);
        let file_caps = file_transfer::FileCapabilities::new(&session, events);
        let extras = self.start_extras(&session, &file_caps, volume);
        Ok(Arc::new(CoreConnection {
            touchpad_owner: Arc::new(tokio::sync::Mutex::new(0)),
            file_caps,
            extras,
            files_active: std::sync::atomic::AtomicBool::new(false),
            files: self.files.clone(),
            file_input: std::sync::Mutex::new(Some(file_input)),
            clipboard: std::sync::Mutex::new(None),
            clipboard_shared: self.clipboard.clone(),
            battery: wind_core::battery::Sender::default(),
            media: wind_core::media::Sender::default(),
            media_allowed,
            notify: wind_core::notify::Sender::new(session.clone()),
            clipboard_input: std::sync::Mutex::new(Some(clipboard_input)),
            session,
            relay: None,
            trust: self.trust.clone(),
            endpoint: self.endpoint.clone(),
        }))
    }
}

/// The code both screens show (§12.6), and the computer's name.
#[derive(uniffi::Record, Clone, Debug)]
pub struct SasCode {
    /// Six digits in two groups: `042 917`.
    pub code: String,
    pub peer_name: String,
}

type SasTask = tokio::task::JoinHandle<
    Result<wind_core::pairing::PairedDevice, wind_core::pairing::PairError>,
>;

/// One pairing by code comparison, from [`CoreClient::start_sas`].
/// Dropping it without a decision rejects.
#[derive(uniffi::Object)]
pub struct SasPairing {
    prompt:
        tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<wind_core::pairing::SasPrompt>>>,
    pending: std::sync::Mutex<Option<wind_core::pairing::SasPrompt>>,
    task: tokio::sync::Mutex<Option<SasTask>>,
}

fn sas_error(error: wind_core::pairing::PairError) -> CoreError {
    use wind_core::pairing::PairError;
    CoreError::Transport {
        reason: match error {
            PairError::Rejected => "the code was rejected",
            PairError::Timeout => "pairing timed out",
            PairError::Refused(ErrorCode::RateLimited) => {
                "the computer is busy or locked after failed attempts; show a new code there"
            }
            PairError::Refused(_) | PairError::Connection(_) => {
                "the computer closed pairing; open pairing by code there again"
            }
            _ => "pairing failed",
        }
        .into(),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SasPairing {
    /// Waits for the code (both sides' nonces exchanged).
    pub async fn code(&self) -> Result<SasCode, CoreError> {
        let receiver = self.prompt.lock().await.take();
        let Some(receiver) = receiver else {
            return Err(CoreError::InvalidInput {
                reason: "the code was already read".into(),
            });
        };
        match receiver.await {
            Ok(prompt) => {
                let code = SasCode {
                    code: prompt.code_text(),
                    peer_name: prompt.peer_name().unwrap_or("Komputer").to_string(),
                };
                *self.pending.lock().unwrap() = Some(prompt);
                Ok(code)
            }
            // The task ended before a code: its error says why.
            Err(_) => Err(self.finish().await.err().unwrap_or(CoreError::Transport {
                reason: "pairing failed".into(),
            })),
        }
    }

    /// The person's answer. `true` pairs only if the computer accepted too;
    /// `false` rejects. Returns the computer once both accepted.
    pub async fn decide(&self, accept: bool) -> Result<PeerInfo, CoreError> {
        let prompt = self.pending.lock().unwrap().take();
        let Some(prompt) = prompt else {
            return Err(CoreError::InvalidInput {
                reason: "no code to decide on".into(),
            });
        };
        if accept {
            prompt.accept();
        } else {
            prompt.reject();
        }
        let device = self.finish().await?;
        Ok(PeerInfo {
            public_key: device.key.as_bytes().to_vec(),
            fingerprint: device.key.fingerprint().to_hex(),
            name: device.name.unwrap_or_else(|| "Komputer".into()),
        })
    }

    /// Stops the attempt (rejects a shown code).
    pub fn cancel(&self) {
        if let Some(prompt) = self.pending.lock().unwrap().take() {
            prompt.reject();
        }
        if let Ok(mut task) = self.task.try_lock() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
    }
}

impl SasPairing {
    async fn finish(&self) -> Result<wind_core::pairing::PairedDevice, CoreError> {
        let task = self
            .task
            .lock()
            .await
            .take()
            .ok_or(CoreError::InvalidInput {
                reason: "pairing already finished".into(),
            })?;
        match task.await {
            Ok(result) => result.map_err(sas_error),
            Err(_) => Err(CoreError::Transport {
                reason: "pairing was cancelled".into(),
            }),
        }
    }
}

impl Drop for SasPairing {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(uniffi::Object)]
pub struct QrOffer {
    offer: std::sync::Mutex<Option<wind_core::pairing::PairingOffer>>,
    info: PeerInfo,
}

#[uniffi::export]
impl QrOffer {
    pub fn preview(&self) -> PeerInfo {
        self.info.clone()
    }
}

#[uniffi::export]
pub fn inspect_qr_link(link: String) -> Result<Arc<QrOffer>, CoreError> {
    let link = Zeroizing::new(link);
    let offer =
        wind_core::pairing::PairingOffer::parse(&link).map_err(|_| CoreError::InvalidInput {
            reason: "invalid WindConnect pairing link".into(),
        })?;
    let info = PeerInfo {
        public_key: offer.key.as_bytes().to_vec(),
        fingerprint: offer.key.fingerprint().to_hex(),
        name: offer.name.clone().unwrap_or_else(|| "Komputer".into()),
    };
    Ok(Arc::new(QrOffer {
        offer: std::sync::Mutex::new(Some(offer)),
        info,
    }))
}

impl Drop for CoreClient {
    fn drop(&mut self) {
        self.endpoint.close();
    }
}

/// An action the peer would run now (PROTOCOL.md §9.2).
#[derive(uniffi::Record, Clone, Debug)]
pub struct RemoteAction {
    pub id: String,
    pub label: String,
}

/// How a remote action ended: "done", "refused", "bad-argument", "failed"
/// or "busy".
fn status_name(status: wind_core::actions::Status) -> String {
    use wind_core::actions::Status;
    match status {
        Status::Done => "done",
        Status::Refused => "refused",
        Status::BadArgument => "bad-argument",
        Status::Failed => "failed",
        Status::Busy => "busy",
    }
    .into()
}

fn action_error(error: wind_core::actions::RequestError) -> CoreError {
    match error {
        wind_core::actions::RequestError::Session(e) => session_error(e),
        e => CoreError::Transport {
            reason: e.to_string(),
        },
    }
}

#[derive(uniffi::Object)]
pub struct CoreConnection {
    touchpad_owner: Arc<tokio::sync::Mutex<u64>>,
    files: Arc<file_transfer::FileBridge>,
    file_input:
        std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<wind_core::session::FileStream>>>,
    file_caps: file_transfer::FileCapabilities,
    /// WN-48: the call state and both volumes.
    extras: phone_extras::PhoneExtras,
    files_active: std::sync::atomic::AtomicBool,
    clipboard: std::sync::Mutex<Option<Arc<wind_core::clipboard::Connection>>>,
    clipboard_shared: Arc<wind_core::clipboard::Clipboard>,
    clipboard_input:
        std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<wind_core::session::ChannelMessage>>>,
    /// WN-49: sends only changes of the battery status.
    battery: wind_core::battery::Sender,
    /// WN-46: the phone's player, sent only on a change; what its commands may do.
    media: wind_core::media::Sender,
    media_allowed: Arc<std::sync::atomic::AtomicU64>,
    /// WN-42: the phone's notifications, by §9.7's rules (icons once, one a second per id).
    notify: Arc<wind_core::notify::Sender>,
    session: Session,
    /// Held while the session runs through the relay.
    relay: Option<RelayLink>,
    trust: Arc<PinnedKeys>,
    endpoint: Endpoint,
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreConnection {
    /// CCS/1 RULE:only an adopted connection may change the system clipboard; a standby LAN probe never takes ownership.
    pub async fn activate_clipboard(&self) {
        let mut active = self.clipboard.lock().unwrap();
        if active.is_none() {
            if let Some(input) = self.clipboard_input.lock().unwrap().take() {
                *active = Some(self.clipboard_shared.attach(self.session.clone(), input));
            }
        }
    }
    pub async fn send_clipboard(&self, text: String, sensitive: bool, manual: bool) -> String {
        let binding = self.clipboard.lock().unwrap().clone();
        match binding {
            Some(binding) => binding.send(text, sensitive, manual).await.name(),
            None => "unavailable",
        }
        .into()
    }
    /// WN-36: sends a PNG with its thumbnail (clipboard/2).
    #[allow(clippy::too_many_arguments)]
    pub async fn send_clipboard_image(
        &self,
        png: Vec<u8>,
        thumbnail: Vec<u8>,
        width: u64,
        height: u64,
        sensitive: bool,
        manual: bool,
        progress: Box<dyn ClipboardImageProgress>,
    ) -> ClipboardSendResult {
        let binding = self.clipboard.lock().unwrap().clone();
        let status = match binding {
            Some(binding) => {
                binding
                    .send_image(
                        wind_core::clipboard::OutgoingImage {
                            png,
                            thumbnail,
                            width,
                            height,
                        },
                        sensitive,
                        manual,
                        move || progress.applied(),
                    )
                    .await
            }
            None => wind_core::clipboard::Status::Unavailable,
        };
        ClipboardSendResult {
            status: status.name().into(),
            peer_limit: match status {
                wind_core::clipboard::Status::PeerLimit(limit) => Some(limit),
                _ => None,
            },
        }
    }
    /// WN-49 (§9.4): the phone's battery for the PC. Sends only when
    /// (level, charging) changed since the last send on this connection, and
    /// nothing when the peer doesn't take `battery/1`. True when sent.
    pub async fn send_battery(&self, level: u8, charging: bool) -> Result<bool, CoreError> {
        let status = wind_core::battery::Status::new(level, charging).ok_or_else(|| {
            CoreError::Transport {
                reason: format!("battery level {level} over 100"),
            }
        })?;
        self.battery
            .send(&self.session, status)
            .await
            .map_err(session_error)
    }
    /// WN-46 (§9.6): the phone's player for the PC; sent only on a change (art once per image).
    /// True when sent. Commands from now on are checked against `now.actions`.
    pub async fn send_media(&self, now: MediaNow, art: Option<Vec<u8>>) -> Result<bool, CoreError> {
        self.media_allowed
            .store(now.actions, std::sync::atomic::Ordering::SeqCst);
        let now = wind_core::media::Now {
            title: now.title,
            artist: now.artist,
            album: now.album,
            app: now.app,
            duration_ms: now.duration_ms,
            position_ms: now.position_ms,
            playing: now.playing,
            speed_permille: now.speed_permille,
            actions: now.actions,
            art: None,
        };
        let art = art.filter(|a| {
            a.len() <= wind_core::media::MAX_ART && wind_core::media::image_mime(a).is_some()
        });
        self.media
            .now(&self.session, now, art.as_deref())
            .await
            .map_err(session_error)
    }
    /// WN-42 (§9.7): a notification for the PC, with its app icon (PNG). "sent" (written to the
    /// stream), "held" (goes out when its one-a-second window ends), "dropped" (unchanged or a progress
    /// step) or "not-accepted" (the PC doesn't take notify/1).
    pub async fn send_notification(
        &self,
        notification: PhoneNotification,
        icon: Option<Vec<u8>>,
    ) -> Result<String, CoreError> {
        let posted = wind_core::notify::Posted {
            id: notification_id(&notification.id)?,
            app: notification.app,
            package: notification.package_name,
            title: notification.title,
            text: notification.text,
            time_ms: notification.time_ms,
            icon: None,
            reply: notification.reply,
            quiet: notification.quiet,
        };
        self.notify
            .posted(posted, icon, notification.progress)
            .await
            .map(|o| o.name().into())
            .map_err(session_error)
    }
    /// WN-48: the phone's call state: 0 idle, 1 ringing, 2 offhook (goes to the PC while it takes `call/1`).
    pub fn set_call_state(&self, state: u8) {
        if let Some(s) = wind_core::call::CallState::from_code(state.into()) {
            self.extras.set_call(s);
        }
    }
    /// WN-57: the call state with who calls (`caller` none: the setting off or no number). Idle ends the caller.
    pub fn set_call(&self, state: u8, caller: Option<CallerInfo>) {
        if let Some(s) = wind_core::call::CallState::from_code(state.into()) {
            self.extras.set_call_with(s, caller.map(Into::into));
        }
    }
    /// WN-57: the newest missed calls, newest first (the whole list; empty with the setting off).
    pub fn set_missed_calls(&self, missed: Vec<MissedCall>) {
        self.extras.set_missed(
            missed
                .into_iter()
                .filter_map(|m| {
                    let number = wind_core::sms::clip(&m.number, wind_core::call::MAX_NUMBER);
                    (!number.is_empty()).then(|| wind_core::call::Missed {
                        number,
                        name: m
                            .name
                            .map(|n| wind_core::sms::clip(&n, wind_core::call::MAX_NAME))
                            .filter(|n| !n.is_empty()),
                        date_ms: m.date_ms,
                    })
                })
                .collect(),
        );
    }
    /// WN-57: announce or withdraw `call-action/1` on this session; true when it changed.
    pub async fn set_call_actions_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::CallAction, accept)
            .await
            .map_err(session_error)
    }
    /// WN-48: STREAM_MUSIC as 0..=100 and muted (goes to the PC while both take `volume/1`).
    pub fn set_volume_level(&self, level: u8, muted: bool) {
        self.extras.set_level(level, muted);
    }
    /// WN-48: the person changes the PC's volume (only what is given); the latest wins.
    pub fn send_volume_set(&self, level: Option<u8>, muted: Option<bool>) {
        self.extras.request(level, muted);
    }
    /// WN-48: announce or withdraw `volume/1` on this session ("Głośność z komputera").
    pub async fn set_volume_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.extras.set_enabled(accept);
        self.session
            .set_accepted(wind_core::protocol::Channel::Volume, accept)
            .await
            .map_err(session_error)
    }
    /// WN-44: how a reply the PC asked for went (`Replied`).
    pub async fn notification_replied(&self, id: Vec<u8>, ok: bool) -> Result<String, CoreError> {
        self.notify
            .replied(notification_id(&id)?, ok)
            .await
            .map(|o| o.name().into())
            .map_err(session_error)
    }
    /// WN-44: announce or withdraw `notify-action/1` (the PC may dismiss and reply) on this session.
    pub async fn set_notify_actions_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::NotifyAction, accept)
            .await
            .map_err(session_error)
    }
    /// WN-42: gone on the phone; `Removed` goes out only for an id sent in this session.
    pub async fn notification_removed(&self, id: Vec<u8>) -> Result<String, CoreError> {
        self.notify
            .removed(notification_id(&id)?)
            .await
            .map(|o| o.name().into())
            .map_err(session_error)
    }
    /// No player on the phone any more.
    pub async fn media_gone(&self) -> Result<bool, CoreError> {
        self.media_allowed
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.media.gone(&self.session).await.map_err(session_error)
    }
    /// WN-47: a button for the PC's player ("play", "pause", "toggle", "next", "previous", "seek" with
    /// `position_ms`). The PC drops what its player doesn't allow. False when the PC doesn't take it.
    pub async fn send_media_command(
        &self,
        action: String,
        position_ms: Option<u64>,
    ) -> Result<bool, CoreError> {
        use wind_core::media::Action;
        let action = match (action.as_str(), position_ms) {
            ("play", _) => Action::Play,
            ("pause", _) => Action::Pause,
            ("toggle", _) => Action::Toggle,
            ("next", _) => Action::Next,
            ("previous", _) => Action::Previous,
            ("seek", Some(at)) => Action::Seek(at),
            _ => {
                return Err(CoreError::InvalidInput {
                    reason: format!("unknown media action {action}"),
                })
            }
        };
        match wind_core::media::command(&self.session, action).await {
            Ok(()) => Ok(true),
            Err(wind_core::session::SessionError::CapabilityMissing(_)) => Ok(false),
            Err(e) => Err(session_error(e)),
        }
    }
    /// WN-47: announce or withdraw `media/1` (the PC's player shown here) on this session.
    pub async fn set_pc_media_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::Media, accept)
            .await
            .map_err(session_error)
    }
    /// Announce or withdraw `media-control/1` on this session.
    pub async fn set_media_control_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::MediaControl, accept)
            .await
            .map_err(session_error)
    }
    /// WN-55: announce or withdraw `sms/1` on this session (the setting "SMS na komputerze" with READ_SMS); true
    /// when it changed. Withdrawn, the PC's watch ends and new requests are refused.
    pub async fn set_sms_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::Sms, accept)
            .await
            .map_err(session_error)
    }
    /// WN-58: announce or withdraw `gallery/1` on this session (the setting "Zdjęcia na komputerze" with photo
    /// access); true when it changed. Withdrawn, the PC's watch ends and new requests are refused.
    pub async fn set_gallery_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::Gallery, accept)
            .await
            .map_err(session_error)
    }
    /// WN-50: announce or withdraw `find/1` on this session (the setting
    /// "Pozwól komputerowi dzwonić"); true when it changed. An error when the
    /// phone has no find sink.
    pub async fn set_find_accepted(&self, accept: bool) -> Result<bool, CoreError> {
        self.session
            .set_accepted(wind_core::protocol::Channel::Find, accept)
            .await
            .map_err(session_error)
    }
    pub fn peer(&self) -> PeerInfo {
        let peer = self.session.peer();
        PeerInfo {
            public_key: peer.key.as_bytes().to_vec(),
            name: peer.hello.name.clone(),
            fingerprint: peer.key.fingerprint().to_hex(),
        }
    }

    pub async fn ping(&self) -> Result<u64, CoreError> {
        self.session
            .ping()
            .await
            .map(|d| d.as_micros() as u64)
            .map_err(session_error)
    }

    /// Asks the peer to run `action` ("lock", "sleep", "open-url" or one
    /// of its own "x-…" ids from [`list_actions`](Self::list_actions));
    /// `url` only for "open-url". The peer decides by its own allowlist;
    /// the answer is a status from [`status_name`].
    pub async fn run_action(
        &self,
        action: String,
        url: Option<String>,
    ) -> Result<String, CoreError> {
        if !wind_core::actions::is_action_id(&action) {
            return Err(CoreError::InvalidInput {
                reason: "not an action id".into(),
            });
        }
        wind_core::actions::run(&self.session, &action, url.as_deref())
            .await
            .map(status_name)
            .map_err(action_error)
    }

    /// What the peer would run now (empty when it accepts no actions).
    pub async fn list_actions(&self) -> Result<Vec<RemoteAction>, CoreError> {
        let accepts = self
            .session
            .peer_capabilities()
            .iter()
            .any(|c| c.name() == "actions" && c.version() == 1);
        if !accepts {
            return Ok(Vec::new());
        }
        wind_core::actions::list(&self.session)
            .await
            .map(|list| {
                list.into_iter()
                    .map(|a| RemoteAction {
                        id: a.id,
                        label: a.label,
                    })
                    .collect()
            })
            .map_err(action_error)
    }

    /// "lan" (direct) or "relay" (PROTOCOL.md §13).
    pub fn route(&self) -> String {
        if self.relay.is_some() { "relay" } else { "lan" }.into()
    }

    pub async fn wait_closed(&self) -> String {
        self.session.closed().await.to_string()
    }

    pub async fn disconnect(&self) {
        self.session.goodbye(ErrorCode::GoingAway, None).await;
    }

    /// Why the session ended, once it has: "unpaired" (the peer removed
    /// the pairing: call [`forget_peer`](Self::forget_peer)), "busy" (the
    /// computer keeps a session with another device: don't retry until the
    /// network changes or the user asks), "going-away", "untrusted" or
    /// "closed" (anything else).
    pub async fn wait_end(&self) -> String {
        let error = self.session.closed().await;
        match wind_core::session::close_code(&error) {
            Some(ErrorCode::Unpaired) => "unpaired",
            Some(ErrorCode::Busy) => "busy",
            Some(ErrorCode::GoingAway) => "going-away",
            _ if wind_core::transport::is_untrusted_close(&error) => "untrusted",
            _ => "closed",
        }
        .into()
    }

    /// The peer removed the pairing ("unpaired"): drop its key, the one
    /// from this connection's TLS handshake, from `directory` and the pins.
    pub fn forget_peer(&self, directory: String) -> Result<(), CoreError> {
        self.unpairing(directory)
            .unpaired_by(&self.session)
            .map_err(storage_error)
    }

    /// Removes the pairing with this connection's peer and tells it now
    /// (§12.10). True when it acknowledged; otherwise the notice is owed.
    pub async fn unpair(&self, directory: String) -> Result<bool, CoreError> {
        self.unpairing(directory)
            .unpair(self.session.peer().key, Some(&self.session))
            .await
            .map_err(storage_error)
    }
}

impl CoreConnection {
    fn unpairing(&self, directory: String) -> wind_core::pairing::Unpairing {
        wind_core::pairing::Unpairing::new(
            wind_core::pairing::PairedDevicesFile::in_dir(directory),
            self.trust.clone(),
            self.endpoint.clone(),
        )
    }
}

fn storage_error(error: std::io::Error) -> CoreError {
    match error.kind() {
        std::io::ErrorKind::InvalidInput => CoreError::InvalidInput {
            reason: "the name is empty".into(),
        },
        std::io::ErrorKind::NotFound => CoreError::InvalidInput {
            reason: "the device is not paired".into(),
        },
        _ => CoreError::Storage {
            reason: "paired devices could not be saved".into(),
        },
    }
}

impl Drop for CoreConnection {
    fn drop(&mut self) {
        self.session.connection().close(ErrorCode::GoingAway, b"");
    }
}
