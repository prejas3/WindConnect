//! CCS/1 M:WN-43 the phone's notifications as native Windows toasts (PROTOCOL.md §9.7, notify/1 in).
//! FLOW:session loop decodes -> [`Notifications::apply`] (popup or not) -> one worker thread (order kept) -> ToastNotifier.
//! SEC:texts come from other apps on the phone: sanitised again here (one line, no control/bidi/zero-width characters,
//! bounded) and XML-escaped; icons are decoded and re-encoded as 64 px PNG before Windows reads them, stored as
//! `<key hex>.png` only. Nothing is logged. Windows keeps toast content in its notification store: each expires after
//! [`EXPIRES`], and the setting turned off or an unpairing removes them and the icon cache.
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
        }
    }

    /// Folds one message in (already decoded and bounded by the core).
    pub fn apply(&mut self, message: Message) {
        let received = Instant::now();
        let job = match message {
            Message::Icon { key, png } => Job::Icon { key, png },
            Message::Posted(n) => {
                let update = !self.shown.insert(n.id);
                // An update replaces its toast quietly; a burst beyond the limit goes to Action Center only.
                let popup = !update && self.burst.admit(received);
                Job::Show {
                    view: ToastView::new(&n, &self.phone),
                    popup,
                    received,
                    received_ms: wall_ms(),
                }
            }
            Message::Removed { id } => {
                self.shown.remove(&id);
                Job::Remove { tag: hex(&id) }
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

/// Starts the worker; icons live in `dir`.
pub fn worker(app: &tauri::AppHandle, dir: PathBuf) -> mpsc::Sender<Job> {
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
        for job in jobs {
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
                    let shown = show(&app, &notifier, &view, icon.as_deref(), popup);
                    #[cfg(debug_assertions)]
                    if shown.is_ok() {
                        LOG.lock().unwrap().push(DevEntry {
                            tag: view.tag.clone(),
                            popup,
                            received_ms,
                            shown_ms: wall_ms(),
                            show_us: received.elapsed().as_micros() as u64,
                        });
                    }
                    #[cfg(not(debug_assertions))]
                    let _ = (received, received_ms);
                    shown
                }
                Job::Remove { tag } => ToastNotificationManager::History().and_then(|h| {
                    h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(GROUP), &id)
                }),
                Job::Clear => {
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
    ) -> windows::core::Result<()> {
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
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            let panel = opener.clone();
            let _ = opener.run_on_main_thread(move || {
                let _ = crate::panel::show(&panel);
            });
            Ok(())
        }))?;
        notifier.Show(&toast)
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
}
