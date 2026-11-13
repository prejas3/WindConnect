//! CCS/1 M:WN-43 the phone's notifications as native Windows toasts (PROTOCOL.md §9.7, notify/1 in).
//! FLOW:session loop decodes -> [`Notifications::apply`] (popup or not) -> one worker thread (order kept) -> ToastNotifier.
//! SEC:texts come from other apps on the phone: sanitised again here (one line, no control/bidi/zero-width characters,
//! bounded) and XML-escaped; icons are decoded and re-encoded as 64 px PNG before Windows reads them, stored as
//! `<key hex>.png` only. Nothing is logged. Windows keeps toast content in its notification store: each expires after
//! [`EXPIRES`], and the setting turned off or an unpairing removes them and the icon cache.
//! WN-44: a reply field on toasts that can be replied to and in the panel (`notify-action/1` to the phone, never trimmed:
//! over [`wind_core::notify::MAX_REPLY`] bytes it isn't sent); closing a popup (✕) or ✕ in the panel dismisses it on the phone.
//! The reply text is never logged nor kept after it was sent. LIMIT:an unpackaged app gets a toast's events only while its
//! ToastNotification object lives here: the worker keeps them until removed or expired; after a restart, old toasts in
//! Action Center at most open the app, and clearing in Action Center fires nothing (no dismissal on the phone).
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use wind_core::notify::{Message, Posted};

/// The toast group of every phone notification (one place to remove them all).
pub const GROUP: &str = "phone";
/// Windows drops a toast from Action Center after this.
pub const EXPIRES: Duration = Duration::from_secs(24 * 60 * 60);
/// At most this many popups in [`BURST_WINDOW`]; the rest go to Action Center quietly.
pub const BURST: usize = 5;
pub const BURST_WINDOW: Duration = Duration::from_secs(2);
/// Icons kept on disk; the oldest go first.
pub const ICONS: usize = 256;
const TITLE_CHARS: usize = 120;
const TEXT_CHARS: usize = 400;

/// One line, no control, bidi or zero-width characters, runs of whitespace as one space, at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut space = true;
    for c in text.chars() {
        let c = if c.is_control() || c.is_whitespace() || crate::clipboard_history::deceptive(c) {
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
        if count == max {
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

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What a toast shows, already cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastView {
    /// The toast's tag: the notification id in hex (32 characters).
    pub tag: String,
    pub app: String,
    pub package: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub phone: String,
    /// The icon's key in hex, when the notification names one.
    pub icon: Option<String>,
    /// The reply action's label, when it can be replied to (WN-44).
    pub reply: Option<String>,
    /// No sound (WN-45): it doesn't pop up (an update, a burst, quiet on the phone, quiet hours, Focus).
    pub silent: bool,
}

impl ToastView {
    pub fn new(n: &Posted, phone: &str) -> Self {
        let some = |s: &Option<String>, max| {
            s.as_deref()
                .map(|t| clean(t, max))
                .filter(|t| !t.is_empty())
        };
        let app = Some(clean(&n.app, 64))
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| clean(&n.package, 64));
        Self {
            tag: hex(&n.id),
            package: clean(&n.package, 128),
            title: some(&n.title, TITLE_CHARS),
            text: some(&n.text, TEXT_CHARS),
            phone: clean(phone, 64),
            icon: n.icon.map(|k| hex(&k)),
            reply: n
                .reply
                .as_deref()
                .map(|r| clean(r, 64))
                .filter(|r| !r.is_empty()),
            silent: false,
            app,
        }
    }
}

/// The toast's XML: title (or the app), text, "app · phone", the app icon, a header per phone app (grouping in
/// Action Center). `icon`: the cached file, when there is one.
pub fn toast_xml(view: &ToastView, icon: Option<&Path>) -> String {
    let mut x = String::from(
        r#"<toast launch="windconnect-notify"><visual><binding template="ToastGeneric">"#,
    );
    let first = view.title.as_deref().unwrap_or(&view.app);
    x.push_str(&format!("<text>{}</text>", escape(first)));
    if let Some(t) = &view.text {
        x.push_str(&format!("<text>{}</text>", escape(t)));
    }
    let via = if view.phone.is_empty() {
        view.app.clone()
    } else {
        format!("{} · {}", view.app, view.phone)
    };
    x.push_str(&format!(
        r#"<text placement="attribution">{}</text>"#,
        escape(&via)
    ));
    if let Some(path) = icon {
        let uri = format!("file:///{}", path.display().to_string().replace('\\', "/"));
        x.push_str(&format!(
            r#"<image placement="appLogoOverride" src="{}"/>"#,
            escape(&uri)
        ));
    }
    x.push_str("</binding></visual>");
    if view.reply.is_some() {
        // WN-44: typed here, sent in the background (the app stays where it is).
        x.push_str(&format!(
            r#"<actions><input id="{REPLY_INPUT}" type="text" placeHolderContent="Odpowiedz…"/><action content="Wyślij" arguments="{REPLY_ARGUMENT}" hint-inputId="{REPLY_INPUT}" activationType="background"/></actions>"#
        ));
    }
    if view.silent {
        x.push_str(r#"<audio silent="true"/>"#);
    }
    let header = if view.package.is_empty() {
        &view.app
    } else {
        &view.package
    };
    x.push_str(&format!(
        r#"<header id="{}" title="{}" arguments="windconnect-notify"/>"#,
        escape(header),
        escape(&view.app)
    ));
    x.push_str("</toast>");
    x
}

/// The toast's text box and its button's arguments (WN-44).
pub const REPLY_INPUT: &str = "reply";
pub const REPLY_ARGUMENT: &str = "reply";

/// What a click on a toast asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// The toast's body (or anything but the reply button): open the panel.
    OpenPanel,
    Reply(String),
    /// The reply button with nothing typed.
    Empty,
    /// Longer than the protocol allows: not sent, never trimmed; the panel shows it with the text kept.
    TooLong(String),
}

/// A toast click by its arguments and the typed text (as `ToastActivatedEventArgs` gives them).
pub fn activation(arguments: &str, input: Option<&str>) -> Activation {
    if arguments != REPLY_ARGUMENT {
        return Activation::OpenPanel;
    }
    let text = input.unwrap_or("").trim();
    if text.is_empty() {
        Activation::Empty
    } else if text.len() > wind_core::notify::MAX_REPLY {
        Activation::TooLong(text.to_owned())
    } else {
        Activation::Reply(text.to_owned())
    }
}

/// Why a toast went away, as `ToastDismissalReason` numbers it: 0 the person, 1 the app (removed by us), 2 timed out.
/// Only the person's own close dismisses it on the phone.
pub fn dismissal_syncs(reason: i32) -> bool {
    reason == 0
}

/// What the toasts tell the session loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    Reply { tag: String, text: String },
    TooLong { tag: String, text: String },
    Dismissed { tag: String },
}

static EVENTS: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<ToastEvent>> =
    std::sync::OnceLock::new();

/// A toast click, from WinRT (or the debug command): the panel opens, or a reply goes to the session loop.
pub fn activated(app: &tauri::AppHandle, tag: &str, arguments: &str, input: Option<&str>) {
    let send = |e| {
        if let Some(events) = EVENTS.get() {
            let _ = events.send(e);
        }
    };
    match activation(arguments, input) {
        Activation::Reply(text) => send(ToastEvent::Reply {
            tag: tag.into(),
            text,
        }),
        Activation::Empty => {}
        Activation::TooLong(text) => {
            send(ToastEvent::TooLong {
                tag: tag.into(),
                text,
            });
            open_panel(app);
        }
        Activation::OpenPanel => open_panel(app),
    }
}

/// A toast went away: the person's own close dismisses it on the phone.
pub fn dismissed(tag: &str, reason: i32) {
    if dismissal_syncs(reason) {
        if let Some(events) = EVENTS.get() {
            let _ = events.send(ToastEvent::Dismissed { tag: tag.into() });
        }
    }
}

fn open_panel(app: &tauri::AppHandle) {
    let panel = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = crate::panel::show(&panel);
    });
}

/// A tag as the toasts use it: the notification id in lower-case hex.
pub fn id_of(tag: &str) -> Option<[u8; 16]> {
    if tag.len() != 32
        || !tag
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    let mut id = [0u8; 16];
    for (i, b) in id.iter_mut().enumerate() {
        *b = u8::from_str_radix(&tag[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(id)
}

/// One of the phone's notifications in the panel (WN-44): texts cleaned, shown as text only.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PanelItem {
    pub tag: String,
    pub app: String,
    pub title: Option<String>,
    pub text: Option<String>,
    /// The reply action's label; none: it can't be replied to.
    pub reply: Option<String>,
    /// "sending", "sent", "failed", "too-long"; none before any reply.
    pub status: Option<&'static str>,
    /// A reply that wasn't sent (too long from a toast): kept for the panel's field, never logged.
    pub draft: Option<String>,
}

/// The panel keeps the newest this many.
pub const PANEL_ITEMS: usize = 20;

/// WN-45: "Ciche godziny", in minutes after local midnight (a time of day, not a timestamp: a DST change can't shift it).
/// Through midnight when `from` > `to`; `from` == `to` is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QuietHours {
    pub on: bool,
    pub from: u16,
    pub to: u16,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            on: false,
            from: 22 * 60,
            to: 7 * 60,
        }
    }
}

impl QuietHours {
    /// Times of day on a 15-minute step.
    pub fn valid(&self) -> bool {
        self.from < 1440
            && self.to < 1440
            && self.from.is_multiple_of(15)
            && self.to.is_multiple_of(15)
    }

    /// Whether `minute` (0..1440, local) is within the quiet hours.
    pub fn contains(&self, minute: u16) -> bool {
        if !self.on || self.from == self.to {
            false
        } else if self.from < self.to {
            minute >= self.from && minute < self.to
        } else {
            minute >= self.from || minute < self.to
        }
    }
}

/// `notify.json`: "Powiadomienia z telefonu" and the quiet hours (WN-45). WN-43 wrote a bare `true`/`false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NotifySettings {
    pub enabled: bool,
    #[serde(default)]
    pub quiet: QuietHours,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet: QuietHours::default(),
        }
    }
}

impl NotifySettings {
    /// The file's bytes: this format, WN-43's bare bool, or anything else (missing, damaged) -> the defaults.
    pub fn parse(bytes: Option<&[u8]>) -> Self {
        let Some(bytes) = bytes else {
            return Self::default();
        };
        if let Ok(mut s) = serde_json::from_slice::<Self>(bytes) {
            if !s.quiet.valid() {
                s.quiet = QuietHours::default();
            }
            return s;
        }
        match serde_json::from_slice::<bool>(bytes) {
            Ok(enabled) => Self {
                enabled,
                ..Self::default()
            },
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> Self {
        Self::parse(std::fs::read(path).ok().as_deref())
    }

    /// Written whole: a temporary file next to it, then renamed over it (never half a file).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let temp = path.with_extension("json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&temp, path)
    }
}

/// `SHQueryUserNotificationState`: busy (fullscreen), a Direct3D fullscreen app, presentation mode and the quiet time after
/// setup want no popups. Windows' own Do not disturb / Focus already holds our toasts back by itself.
pub fn state_silences(state: i32) -> bool {
    matches!(state, 2..=4 | 6)
}

/// WN-45: the PC wants quiet now: Windows is busy (see [`state_silences`]) or it is within the quiet hours.
pub fn silenced_now(quiet: &QuietHours) -> bool {
    platform::busy() || quiet.contains(platform::local_minute())
}

/// Popups allowed: at most [`BURST`] in any [`BURST_WINDOW`].
#[derive(Default)]
pub struct Burst {
    shown: VecDeque<Instant>,
}

impl Burst {
    /// True: this one may pop up (and counts).
    pub fn admit(&mut self, now: Instant) -> bool {
        while self
            .shown
            .front()
            .is_some_and(|t| now.duration_since(*t) >= BURST_WINDOW)
        {
            self.shown.pop_front();
        }
        if self.shown.len() >= BURST {
            return false;
        }
        self.shown.push_back(now);
        true
    }
}

/// The icon cache folder: `<key hex>.png` files, at most [`ICONS`].
pub struct IconCache {
    dir: PathBuf,
}

impl IconCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The file for a key given as 16 hex characters; anything else has none.
    pub fn path(&self, key: &str) -> Option<PathBuf> {
        (key.len() == 16
            && key
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        .then(|| self.dir.join(format!("{key}.png")))
    }

    /// Stores an already re-encoded PNG; then keeps the newest [`ICONS`].
    pub fn store(&self, key: &[u8; 8], png: &[u8]) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(&hex(key)).expect("hex of 8 bytes");
        let tmp = self.dir.join(format!("{}.tmp", hex(key)));
        std::fs::write(&tmp, png)?;
        std::fs::rename(&tmp, &path)?;
        self.prune(ICONS);
        Ok(path)
    }

    fn prune(&self, keep: usize) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if files.len() <= keep {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Removes every cached icon.
    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What the worker thread does, in order.
pub enum Job {
    Icon {
        key: [u8; 8],
        png: Vec<u8>,
    },
    Show {
        view: ToastView,
        popup: bool,
        received: Instant,
        received_ms: u64,
    },
    Remove {
        tag: String,
    },
    /// Every phone toast and the icon cache (setting off, unpairing).
    Clear,
}

/// The phone's notifications in one session: what was shown (an update doesn't pop up again) and the popup limit.
pub struct Notifications {
    jobs: mpsc::Sender<Job>,
    phone: String,
    shown: HashSet<[u8; 16]>,
    burst: Burst,
    /// Newest first, at most [`PANEL_ITEMS`].
    items: VecDeque<PanelItem>,
    /// WN-45: the PC wants quiet now (quiet hours, Focus/fullscreen); asked per notification, never polled.
    silence: Box<dyn Fn() -> bool + Send>,
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Notifications {
    pub fn new(jobs: mpsc::Sender<Job>, phone: &str) -> Self {
        Self {
            jobs,
            phone: phone.into(),
            shown: HashSet::new(),
            burst: Burst::default(),
            items: VecDeque::new(),
            silence: Box::new(|| false),
        }
    }

    /// WN-45: what says the PC wants quiet now.
    pub fn silenced_by(mut self, silence: impl Fn() -> bool + Send + 'static) -> Self {
        self.silence = Box::new(silence);
        self
    }

    /// The panel's list, newest first.
    pub fn items(&self) -> Vec<PanelItem> {
        self.items.iter().cloned().collect()
    }

    fn item(&mut self, tag: &str) -> Option<&mut PanelItem> {
        self.items.iter_mut().find(|i| i.tag == tag)
    }

    /// The id of a notification shown in this session; anything else is none (never acted on).
    pub fn known(&self, tag: &str) -> Option<[u8; 16]> {
        id_of(tag).filter(|id| self.shown.contains(id))
    }

    /// A reply went to the phone; `ok` false: the phone doesn't take it now.
    pub fn reply_sent(&mut self, tag: &str, ok: bool) {
        if let Some(i) = self.item(tag) {
            i.status = Some(if ok { "sending" } else { "failed" });
            i.draft = None;
        }
    }

    /// A reply too long to send (from a toast): kept for the panel, not sent.
    pub fn too_long(&mut self, tag: &str, text: String) {
        if let Some(i) = self.item(tag) {
            i.status = Some("too-long");
            i.draft = Some(text);
        }
    }

    /// Dismissed here (toast ✕ or the panel): off the list and the toasts; the phone is told by the caller.
    pub fn dismiss(&mut self, tag: &str) {
        self.items.retain(|i| i.tag != tag);
        if let Some(id) = id_of(tag) {
            self.shown.remove(&id);
        }
        let _ = self.jobs.send(Job::Remove { tag: tag.into() });
    }

    /// Folds one message in (already decoded and bounded by the core).
    pub fn apply(&mut self, message: Message) {
        let received = Instant::now();
        let job = match message {
            Message::Icon { key, png } => Job::Icon { key, png },
            Message::Posted(n) => {
                let update = !self.shown.insert(n.id);
                // An update replaces its toast quietly; a burst beyond the limit goes to Action Center only. WN-45: quiet on
                // the phone (Do not disturb) or wanted quiet here goes there too, without using up the burst.
                let popup = !update && !n.quiet && !(self.silence)() && self.burst.admit(received);
                let mut view = ToastView::new(&n, &self.phone);
                view.silent = !popup;
                let status = self.item(&view.tag).and_then(|i| i.status);
                self.items.retain(|i| i.tag != view.tag);
                self.items.push_front(PanelItem {
                    tag: view.tag.clone(),
                    app: view.app.clone(),
                    title: view.title.clone(),
                    text: view.text.clone(),
                    reply: view.reply.clone(),
                    status,
                    draft: None,
                });
                self.items.truncate(PANEL_ITEMS);
                Job::Show {
                    view,
                    popup,
                    received,
                    received_ms: wall_ms(),
                }
            }
            Message::Removed { id } => {
                self.shown.remove(&id);
                let tag = hex(&id);
                self.items.retain(|i| i.tag != tag);
                Job::Remove { tag }
            }
            Message::Replied { id, ok } => {
                if let Some(i) = self.item(&hex(&id)) {
                    i.status = Some(if ok { "sent" } else { "failed" });
                }
                return;
            }
        };
        let _ = self.jobs.send(job);
    }
}

/// One toast as the debug log keeps it: no content, only times (the latency check reads them).
#[derive(Clone, serde::Serialize)]
pub struct DevEntry {
    pub tag: String,
    pub popup: bool,
    pub received_ms: u64,
    pub shown_ms: u64,
    /// Received in the session loop -> ToastNotifier.Show returned.
    pub show_us: u64,
    /// The toast carried a reply field (WN-44).
    pub reply: bool,
}

#[cfg(debug_assertions)]
static LOG: std::sync::Mutex<Vec<DevEntry>> = std::sync::Mutex::new(Vec::new());

#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_notify_log() -> Vec<DevEntry> {
    LOG.lock().unwrap().clone()
}

/// The tags of the phone toasts Windows holds now (debug: the native check reads them).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_notify_history(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    platform::history(&app)
}

/// A toast click as WinRT would deliver it (debug: the native check can't click system toasts reliably).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_toast_activate(
    app: tauri::AppHandle,
    tag: String,
    arguments: String,
    input: Option<String>,
) {
    activated(&app, &tag, &arguments, input.as_deref());
}

/// A toast closed as WinRT would report it (`reason` as `ToastDismissalReason`).
#[cfg(debug_assertions)]
#[tauri::command]
pub fn dev_toast_dismissed(tag: String, reason: i32) {
    dismissed(&tag, reason);
}

/// Starts the worker; icons live in `dir`; toast clicks and closes go to `events`.
pub fn worker(
    app: &tauri::AppHandle,
    dir: PathBuf,
    events: tokio::sync::mpsc::UnboundedSender<ToastEvent>,
) -> mpsc::Sender<Job> {
    let _ = EVENTS.set(events);
    let (tx, rx) = mpsc::channel();
    let app = app.clone();
    std::thread::Builder::new()
        .name("WindConnect toasts".into())
        .spawn(move || platform::run(app, IconCache::new(dir), rx))
        .expect("toast thread");
    tx
}

#[cfg(windows)]
mod platform {
    use super::*;

    /// Windows wants no popups now (fullscreen, presentation, quiet time); see [`state_silences`].
    pub(super) fn busy() -> bool {
        let mut state = 0;
        // SAFETY: an out parameter of the documented call.
        let ok =
            unsafe { windows_sys::Win32::UI::Shell::SHQueryUserNotificationState(&mut state) } >= 0;
        ok && state_silences(state)
    }

    /// Minutes after local midnight.
    pub(super) fn local_minute() -> u16 {
        // SAFETY: fills the struct.
        let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
        t.wHour * 60 + t.wMinute
    }
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{DateTime, PropertyValue, TypedEventHandler};
    use windows::Graphics::Imaging::{
        BitmapDecoder, BitmapEncoder, BitmapInterpolationMode, BitmapPixelFormat, SoftwareBitmap,
    };
    use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    pub(super) fn run(app: tauri::AppHandle, icons: IconCache, jobs: mpsc::Receiver<Job>) {
        // SAFETY: once, on this thread, before any WinRT call here.
        let _ = unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
        };
        let id = match crate::toast::app_id(&app) {
            Ok(id) => HSTRING::from(id),
            Err(error) => {
                eprintln!("notification toasts unavailable: {error}");
                return;
            }
        };
        let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&id) {
            Ok(n) => n,
            Err(error) => {
                eprintln!("notification toasts unavailable: {error}");
                return;
            }
        };
        // An unpackaged app gets a toast's events only while its object lives (WN-44 review): kept until removed/expired.
        let mut alive: std::collections::HashMap<String, (ToastNotification, Instant)> =
            std::collections::HashMap::new();
        for job in jobs {
            alive.retain(|_, (_, at)| at.elapsed() < EXPIRES);
            let result = match job {
                Job::Icon { key, png } => reencode(&png).and_then(|p| {
                    icons.store(&key, &p).map(|_| ()).map_err(|e| {
                        windows::core::Error::new(
                            windows::core::HRESULT(0x8000_4005_u32 as i32),
                            e.to_string(),
                        )
                    })
                }),
                Job::Show {
                    view,
                    popup,
                    received,
                    received_ms,
                } => {
                    let icon = view
                        .icon
                        .as_deref()
                        .and_then(|k| icons.path(k))
                        .filter(|p| p.exists());
                    let shown = show(&app, &notifier, &view, icon.as_deref(), popup).map(|toast| {
                        if alive.len() >= 256 {
                            if let Some(oldest) = alive
                                .iter()
                                .min_by_key(|(_, (_, at))| *at)
                                .map(|(k, _)| k.clone())
                            {
                                alive.remove(&oldest);
                            }
                        }
                        alive.insert(view.tag.clone(), (toast, Instant::now()));
                    });
                    #[cfg(debug_assertions)]
                    if shown.is_ok() {
                        LOG.lock().unwrap().push(DevEntry {
                            tag: view.tag.clone(),
                            popup,
                            received_ms,
                            shown_ms: wall_ms(),
                            show_us: received.elapsed().as_micros() as u64,
                            reply: view.reply.is_some(),
                        });
                    }
                    #[cfg(not(debug_assertions))]
                    let _ = (received, received_ms);
                    shown
                }
                Job::Remove { tag } => {
                    alive.remove(&tag);
                    ToastNotificationManager::History().and_then(|h| {
                        h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(GROUP), &id)
                    })
                }
                Job::Clear => {
                    alive.clear();
                    icons.clear();
                    ToastNotificationManager::History()
                        .and_then(|h| h.RemoveGroupWithId(&HSTRING::from(GROUP), &id))
                }
            };
            if let Err(error) = result {
                eprintln!("notification toast: {error}");
            }
        }
    }

    fn show(
        app: &tauri::AppHandle,
        notifier: &windows::UI::Notifications::ToastNotifier,
        view: &ToastView,
        icon: Option<&Path>,
        popup: bool,
    ) -> windows::core::Result<ToastNotification> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(toast_xml(view, icon)))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        toast.SetTag(&HSTRING::from(&view.tag))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        toast.SetSuppressPopup(!popup)?;
        let expires = SystemTime::now() + EXPIRES;
        let ticks = expires
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64 / 100)
            .unwrap_or(0)
            + 116_444_736_000_000_000;
        let when = PropertyValue::CreateDateTime(DateTime {
            UniversalTime: ticks,
        })?;
        let when: windows::Foundation::IReference<DateTime> =
            windows::core::Interface::cast(&when)?;
        toast.SetExpirationTime(&when)?;
        let opener = app.clone();
        let tag = view.tag.clone();
        toast.Activated(&TypedEventHandler::new(
            move |_, args: windows::core::Ref<windows::core::IInspectable>| {
                let (arguments, input) = args
                    .as_ref()
                    .and_then(|a| {
                        windows::core::Interface::cast::<
                            windows::UI::Notifications::ToastActivatedEventArgs,
                        >(a)
                        .ok()
                    })
                    .map(|a| {
                        let arguments = a.Arguments().map(|s| s.to_string()).unwrap_or_default();
                        let input = a
                            .UserInput()
                            .ok()
                            .and_then(|set| set.Lookup(&HSTRING::from(REPLY_INPUT)).ok())
                            .and_then(|v| {
                                windows::core::Interface::cast::<
                                        windows::Foundation::IPropertyValue,
                                    >(&v)
                                    .ok()
                            })
                            .and_then(|v| v.GetString().ok())
                            .map(|s| s.to_string());
                        (arguments, input)
                    })
                    .unwrap_or_default();
                activated(&opener, &tag, &arguments, input.as_deref());
                Ok(())
            },
        ))?;
        let closed = view.tag.clone();
        toast.Dismissed(
            &TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    windows::UI::Notifications::ToastDismissedEventArgs,
                >| {
                    if let Some(a) = args.as_ref() {
                        dismissed(&closed, a.Reason().map(|r| r.0).unwrap_or(-1));
                    }
                    Ok(())
                },
            ),
        )?;
        notifier.Show(&toast)?;
        Ok(toast)
    }

    /// Decodes the sender's PNG and encodes a fresh 64 px one: Windows never reads the sender's bytes.
    fn reencode(png: &[u8]) -> windows::core::Result<Vec<u8>> {
        let input = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&input)?;
        writer.WriteBytes(png)?;
        writer.StoreAsync()?.get()?;
        writer.DetachStream()?;
        input.Seek(0)?;
        let decoder =
            BitmapDecoder::CreateWithIdAsync(BitmapDecoder::PngDecoderId()?, &input)?.get()?;
        let (w, h) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
        if w == 0 || h == 0 || w > 512 || h > 512 {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8007_000D_u32 as i32,
            )));
        }
        let bitmap = SoftwareBitmap::Convert(
            &decoder.GetSoftwareBitmapAsync()?.get()?,
            BitmapPixelFormat::Bgra8,
        )?;
        let out = InMemoryRandomAccessStream::new()?;
        let encoder = BitmapEncoder::CreateAsync(BitmapEncoder::PngEncoderId()?, &out)?.get()?;
        encoder.SetSoftwareBitmap(&bitmap)?;
        let t = encoder.BitmapTransform()?;
        t.SetScaledWidth(64)?;
        t.SetScaledHeight(64)?;
        t.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
        encoder.FlushAsync()?.get()?;
        let size = out.Size()? as u32;
        let reader = DataReader::CreateDataReader(&out.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut bytes = vec![0u8; size as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }

    #[cfg(debug_assertions)]
    pub(super) fn history(app: &tauri::AppHandle) -> Result<Vec<String>, String> {
        let id = crate::toast::app_id(app)?;
        let list = ToastNotificationManager::History()
            .and_then(|h| h.GetHistoryWithId(&HSTRING::from(id)))
            .map_err(|e| e.to_string())?;
        let mut tags = Vec::new();
        for t in list {
            if t.Group().map(|g| g == GROUP).unwrap_or(false) {
                tags.push(t.Tag().map(|t| t.to_string()).unwrap_or_default());
            }
        }
        Ok(tags)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) fn busy() -> bool {
        false
    }
    pub(super) fn local_minute() -> u16 {
        0
    }
    pub(super) fn run(_: tauri::AppHandle, _: IconCache, jobs: mpsc::Receiver<Job>) {
        for _ in jobs {}
    }
    #[cfg(debug_assertions)]
    pub(super) fn history(_: &tauri::AppHandle) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posted() -> Posted {
        Posted {
            id: [0xab; 16],
            app: "Signal".into(),
            package: "org.thoughtcrime.securesms".into(),
            title: Some("Ola".into()),
            text: Some("Będziesz o 19?".into()),
            time_ms: 0,
            icon: Some([1; 8]),
            reply: None,
            quiet: false,
        }
    }

    #[test]
    fn xml_carries_title_text_app_phone_icon_and_header() {
        let view = ToastView::new(&posted(), "Pixel 9");
        assert_eq!(view.tag, "ab".repeat(16));
        assert_eq!(view.icon.as_deref(), Some("0101010101010101"));
        let x = toast_xml(
            &view,
            Some(Path::new(r"C:\Users\a b\icons\0101010101010101.png")),
        );
        assert!(
            x.contains("<text>Ola</text><text>Będziesz o 19?</text>"),
            "{x}"
        );
        assert!(
            x.contains(r#"<text placement="attribution">Signal · Pixel 9</text>"#),
            "{x}"
        );
        assert!(
            x.contains(r#"src="file:///C:/Users/a b/icons/0101010101010101.png""#),
            "{x}"
        );
        assert!(x.contains(r#"<header id="org.thoughtcrime.securesms" title="Signal" arguments="windconnect-notify"/>"#), "{x}");
    }

    #[test]
    fn untrusted_text_is_escaped_and_cleaned() {
        let n = Posted {
            title: Some("<b>&\"x'</b>".into()),
            text: Some("a\u{202E}gnp.exe\n\n\tb\u{200B}c\u{7}".into()),
            app: "<script>".into(),
            ..posted()
        };
        let view = ToastView::new(&n, "Pix\nel");
        let x = toast_xml(&view, None);
        assert!(
            x.contains("<text>&lt;b&gt;&amp;&quot;x&apos;&lt;/b&gt;</text>"),
            "{x}"
        );
        assert!(x.contains("<text>a gnp.exe b c</text>"), "{x}");
        assert!(!x.contains('\u{202E}') && !x.contains('\u{200B}') && !x.contains('\u{7}'));
        assert!(x.contains("&lt;script&gt; · Pix el"), "{x}");
        assert!(!x.contains("<image"), "no icon file: no image");
    }

    #[test]
    fn missing_title_shows_the_app_and_texts_are_bounded() {
        let n = Posted {
            title: None,
            text: Some("x".repeat(5000)),
            ..posted()
        };
        let view = ToastView::new(&n, "Pixel");
        let x = toast_xml(&view, None);
        assert!(x.contains("<text>Signal</text>"), "{x}");
        assert_eq!(
            view.text.as_ref().unwrap().chars().count(),
            TEXT_CHARS + 1,
            "cut with …"
        );
        let empty = ToastView::new(
            &Posted {
                title: Some(" \n".into()),
                text: Some("\u{200B}".into()),
                app: "".into(),
                ..posted()
            },
            "",
        );
        assert_eq!(empty.title, None);
        assert_eq!(empty.text, None);
        assert_eq!(
            empty.app, "org.thoughtcrime.securesms",
            "no label: the package"
        );
    }

    #[test]
    fn at_most_five_popups_in_two_seconds() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut b = Burst::default();
        let admitted: Vec<bool> = (0..30).map(|i| b.admit(ms(i * 30))).collect();
        assert_eq!(
            admitted.iter().filter(|a| **a).count(),
            5,
            "30 in under a second: 5 pop up"
        );
        assert!(admitted[..5].iter().all(|a| *a));
        assert!(b.admit(ms(2_000)), "the window moved on");
        assert!(b.admit(ms(2_100)));
    }

    #[test]
    fn updates_and_bursts_go_quietly() {
        let (tx, rx) = mpsc::channel();
        let mut n = Notifications::new(tx, "Pixel");
        n.apply(Message::Posted(posted()));
        n.apply(Message::Posted(Posted {
            text: Some("edited".into()),
            ..posted()
        }));
        let popups: Vec<bool> = rx
            .try_iter()
            .map(|j| match j {
                Job::Show { popup, .. } => popup,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(popups, [true, false], "an update replaces quietly");
        n.apply(Message::Removed { id: [0xab; 16] });
        n.apply(Message::Posted(posted()));
        let jobs: Vec<Job> = rx.try_iter().collect();
        assert!(matches!(&jobs[0], Job::Remove { tag } if tag == &"ab".repeat(16)));
        assert!(
            matches!(&jobs[1], Job::Show { popup: true, .. }),
            "new again after removal"
        );
        for i in 0..10u8 {
            n.apply(Message::Posted(Posted {
                id: [i; 16],
                ..posted()
            }));
        }
        let popups = rx
            .try_iter()
            .filter(|j| matches!(j, Job::Show { popup: true, .. }))
            .count();
        assert_eq!(popups, 3, "5 in the window, 2 already used");
    }

    #[test]
    fn icon_cache_names_by_hex_and_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IconCache::new(dir.path().join("notify-icons"));
        assert!(cache.path("../../evil").is_none());
        assert!(cache.path("0101010101010101").is_some());
        assert!(
            cache.path("ABCDEF0123456789").is_none(),
            "lower-case hex only"
        );
        let first = cache.store(&[0; 8], b"png").unwrap();
        assert_eq!(first.file_name().unwrap(), "0000000000000000.png");
        std::thread::sleep(Duration::from_millis(20));
        for i in 1..=(ICONS as u16 + 3) {
            let mut key = [0u8; 8];
            key[6..].copy_from_slice(&i.to_be_bytes());
            cache.store(&key, b"png").unwrap();
        }
        let count = std::fs::read_dir(dir.path().join("notify-icons"))
            .unwrap()
            .count();
        assert_eq!(count, ICONS);
        assert!(!first.exists(), "the oldest went first");
        cache.clear();
        assert!(!dir.path().join("notify-icons").exists());
    }

    #[test]
    fn a_reply_field_only_when_it_can_be_replied_to() {
        let plain = toast_xml(&ToastView::new(&posted(), "Pixel 9"), None);
        assert!(!plain.contains("<actions>"), "{plain}");
        let n = Posted {
            reply: Some("Odpowiedz\u{202E}<x>".into()),
            ..posted()
        };
        let view = ToastView::new(&n, "Pixel 9");
        assert_eq!(
            view.reply.as_deref(),
            Some("Odpowiedz <x>"),
            "bidi control becomes a space"
        );
        let x = toast_xml(&view, None);
        assert!(
            x.contains(r#"<input id="reply" type="text" placeHolderContent="Odpowiedz…"/>"#),
            "{x}"
        );
        assert!(
            x.contains(r#"arguments="reply" hint-inputId="reply" activationType="background""#),
            "{x}"
        );
        assert!(
            x.find("</visual>").unwrap() < x.find("<actions>").unwrap(),
            "actions after visual: {x}"
        );
        let blank = Posted {
            reply: Some("\n\u{200B}".into()),
            ..posted()
        };
        assert_eq!(ToastView::new(&blank, "P").reply, None);
    }

    #[test]
    fn toast_activation_by_arguments_and_input() {
        assert_eq!(activation("", None), Activation::OpenPanel);
        assert_eq!(
            activation("windconnect-notify", Some("x")),
            Activation::OpenPanel
        );
        assert_eq!(
            activation("reply", Some("  Tak, o 19 ")),
            Activation::Reply("Tak, o 19".into())
        );
        assert_eq!(activation("reply", Some(" \t")), Activation::Empty);
        assert_eq!(activation("reply", None), Activation::Empty);
        let max = "ą".repeat(wind_core::notify::MAX_REPLY / 2);
        assert_eq!(
            activation("reply", Some(&max)),
            Activation::Reply(max.clone())
        );
        let over = format!("{max}x");
        assert_eq!(
            activation("reply", Some(&over)),
            Activation::TooLong(over.clone()),
            "never trimmed"
        );
    }

    #[test]
    fn only_the_persons_close_dismisses_on_the_phone() {
        assert!(dismissal_syncs(0), "UserCanceled");
        assert!(!dismissal_syncs(1), "ApplicationHidden: removed by us");
        assert!(!dismissal_syncs(2), "TimedOut: still in Action Center");
        assert!(!dismissal_syncs(-1));
    }

    #[test]
    fn tags_back_to_ids() {
        assert_eq!(id_of(&"ab".repeat(16)), Some([0xab; 16]));
        assert_eq!(id_of(&"AB".repeat(16)), None);
        assert_eq!(id_of("ab"), None);
        assert_eq!(id_of(&"zz".repeat(16)), None);
    }

    #[test]
    fn the_panel_list_follows_the_phone() {
        let (tx, rx) = mpsc::channel();
        let mut n = Notifications::new(tx, "Pixel 9");
        let with = |i: u8, reply: bool| Posted {
            id: [i; 16],
            reply: reply.then(|| "Odpowiedz".into()),
            ..posted()
        };
        for i in 0..(PANEL_ITEMS as u8 + 5) {
            n.apply(Message::Posted(with(i, i % 2 == 0)));
        }
        let items = n.items();
        assert_eq!(items.len(), PANEL_ITEMS);
        assert_eq!(
            items[0].tag,
            hex(&[PANEL_ITEMS as u8 + 4; 16]),
            "newest first"
        );
        assert_eq!(items[0].reply.as_deref(), Some("Odpowiedz"));
        assert_eq!(items[1].reply, None);
        // Only ids shown in this session are acted on.
        let tag = hex(&[24; 16]);
        assert_eq!(n.known(&tag), Some([24; 16]));
        assert_eq!(n.known(&hex(&[99; 16])), None);
        // A reply: queued, then the phone's answer; an update keeps the status.
        n.reply_sent(&tag, true);
        assert_eq!(n.items()[0].status, Some("sending"));
        n.apply(Message::Replied {
            id: [24; 16],
            ok: true,
        });
        assert_eq!(n.items()[0].status, Some("sent"));
        n.apply(Message::Posted(with(24, true)));
        assert_eq!(n.items()[0].status, Some("sent"));
        n.apply(Message::Replied {
            id: [24; 16],
            ok: false,
        });
        assert_eq!(n.items()[0].status, Some("failed"));
        // Too long from a toast: kept as a draft for the panel.
        n.too_long(&tag, "długi".into());
        assert_eq!(n.items()[0].draft.as_deref(), Some("długi"));
        assert_eq!(n.items()[0].status, Some("too-long"));
        n.reply_sent(&tag, true);
        assert_eq!(n.items()[0].draft, None, "not kept after sending");
        // Removed on the phone, or dismissed here: off the list (and the toast).
        n.apply(Message::Removed { id: [24; 16] });
        assert!(n.items().iter().all(|i| i.tag != tag));
        let other = hex(&[23; 16]);
        let _ = rx.try_iter().count();
        n.dismiss(&other);
        assert!(n.items().iter().all(|i| i.tag != other));
        assert_eq!(n.known(&other), None, "acted on once");
        assert!(rx
            .try_iter()
            .any(|j| matches!(j, Job::Remove { tag } if tag == other)));
    }

    #[test]
    fn quiet_hours_by_time_of_day() {
        let q = |from: u16, to: u16| QuietHours { on: true, from, to };
        let night = q(22 * 60, 7 * 60);
        for (minute, inside) in [
            (21 * 60 + 59, false),
            (22 * 60, true),
            (23 * 60 + 59, true),
            (0, true),
            (6 * 60 + 59, true),
            (7 * 60, false),
            (12 * 60, false),
        ] {
            assert_eq!(
                night.contains(minute),
                inside,
                "through midnight at {minute}"
            );
        }
        let day = q(9 * 60, 17 * 60);
        assert!(
            !day.contains(8 * 60 + 59)
                && day.contains(9 * 60)
                && day.contains(16 * 60 + 59)
                && !day.contains(17 * 60)
        );
        assert!(!q(600, 600).contains(600), "from == to is none");
        assert!(!QuietHours { on: false, ..night }.contains(23 * 60), "off");
        // A time of day: the same minute is quiet on a DST day as on any other (no timestamps involved).
        assert!(night.contains(2 * 60 + 30));
        assert!(night.valid() && !q(1440, 0).valid() && !q(22 * 60 + 7, 0).valid());
    }

    #[test]
    fn notify_json_migrates_and_survives_damage() {
        assert_eq!(NotifySettings::parse(None), NotifySettings::default());
        assert!(NotifySettings::default().enabled && !NotifySettings::default().quiet.on);
        // WN-43's bare bool.
        assert_eq!(
            NotifySettings::parse(Some(b"false")),
            NotifySettings {
                enabled: false,
                ..Default::default()
            }
        );
        assert_eq!(
            NotifySettings::parse(Some(b"true")),
            NotifySettings::default()
        );
        // Damaged -> defaults (notifications on, quiet hours off).
        for bad in [&b"{\"enab"[..], b"", b"42", b"\xff\xfe"] {
            assert_eq!(NotifySettings::parse(Some(bad)), NotifySettings::default());
        }
        // Invalid hours -> default hours, the rest kept.
        let odd = br#"{"enabled":false,"quiet":{"on":true,"from":5000,"to":3}}"#;
        assert_eq!(
            NotifySettings::parse(Some(odd)),
            NotifySettings {
                enabled: false,
                quiet: QuietHours::default()
            }
        );
        // Saved whole and read back; no temporary file left.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notify.json");
        std::fs::write(&path, b"true").unwrap();
        let s = NotifySettings {
            enabled: true,
            quiet: QuietHours {
                on: true,
                from: 23 * 60,
                to: 6 * 60 + 30,
            },
        };
        s.save(&path).unwrap();
        assert_eq!(NotifySettings::load(&path), s);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn windows_busy_states_silence() {
        // QUNS_: 1 not present, 2 busy, 3 D3D fullscreen, 4 presentation, 5 accepts, 6 quiet time, 7 app.
        let silences: Vec<i32> = (0..9).filter(|s| state_silences(*s)).collect();
        assert_eq!(silences, [2, 3, 4, 6]);
    }

    #[test]
    fn quiet_ones_go_silently_and_keep_the_burst() {
        let (tx, rx) = mpsc::channel();
        let quiet_here = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = quiet_here.clone();
        let mut n = Notifications::new(tx, "Pixel 9")
            .silenced_by(move || flag.load(std::sync::atomic::Ordering::SeqCst));
        let with = |i: u8, quiet: bool| Posted {
            id: [i; 16],
            quiet,
            ..posted()
        };
        // Quiet on the phone (Do not disturb): no popup, no sound.
        n.apply(Message::Posted(with(1, true)));
        // Quiet here (quiet hours / Focus).
        quiet_here.store(true, std::sync::atomic::Ordering::SeqCst);
        n.apply(Message::Posted(with(2, false)));
        quiet_here.store(false, std::sync::atomic::Ordering::SeqCst);
        // Neither used up the burst: five popups still follow.
        for i in 3..9 {
            n.apply(Message::Posted(with(i, false)));
        }
        let shows: Vec<(bool, bool, String)> = rx
            .try_iter()
            .filter_map(|j| match j {
                Job::Show { view, popup, .. } => Some((popup, view.silent, toast_xml(&view, None))),
                _ => None,
            })
            .collect();
        assert_eq!(
            shows.iter().map(|s| s.0).collect::<Vec<_>>(),
            [false, false, true, true, true, true, true, false]
        );
        for (popup, silent, xml) in &shows {
            assert_eq!(*silent, !popup);
            assert_eq!(xml.contains(r#"<audio silent="true"/>"#), !popup, "{xml}");
        }
        // Still in the panel; a silent toast's close is never the person's own (only UserCanceled syncs, WN-44).
        assert_eq!(n.items().len(), 8);
        for reason in [1, 2] {
            assert!(!dismissal_syncs(reason));
        }
    }
}
