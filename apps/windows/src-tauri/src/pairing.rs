use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, watch};
use wind_core::{
    identity::{DeviceIdentity, DpapiKeyStore, PublicKey},
    network::{NetworkChanges, NetworkDiscovery},
    pairing::{
        clean_name, PairedDevicesFile, PairingConfig, PairingEvent, PairingListener, Unpairing,
        WindowEnd,
    },
    protocol::{DeviceKind, ErrorCode},
    session::{admit, close_code, turn_away, Admit, LocalInfo, Router, Session, TurnedAway},
    transport::{Endpoint, PinnedKeys},
};

/// The modules of the pairing QR, which encode the window's one-time secret.
///
/// CCS/1 SEC:every copy is wiped on drop (WN-94): the loop's snapshot, the
/// watch value (send_replace drops the old one), publish's clones and
/// pairing_snapshot's clone. Closing, refreshing or expiry sets qr = None, so
/// no app-held copy outlives its window. Beyond reach: the JSON that Tauri
/// serializes for IPC (emit and command reply) and the WebView's copy (JS
/// heap, GC); the panel zeroes its array and clears the canvas.
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct QrBits {
    pub width: usize,
    pub dark: Vec<bool>,
}
impl QrBits {
    fn wipe(&mut self) {
        use zeroize::Zeroize;
        self.dark.zeroize();
        self.width.zeroize();
    }
}
impl Drop for QrBits {
    fn drop(&mut self) {
        self.wipe();
    }
}
/// Overwrites the modules `qrcode` handed over (its `Color` is not Zeroize).
fn wipe_colors(colors: &mut [qrcode::Color]) {
    for c in colors.iter_mut() {
        // SAFETY: `c` is a valid, aligned, exclusive reference; a volatile
        // write keeps the compiler from dropping a store to memory about to
        // be freed.
        unsafe { std::ptr::write_volatile(c, qrcode::Color::Light) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct Snapshot {
    pub touchpad: crate::touchpad::View,
    pub phase: String,
    pub phone: Option<String>,
    pub qr: Option<QrBits>,
    pub message: String,
    pub skipped_records: usize,
    pub pair_to_connected_ms: Option<u64>,
    /// SAS (§12.6): the code to compare with the phone, while it waits for a decision.
    pub code: Option<String>,
    /// A phone has the session now (also while another one is being paired).
    pub connected: bool,
    /// Paired phones and those still owed an unpair notice (WC-28).
    pub devices: Vec<DeviceRow>,
    /// The connected phone's battery (WN-49), pushed by the phone; None without a session or report.
    pub battery: Option<crate::battery::Battery>,
    /// Find my phone (WN-50).
    pub find: FindView,
    /// WN-55: the phone offers its SMS (sms/1): the panel's "Wiadomości" button.
    pub sms: bool,
    /// WN-100: the phone takes links (actions/1): the panel's "Otwórz na telefonie".
    pub open_on_phone: bool,
    /// The phone's player (WN-46); None without a session, access or player.
    pub media: Option<crate::media::MediaView>,
    /// WN-47: "Udostępnij odtwarzanie telefonowi" (the PC's player on the phone).
    pub share_media: bool,
    /// WN-43: "Powiadomienia z telefonu" (the phone's notifications as toasts; notify/1).
    pub phone_notifications: bool,
    /// WN-45: "Ciche godziny" for the phone's notifications.
    pub quiet_hours: crate::notify::QuietHours,
    /// WN-48: "Wstrzymaj odtwarzanie przy połączeniu", "Głośność z telefonu", and the phone's volume (none without one).
    pub pause_on_call: bool,
    pub volume_remote: bool,
    /// WN-57: "Połączenia z telefonu", and the phone's missed calls for the panel (newest first).
    pub call_toasts: bool,
    pub missed_calls: Vec<crate::calls::MissedView>,
    pub phone_volume: Option<crate::pc_volume::PhoneVolume>,
    /// WN-44: the phone's notifications in the panel, newest first (empty without a session).
    pub notifications: Vec<crate::notify::PanelItem>,
    /// A phone is paired: [`has_paired_in`], the rule the first window
    /// follows too (WN-113). A phone owed an unpair notice doesn't count.
    pub paired: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            touchpad: crate::touchpad::View {
                allowed: true,
                ..Default::default()
            },
            phase: "loading".into(),
            phone: None,
            qr: None,
            message: crate::i18n::tr("pairing.preparing").into(),
            skipped_records: 0,
            pair_to_connected_ms: None,
            code: None,
            connected: false,
            devices: Vec::new(),
            battery: None,
            find: FindView::default(),
            sms: false,
            open_on_phone: false,
            media: None,
            share_media: true,
            phone_notifications: true,
            quiet_hours: crate::notify::QuietHours::default(),
            pause_on_call: true,
            volume_remote: true,
            call_toasts: true,
            missed_calls: Vec::new(),
            phone_volume: None,
            notifications: Vec::new(),
            paired: false,
        }
    }
}
/// The "Dzwoń" tile (WN-50): whether the phone takes `find/1`, the ring now, and how the last one ended.
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct FindView {
    pub available: bool,
    /// "idle", "starting" or "ringing".
    pub state: &'static str,
    /// How the last ring ended, for the status line.
    pub last: Option<String>,
    /// Click to the phone's "ringing" (its sound already playing), in ms.
    pub ring_ms: Option<u64>,
}

impl Default for FindView {
    fn default() -> Self {
        Self {
            available: false,
            state: "idle",
            last: None,
            ring_ms: None,
        }
    }
}

/// The status line for a ring that ended with `end` (None: the stream or session broke).
pub fn find_outcome(end: Option<wind_core::find::State>) -> &'static str {
    use wind_core::find::State;
    match end {
        Some(State::StoppedHere) => crate::i18n::tr("find.stopped_on_phone"),
        Some(State::StoppedByRequest) => crate::i18n::tr("find.stopped"),
        Some(State::TimedOut) => crate::i18n::tr("find.timed_out"),
        Some(State::Unavailable) => crate::i18n::tr("find.unavailable"),
        Some(State::Ringing) | None => crate::i18n::tr("find.broken"),
    }
}

enum Action {
    Touchpad(bool),
    /// WN-47: share the PC's player with the phone, or stop.
    ShareMedia(bool),
    /// WN-43: show the phone's notifications, or stop (and remove those shown).
    PhoneNotifications(bool),
    /// WN-48: the call and volume settings (only what is given).
    CallVolume {
        pause: Option<bool>,
        volume: Option<bool>,
        calls: Option<bool>,
    },
    /// WN-48: the panel changes the phone's volume (only what is given).
    PhoneVolume(Option<u8>, Option<bool>),
    /// WN-45: the quiet hours changed.
    QuietHours(crate::notify::QuietHours),
    /// WN-44: a reply typed in the panel for one of the phone's notifications (by its tag).
    NotifyReply(String, String),
    /// WN-44: ✕ in the panel: dismiss it here and on the phone.
    NotifyDismiss(String),
    /// A button of the phone's player in the panel (WN-46).
    Media(wind_core::media::Action),
    /// Find my phone (WN-50): ring it, or stop the ring.
    Ring,
    StopRing,
    Refresh,
    Close,
    /// Pair without a camera: open a SAS window (WN-27).
    Sas,
    /// The person's answer to the shown code.
    Decide(bool),
    /// Device list (WC-28): a local name, removal on both sides, ending the session.
    Rename(PublicKey, String),
    Unpair(PublicKey),
    Disconnect(PublicKey),
}
struct AcceptTask(tokio::task::JoinHandle<()>);
impl Drop for AcceptTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub struct Runtime {
    state: watch::Sender<Snapshot>,
    commands: mpsc::UnboundedSender<Action>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Action>>>,
    worker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    main_address: Mutex<Option<SocketAddr>>,
}
impl Default for Runtime {
    fn default() -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        Self {
            state: watch::channel(Snapshot::default()).0,
            commands,
            receiver: Mutex::new(Some(receiver)),
            worker: Mutex::new(None),
            main_address: Mutex::new(None),
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            worker.abort();
        }
    }
}

fn publish(app: &tauri::AppHandle, state: &watch::Sender<Snapshot>, value: Snapshot) {
    if *state.borrow() == value {
        return;
    }
    state.send_replace(value.clone());
    // The main window shows the same single pairing window during onboarding (WC-34).
    let _ = app.emit_to("main", "pairing-state", value.clone());
    let _ = app.emit_to("panel", "pairing-state", value);
}

/// Whether any phone is paired (read once at start; WC-34 picks the first
/// window: onboarding in the main window, else the panel).
pub fn has_paired() -> bool {
    data_dir().is_ok_and(|dir| has_paired_in(&PairedDevicesFile::in_dir(dir)))
}
/// "Has a paired phone", one rule for the first window and the onboarding
/// view (WN-113): a phone in `devices`. A removed phone still owed its
/// unpair notice sits in `revoked` and is already unpaired for the person.
pub fn has_paired_in(store: &PairedDevicesFile) -> bool {
    store.load().is_ok_and(|list| !list.devices.is_empty())
}
fn addresses(port: u16) -> std::io::Result<Vec<SocketAddr>> {
    // CCS/1 RULE:LAN IPv4 only, default-route interface first; no network packet is sent by UDP connect.
    let preferred = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        })
        .ok()
        .map(|a| a.ip());
    let mut ips: Vec<_> = if_addrs::get_if_addrs()?
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| matches!(ip, IpAddr::V4(a) if !a.is_unspecified() && !a.is_link_local()))
        .collect();
    ips.sort_unstable();
    ips.dedup();
    ips.sort_by_key(|ip| Some(*ip) != preferred);
    #[cfg(debug_assertions)]
    if let Ok(address) = std::env::var("WINDCONNECT_TEST_PAIR_IP") {
        if std::env::var_os("WINDCONNECT_TEST_DATA_DIR").is_some() {
            ips = vec![address.parse().map_err(std::io::Error::other)?];
        }
    }
    Ok(ips
        .into_iter()
        .take(8)
        .map(|ip| SocketAddr::new(ip, port))
        .collect())
}
fn data_store() -> std::io::Result<DpapiKeyStore> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("WINDCONNECT_TEST_DATA_DIR") {
        return Ok(DpapiKeyStore::new(
            std::path::PathBuf::from(path).join("identity.key"),
        ));
    }
    DpapiKeyStore::in_user_profile()
}
/// The folder of the identity key, paired devices and actions.json.
pub fn data_dir() -> std::io::Result<std::path::PathBuf> {
    let store = data_store()?;
    store
        .path()
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| std::io::Error::other("identity directory missing"))
}
fn open_window(
    listener: &PairingListener,
) -> Result<QrBits, Box<dyn std::error::Error + Send + Sync>> {
    listener.close_window();
    let addresses = addresses(listener.local_addr()?.port())?;
    if addresses.is_empty() {
        return Err(crate::i18n::tr("pairing.no_network").into());
    }
    let offer = listener.open_qr(addresses);
    let link = offer.to_link();
    // `link` and the offer's secret are Zeroizing (wind-core). The encoder's
    // own working buffers are dropped inside qrcode, beyond reach.
    let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M)?;
    Ok(qr_bits(code))
}
/// The modules of `code`, its module buffer wiped once copied.
fn qr_bits(code: qrcode::QrCode) -> QrBits {
    let width = code.width();
    let mut colors = code.into_colors();
    let dark = colors.iter().map(|c| *c == qrcode::Color::Dark).collect();
    wipe_colors(&mut colors);
    QrBits { width, dark }
}

pub fn start(app: tauri::AppHandle, changes: Arc<NetworkChanges>) {
    let runtime = app.state::<Runtime>();
    let receiver = runtime
        .receiver
        .lock()
        .unwrap()
        .take()
        .expect("pairing started once");
    let state = runtime.state.clone();
    let handle = app.clone();
    let worker = tauri::async_runtime::spawn(async move {
        if run(handle.clone(), changes, state.clone(), receiver)
            .await
            .is_err()
        {
            publish(
                &handle,
                &state,
                Snapshot {
                    phase: "failed".into(),
                    message: crate::i18n::tr("pairing.start_failed").into(),
                    ..Default::default()
                },
            );
        }
    });
    *runtime.worker.lock().unwrap() = Some(worker);
}

/// A paired phone as the panel lists it (WC-28).
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct DeviceRow {
    /// Its key in hex: how the panel names it in `device_action`.
    pub key: String,
    /// The local name (alias) or the name it gave when pairing.
    pub name: String,
    /// The SAS-style fingerprint, for telling two phones apart.
    pub fingerprint: String,
    /// Seconds since the Unix epoch: paired, or (owed) removed.
    pub paired_at: u64,
    pub connected: bool,
    /// Removed here; the phone learns it on its next connection (§12.10).
    pub owed: bool,
}

fn key_hex(key: &PublicKey) -> String {
    key.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_key(hex: &str) -> Option<PublicKey> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    PublicKey::from_bytes(&bytes)
}

/// The list the panel shows: paired phones, then those still owed a notice.
fn device_rows(store: &PairedDevicesFile, connected: Option<&PublicKey>) -> Vec<DeviceRow> {
    let Ok(list) = store.load() else {
        return Vec::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let owed = list.owed(now);
    let mut rows: Vec<DeviceRow> = list
        .devices
        .iter()
        .map(|d| DeviceRow {
            key: key_hex(&d.key),
            name: d
                .name
                .clone()
                .unwrap_or_else(|| crate::i18n::tr("common.phone_capital").into()),
            fingerprint: d.key.fingerprint().to_string(),
            paired_at: d.paired_at,
            connected: connected == Some(&d.key),
            owed: false,
        })
        .collect();
    rows.extend(
        list.revoked
            .iter()
            .filter(|(k, _)| owed.contains(k))
            .map(|(k, at)| DeviceRow {
                key: key_hex(k),
                name: crate::i18n::tr("pairing.removed_phone").into(),
                fingerprint: k.fingerprint().to_string(),
                paired_at: *at,
                connected: false,
                owed: true,
            }),
    );
    rows
}

/// The local name of `key` (its alias), else what it said in `Hello`.
fn shown_name(store: &PairedDevicesFile, session: &Session) -> Option<String> {
    store
        .load()
        .ok()
        .and_then(|l| l.get(&session.peer().key).and_then(|d| d.name.clone()))
        .or_else(|| clean_name(&session.peer().hello.name))
}

/// How long a freshly paired phone may take over the session from the
/// connected one: pairing at the PC is the person's explicit choice.
const PAIRED_TAKES_OVER: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a phone disconnected from the panel is told BUSY when it comes
/// back by itself, so another phone can take the session (WC-28).
const DISMISSED_FOR: std::time::Duration = std::time::Duration::from_secs(600);

type Incoming = (
    Session,
    mpsc::UnboundedReceiver<wind_core::session::SessionEvent>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    mpsc::Receiver<wind_core::session::ActionStream>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    Option<mpsc::Receiver<wind_core::session::FileStream>>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    // WN-48: call/1 and volume/1 from the phone.
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    mpsc::Receiver<wind_core::session::ChannelMessage>,
    mpsc::Receiver<wind_core::input::InputEvent>,
);

async fn run(
    app: tauri::AppHandle,
    changes: Arc<NetworkChanges>,
    state: watch::Sender<Snapshot>,
    mut commands: mpsc::UnboundedReceiver<Action>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key_store = data_store()?;
    let store = Arc::new(PairedDevicesFile::in_dir(
        key_store
            .path()
            .parent()
            .ok_or("identity directory missing")?,
    ));
    let identity = Arc::new(DeviceIdentity::load_or_create(&key_store)?);
    // WN-47: the setting, read once; the accept task announces media-control/1 by it.
    let share_path = key_store.path().parent().map(|d| d.join("pc-media.json"));
    let share_media = Arc::new(std::sync::atomic::AtomicBool::new(
        share_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<bool>(&b).ok())
            .unwrap_or(true),
    ));
    let accept_share = share_media.clone();
    // WN-43: the phone's notifications as toasts (default on: the phone side asks for consent, WN-42).
    let notify_path = key_store.path().parent().map(|d| d.join("notify.json"));
    // WN-45: with the quiet hours (an old bare `true`/`false` file still reads; a damaged one gives the defaults).
    let notify_settings = notify_path
        .as_deref()
        .map(crate::notify::NotifySettings::load)
        .unwrap_or_default();
    let phone_notifications = Arc::new(std::sync::atomic::AtomicBool::new(notify_settings.enabled));
    let quiet_hours = Arc::new(std::sync::Mutex::new(notify_settings.quiet));
    let save_notify = {
        let (path, enabled, quiet) = (
            notify_path.clone(),
            phone_notifications.clone(),
            quiet_hours.clone(),
        );
        move || {
            if let Some(path) = &path {
                let settings = crate::notify::NotifySettings {
                    enabled: enabled.load(std::sync::atomic::Ordering::SeqCst),
                    quiet: *quiet.lock().unwrap(),
                };
                let _ = settings.save(path);
            }
        }
    };
    let accept_notify = phone_notifications.clone();
    // WN-48: "Wstrzymaj odtwarzanie przy połączeniu" and "Głośność z telefonu" (both on by default).
    let call_volume_path = key_store
        .path()
        .parent()
        .map(|d| d.join("call-volume.json"));
    let call_volume = call_volume_path
        .as_deref()
        .map(crate::call_pause::CallVolumeSettings::load)
        .unwrap_or_default();
    let pause_on_call = Arc::new(std::sync::atomic::AtomicBool::new(
        call_volume.pause_on_call,
    ));
    let volume_remote = Arc::new(std::sync::atomic::AtomicBool::new(call_volume.volume));
    let call_toasts = Arc::new(std::sync::atomic::AtomicBool::new(call_volume.calls));
    let (accept_call, accept_toasts, accept_volume) = (
        pause_on_call.clone(),
        call_toasts.clone(),
        volume_remote.clone(),
    );
    // WN-44: replies and closes from the toasts come back here.
    let (toast_events_tx, mut toast_events) =
        mpsc::unbounded_channel::<crate::notify::ToastEvent>();
    let toasts = crate::notify::worker(
        &app,
        key_store
            .path()
            .parent()
            .ok_or("identity directory missing")?
            .join("notify-icons"),
        toast_events_tx,
    );
    let trust = Arc::new(PinnedKeys::new());
    let loaded = store.load()?;
    loaded.pin_all(&trust);
    if let Some(dir) = key_store.path().parent() {
        if let Err(error) = crate::clipboard::init(&app, dir) {
            eprintln!("clipboard unavailable: {error}");
        }
        if let Err(error) = crate::files::init(&app, dir) {
            eprintln!("file sending unavailable: {error}");
        }
    }
    if let Some(device) = loaded.devices.last() {
        crate::files::set_target(
            &app,
            *device.key.fingerprint().as_bytes(),
            device.name.clone(),
        );
    }
    let endpoint = Endpoint::bind("0.0.0.0:0".parse()?, &identity, trust.clone())?;
    *app.state::<Runtime>().main_address.lock().unwrap() = Some(endpoint.local_addr()?);
    // WN-92: the relay setting (off by default) keeps this PC reachable away from home.
    if let Some(dir) = key_store.path().parent() {
        crate::relay::start(&app, dir, identity.clone(), endpoint.clone(), store.clone());
    }
    let unpairing = Unpairing::new((*store).clone(), trust.clone(), endpoint.clone());
    let mut discovery =
        NetworkDiscovery::start(identity.public_key(), endpoint.clone(), changes.clone())?;
    let name = clean_name(&std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Komputer".into()))
        .unwrap_or_else(|| "Komputer".into());
    let local = LocalInfo {
        name: name.clone(),
        kind: DeviceKind::Computer,
        app: Some(wind_core::VERSION.into()),
    };
    // CCS/1 RULE:accept/HELLO is owned outside select, so refresh/network events cannot cancel an in-flight TLS handshake or block the UI actor.
    // The loop admits (one session at a time, WC-28) before anything is attached.
    let (session_sender, mut sessions) = mpsc::channel::<Incoming>(1);
    // Status lines from tasks that settle or receive unpair notices (§12.10).
    let (notice_sender, mut notices) = mpsc::unbounded_channel::<String>();
    let loop_notices = notice_sender.clone();
    let accepting = endpoint.clone();
    let files_enabled = crate::files::receiving_enabled(&app);
    let touchpad_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|p| p.join("touchpad-enabled"));
    let touchpad_allowed = Arc::new(std::sync::atomic::AtomicBool::new(
        touchpad_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .as_deref()
            != Some("false"),
    ));
    let accept_touchpad = touchpad_allowed.clone();
    let accept_unpairing = unpairing.clone();
    let _accept_task = AcceptTask(tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            if let Ok(connection) = incoming {
                // CCS/1 SEC:actions/1 accepted from pinned phones only; each request re-checks the pin (WN-54).
                let mut router = Router::new();
                let touchpad_input = router.route_input_version(4, 128);
                if !accept_touchpad.load(std::sync::atomic::Ordering::SeqCst) {
                    router.start_disabled(wind_core::protocol::Channel::Input);
                }
                let clipboard_input = wind_core::clipboard::Clipboard::route(&mut router);
                let action_streams = router.route_actions(8);
                // CCS/1 IO:battery/1 from the phone (§9.4, WN-49): status only, never acted on beyond display and alerts.
                let battery_input = wind_core::battery::route(&mut router);
                // CCS/1 IO:media/1 from the phone (§9.6, WN-46): shown as text and checked images only.
                let media_input = wind_core::media::route_player(&mut router);
                let files_input = files_enabled.then(|| router.route_files(1, 32));
                // CCS/1 IO:media-control/1 from the phone (§9.6, WN-47): commands for the PC's player, checked against its actions.
                let media_commands = wind_core::media::route_control(&mut router);
                if !accept_share.load(std::sync::atomic::Ordering::SeqCst) {
                    router.start_disabled(wind_core::protocol::Channel::MediaControl);
                }
                // CCS/1 IO:notify/1 from the phone (§9.7, WN-43): shown as toasts, texts sanitised, icons re-encoded.
                let notify_input = wind_core::notify::route(&mut router);
                if !accept_notify.load(std::sync::atomic::Ordering::SeqCst) {
                    router.start_disabled(wind_core::protocol::Channel::Notify);
                }
                // CCS/1 IO:call/1 from the phone (§9.10, WN-48/57): the state pauses the PC's players; its caller and the missed
                // calls make toasts. Taken while either setting is on.
                let call_input = wind_core::call::route(&mut router);
                if !accept_call.load(std::sync::atomic::Ordering::SeqCst)
                    && !accept_toasts.load(std::sync::atomic::Ordering::SeqCst)
                {
                    router.start_disabled(wind_core::protocol::Channel::Call);
                }
                // CCS/1 IO:volume/1 both ways (§9.9, WN-48): the phone's level for the panel, its Sets for Core Audio.
                let volume_input = wind_core::volume::route(&mut router);
                if !accept_volume.load(std::sync::atomic::Ordering::SeqCst) {
                    router.start_disabled(wind_core::protocol::Channel::Volume);
                }
                match tokio::time::timeout(
                    std::time::Duration::from_secs(6),
                    Session::accept(connection, local.clone(), router),
                )
                .await
                {
                    Ok(Ok((session, events))) => {
                        if session_sender
                            .send((
                                session,
                                events,
                                clipboard_input,
                                action_streams,
                                battery_input,
                                media_input,
                                files_input,
                                media_commands,
                                notify_input,
                                call_input,
                                volume_input,
                                touchpad_input,
                            ))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    // CCS/1 FLOW:a removed phone came back: notice mode told it UNPAIRED; settle when acked (§12.10).
                    Ok(Err(error)) => {
                        if accept_unpairing.notice_result(&error).unwrap_or(false) {
                            let _ =
                                notice_sender.send(crate::i18n::tr("pairing.removed_told").into());
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    }));
    let mut listener = PairingListener::bind(
        "0.0.0.0:0".parse()?,
        &identity,
        Some(&name),
        store.clone(),
        trust.clone(),
        PairingConfig::default(),
    )?;
    // CCS/1 FLOW:SAS window open -> mDNS TXT pair=<port> -> phone lists this PC -> both show the code (§12.4, §12.6).
    listener.advertise_with(discovery.pairing_advertiser());
    let mut prompt: Option<wind_core::pairing::SasPrompt> = None;
    let indicator = app.state::<crate::tray::TrayRuntime>().indicator.clone();
    let mut snapshot = Snapshot {
        skipped_records: loaded.skipped.len(),
        phone: loaded.devices.last().and_then(|p| p.name.clone()),
        phase: "offline".into(),
        message: crate::i18n::tr("pairing.paired_waiting").into(),
        ..Default::default()
    };
    let mut network = changes.subscribe();
    let mut paired_at = None::<Instant>;
    // The phone paired here last, and when: it may take over the session.
    let mut preferred: Option<(PublicKey, Instant)> = None;
    // The phone disconnected from the panel, and when (BUSY for DISMISSED_FOR).
    let mut dismissed: Option<(PublicKey, Instant)> = None;
    let mut active: Option<(Session, ())> = None;
    let mut touchpad: Option<crate::touchpad::Control> = None;
    // The active session's capability changes (e.g. "Pozwól komputerowi dzwonić", WN-50) wake the loop.
    let (caps_changed, mut caps_changes) = mpsc::unbounded_channel::<()>();
    // WN-48: pausing for calls (lives across sessions), the PC's volume (Core Audio) and the per-session volume tasks' ends.
    let pauser = crate::call_pause::CallPauser::start();
    let (pc_levels_tx, mut pc_levels) = mpsc::unbounded_channel::<crate::pc_volume::Level>();
    let pc_volume = crate::pc_volume::PcVolume::start(pc_levels_tx);
    let (pc_level, _) = watch::channel::<Option<crate::pc_volume::Level>>(None);
    let (volume_on, _) = watch::channel(volume_remote.load(std::sync::atomic::Ordering::SeqCst));
    let (phone_levels_tx, mut phone_levels) = mpsc::unbounded_channel::<(u8, bool)>();
    let mut call_input: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    // WN-57: the live session's call toasts and missed calls.
    let mut calls: Option<crate::calls::Calls> = None;
    let mut phone_sets: Option<mpsc::Sender<(Option<u8>, Option<bool>)>> = None;
    let save_call_volume = {
        let (path, pause, volume, calls) = (
            call_volume_path.clone(),
            pause_on_call.clone(),
            volume_remote.clone(),
            call_toasts.clone(),
        );
        move || {
            if let Some(path) = &path {
                let _ = crate::call_pause::CallVolumeSettings {
                    pause_on_call: pause.load(std::sync::atomic::Ordering::SeqCst),
                    volume: volume.load(std::sync::atomic::Ordering::SeqCst),
                    calls: calls.load(std::sync::atomic::Ordering::SeqCst),
                }
                .save(path);
            }
        }
    };
    // The active session's battery messages and its alerts (WN-49); both end with the session.
    let mut battery_input: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    let mut alerts = wind_core::battery::Alerts::default();
    // The active session's player messages and what they built (WN-46).
    let mut media_input: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    let mut player = crate::media::Player::default();
    // WN-47: the PC's player followed for the live session, and the phone's commands for it.
    let mut pc_media: Option<crate::pc_media::PcMedia> = None;
    let mut media_commands: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    // WN-43: the phone's notifications in the live session (what was shown, the popup limit).
    let mut notify_input: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    let mut notifications: Option<crate::notify::Notifications> = None;
    // The ring asked of the phone, and when it was asked (WN-50).
    let mut ring: Option<(wind_core::find::Ring, Instant)> = None;
    let mut refresh = loaded.devices.is_empty();
    loop {
        if refresh {
            refresh = false;
            match open_window(&listener) {
                Ok(qr) => {
                    snapshot.phase = "waiting".into();
                    snapshot.qr = Some(qr);
                    snapshot.message = crate::i18n::tr("pairing.scan").into();
                }
                Err(_) => {
                    snapshot.phase = "no-network".into();
                    snapshot.qr = None;
                    snapshot.message = crate::i18n::tr("pairing.connect_network").into();
                }
            }
        }
        let active_key = active.as_ref().map(|(s, _): &(Session, _)| s.peer().key);
        snapshot.connected = active_key.is_some();
        if active.is_none() {
            if let Some(control) = touchpad.take() {
                control.stop().await;
            }
            battery_input = None;
            snapshot.battery = None;
            media_input = None;
            player = crate::media::Player::default();
            pc_media = None;
            media_commands = None;
            notify_input = None;
            notifications = None;
            call_input = None;
            calls = None;
            phone_sets = None;
            snapshot.phone_volume = None;
        }
        snapshot.touchpad = touchpad
            .as_ref()
            .map(|c| c.view())
            .unwrap_or(crate::touchpad::View {
                allowed: touchpad_allowed.load(std::sync::atomic::Ordering::SeqCst),
                ..Default::default()
            });
        snapshot.share_media = share_media.load(std::sync::atomic::Ordering::SeqCst);
        snapshot.phone_notifications =
            phone_notifications.load(std::sync::atomic::Ordering::SeqCst);
        snapshot.quiet_hours = *quiet_hours.lock().unwrap();
        snapshot.pause_on_call = pause_on_call.load(std::sync::atomic::Ordering::SeqCst);
        snapshot.volume_remote = volume_remote.load(std::sync::atomic::Ordering::SeqCst);
        snapshot.call_toasts = call_toasts.load(std::sync::atomic::Ordering::SeqCst);
        snapshot.missed_calls = calls.as_ref().map(|c| c.items()).unwrap_or_default();
        let controllable = active.as_ref().is_some_and(|(s, _): &(Session, _)| {
            s.peer_capabilities()
                .iter()
                .any(|c| c.name() == "volume" && c.version() == 1)
        });
        if let Some(v) = &mut snapshot.phone_volume {
            v.controllable = controllable && snapshot.volume_remote;
        }
        snapshot.notifications = notifications
            .as_ref()
            .map(|n| n.items())
            .unwrap_or_default();
        snapshot.media = player.view.clone();
        crate::battery::set(&app, snapshot.battery);
        snapshot.find.available = active.as_ref().is_some_and(|(s, _): &(Session, _)| {
            s.peer_capabilities()
                .iter()
                .any(|c| c.name() == "find" && c.version() == 1)
        });
        snapshot.sms = active.as_ref().is_some_and(|(s, _): &(Session, _)| {
            s.peer_capabilities()
                .iter()
                .any(|c| c.name() == "sms" && c.version() == 1)
        });
        snapshot.open_on_phone = active.as_ref().is_some_and(|(s, _): &(Session, _)| {
            s.peer_capabilities()
                .iter()
                .any(|c| c.name() == "actions" && c.version() == 1)
        });
        snapshot.devices = device_rows(&store, active_key.as_ref());
        snapshot.paired = has_paired_in(&store);
        publish(&app, &state, snapshot.clone());
        tokio::select! {
            _ = async { if let Some(start) = paired_at.filter(|_| snapshot.phase == "connecting") { tokio::time::sleep_until(tokio::time::Instant::from_std(start + std::time::Duration::from_secs(10))).await; } else { std::future::pending().await } } => {
                snapshot.phase = "offline".into(); snapshot.message = crate::i18n::tr("pairing.paired_offline").into();
            },
            command = commands.recv() => match command {
                Some(Action::Touchpad(on)) => {
                    touchpad_allowed.store(on,std::sync::atomic::Ordering::SeqCst);
                    if let Some(path)=&touchpad_path { let _=std::fs::write(path,if on {"true"} else {"false"}); }
                    if let Some(c)=&touchpad {c.set(on).await;}
                }
                Some(Action::ShareMedia(on)) => {
                    share_media.store(on, std::sync::atomic::Ordering::SeqCst);
                    if let Some(path) = &share_path { let _ = std::fs::write(path, if on { b"true" as &[u8] } else { b"false" }); }
                    if let Some((session, _)) = &active {
                        let _ = session.set_accepted(wind_core::protocol::Channel::MediaControl, on).await;
                        // Off: dropping it sends Gone and drops the SMTC subscriptions.
                        pc_media = if on { crate::pc_media::PcMedia::start(session.clone(), tokio::runtime::Handle::current()) } else { None };
                    }
                }
                Some(Action::PhoneNotifications(on)) => {
                    phone_notifications.store(on, std::sync::atomic::Ordering::SeqCst);
                    save_notify();
                    if let Some((session, _)) = &active { let _ = session.set_accepted(wind_core::protocol::Channel::Notify, on).await; }
                    // Off: Windows keeps toast content in its store; remove what was shown and the icons.
                    if !on { let _ = toasts.send(crate::notify::Job::Clear); }
                }
                Some(Action::CallVolume { pause, volume, calls: toasts_setting }) => {
                    if let Some(on) = pause {
                        pause_on_call.store(on, std::sync::atomic::Ordering::SeqCst);
                        // Off during a call: settle what was paused.
                        if !on { pauser.state(wind_core::call::CallState::Idle); }
                    }
                    if let Some(on) = toasts_setting {
                        call_toasts.store(on, std::sync::atomic::Ordering::SeqCst);
                        if !on { if let Some(c) = calls.as_mut() { c.clear(); } }
                    }
                    if pause.is_some() || toasts_setting.is_some() {
                        let take = pause_on_call.load(std::sync::atomic::Ordering::SeqCst) || call_toasts.load(std::sync::atomic::Ordering::SeqCst);
                        if let Some((session, _)) = &active { let _ = session.set_accepted(wind_core::protocol::Channel::Call, take).await; }
                    }
                    if let Some(on) = volume {
                        volume_remote.store(on, std::sync::atomic::Ordering::SeqCst);
                        volume_on.send_replace(on);
                        if let Some((session, _)) = &active { let _ = session.set_accepted(wind_core::protocol::Channel::Volume, on).await; }
                        if !on { snapshot.phone_volume = None; }
                    }
                    save_call_volume();
                }
                Some(Action::PhoneVolume(level, muted)) => {
                    if let Some(sets) = &phone_sets { let _ = sets.try_send((level, muted)); }
                }
                Some(Action::QuietHours(quiet)) => {
                    *quiet_hours.lock().unwrap() = quiet;
                    save_notify();
                }
                Some(Action::NotifyReply(tag, text)) => {
                    notify_reply(active.as_ref().map(|(s, _)| s), notifications.as_mut(), &tag, text).await;
                }
                Some(Action::NotifyDismiss(tag)) => {
                    notify_dismiss(active.as_ref().map(|(s, _)| s), notifications.as_mut(), &tag).await;
                }
                Some(Action::Media(action)) => {
                    if let Some((session, _)) = &active {
                        // The phone ignores what its player doesn't allow (§9.6, rule 4).
                        let _ = wind_core::media::command(session, action).await;
                    }
                }
                Some(Action::Ring) => {
                    if let (None, Some((session, _))) = (&ring, &active) {
                        snapshot.find.last = None;
                        snapshot.find.ring_ms = None;
                        match tokio::time::timeout(std::time::Duration::from_secs(3), wind_core::find::ring(session)).await {
                            Ok(Ok(r)) => { snapshot.find.state = "starting"; ring = Some((r, Instant::now())); }
                            Ok(Err(wind_core::session::SessionError::CapabilityMissing(_))) => { snapshot.find.last = Some(crate::i18n::tr("find.refused").into()); }
                            _ => { snapshot.find.last = Some(crate::i18n::tr("find.failed").into()); }
                        }
                    }
                }
                Some(Action::StopRing) => { if let Some((r, _)) = &ring { r.stop(); } }
                // Pairing another phone is allowed during a session (WC-28); the session stays.
                Some(Action::Refresh) => { if let Some(p) = prompt.take() { p.reject(); } snapshot.code = None; refresh = true; }
                Some(Action::Close) => {
                    if let Some(p) = prompt.take() { p.reject(); }
                    listener.close_window(); snapshot.qr = None; snapshot.code = None;
                    if active.is_some() { snapshot.phase = "connected".into(); snapshot.message = crate::i18n::tr("pairing.connected").into(); }
                    else { snapshot.phase = "offline".into(); snapshot.message = crate::i18n::tr("pairing.cancelled").into(); }
                }
                Some(Action::Sas) => {
                    if let Some(p) = prompt.take() { p.reject(); }
                    listener.open_sas();
                    snapshot.qr = None; snapshot.code = None;
                    snapshot.phase = "sas-waiting".into();
                    snapshot.message = crate::i18n::trf("pairing.sas_choose", &[("name", &name)]);
                }
                Some(Action::Decide(accept)) => {
                    if let Some(p) = prompt.take() {
                        if accept {
                            p.accept();
                            snapshot.phase = "sas-confirming".into();
                            snapshot.message = crate::i18n::tr("pairing.sas_waiting").into();
                        } else {
                            p.reject();
                            snapshot.code = None;
                            snapshot.phase = "sas-waiting".into();
                            snapshot.message = crate::i18n::tr("pairing.sas_rejected").into();
                        }
                    }
                }
                Some(Action::Rename(key, new_name)) => {
                    match store.rename(&key, &new_name) {
                        Ok(()) => { if active_key == Some(key) { snapshot.phone = clean_name(&new_name); } }
                        Err(_) => { snapshot.message = crate::i18n::tr("pairing.rename_failed").into(); }
                    }
                }
                // CCS/1 FLOW:unpair -> `-` + revoke now; Goodbye UNPAIRED in the session or on its next connection (§12.10).
                Some(Action::Unpair(key)) => {
                    let gone = store.load().ok().and_then(|l| l.get(&key).and_then(|d| d.name.clone())).unwrap_or_else(|| crate::i18n::tr("common.phone_capital").into());
                    let session = active.as_ref().filter(|(s, _)| s.peer().key == key).map(|(s, _)| s.clone());
                    match unpairing.unpair(key, session.as_ref()).await {
                        Ok(_) => {
                            // The phone's toasts and icons go with its pairing, and so does what was read of its SMS.
                            crate::sms::forget(&app, &key);
                            crate::gallery::forget(&app, &key);
                            crate::relay::rescan(&app); // its relay keeper ends now (WN-139)
                            let _ = toasts.send(crate::notify::Job::Clear);
                            if session.is_some() { crate::clipboard::detach(&app); crate::sms::detach(&app); crate::gallery::detach(&app); active = None; indicator.unavailable(); snapshot.phase = "offline".into(); }
                            snapshot.message = crate::i18n::trf("pairing.unpaired_from", &[("name", &gone)]);
                            let left = store.load().map(|l| l.devices).unwrap_or_default();
                            match left.last() {
                                Some(d) => crate::files::set_target(&app, *d.key.fingerprint().as_bytes(), d.name.clone()),
                                None => { snapshot.phone = None; if active.is_none() { refresh = true; } }
                            }
                            if active.is_none() { snapshot.phone = left.last().and_then(|d| d.name.clone()); }
                        }
                        Err(_) => { snapshot.message = crate::i18n::tr("pairing.unpair_failed").into(); }
                    }
                }
                Some(Action::Disconnect(key)) => {
                    if let Some((session, _)) = active.take_if(|(s, _)| s.peer().key == key) {
                        dismissed = Some((key, Instant::now()));
                        // GOING_AWAY: the phone doesn't come back on its own (§7.7), so another one can connect.
                        session.goodbye(ErrorCode::GoingAway, None).await;
                        crate::clipboard::detach(&app); crate::sms::detach(&app); crate::gallery::detach(&app); indicator.unavailable();
                        snapshot.phase = "offline".into(); snapshot.message = crate::i18n::tr("pairing.disconnected").into();
                    }
                }
                None => break,
            },
            Some(line) = notices.recv() => {
                snapshot.message = line;
            },
            changed = network.changed() => {
                if changed.is_err() { break; }
                if snapshot.phase == "waiting" || snapshot.phase == "no-network" { refresh = true; }
            },
            event = listener.next_event() => match event {
                Some(PairingEvent::Sas(p)) => {
                    if let Some(old) = prompt.replace(p) { old.reject(); }
                    let shown = prompt.as_ref().expect("just set");
                    snapshot.code = Some(shown.code_text());
                    snapshot.phone = shown.peer_name().map(String::from);
                    snapshot.phase = "sas-code".into();
                    snapshot.message = crate::i18n::trf("pairing.same_code", &[("name", &shown.peer_name().unwrap_or(crate::i18n::tr("common.phone")))]);
                }
                Some(PairingEvent::Paired(device)) => {
                    prompt = None; snapshot.code = None;
                    preferred = Some((device.key, Instant::now()));
                    paired_at = Some(Instant::now()); snapshot.qr = None;
                    if active.as_ref().is_none_or(|(session, _)| session.peer().key != device.key) { snapshot.phase = "connecting".into(); snapshot.phone = device.name; snapshot.message = crate::i18n::tr("pairing.paired_securing").into(); }
                }
                Some(PairingEvent::WindowClosed(WindowEnd::Expired | WindowEnd::TooManyAttempts)) if snapshot.phase == "waiting" || snapshot.phase.starts_with("sas-") => { prompt = None; snapshot.code = None; snapshot.phase = "expired".into(); snapshot.qr = None; snapshot.message = crate::i18n::tr("pairing.expired").into(); }
                Some(PairingEvent::AttemptFailed { failures, .. }) => {
                    prompt = None;
                    if snapshot.phase.starts_with("sas-") { snapshot.code = None; snapshot.phase = "sas-waiting".into(); }
                    snapshot.message = crate::i18n::trf("pairing.attempt_rejected", &[("failures", &failures)]);
                }
                _ => {},
            },
            incoming = sessions.recv() => {
                let Some((session, mut events, clipboard_input, action_streams, session_battery, session_media, files_input, session_commands, session_notify, session_call, session_volume, session_touchpad)) = incoming else { break };
                // CCS/1 RULE:one session; another phone gets BUSY unless just paired here (the person's choice) (WC-28).
                let fresh = preferred.is_some_and(|(k, at)| k == session.peer().key && at.elapsed() < PAIRED_TAKES_OVER);
                let sent_away = dismissed.is_some_and(|(k, at)| k == session.peer().key && at.elapsed() < DISMISSED_FOR);
                let busy = matches!(admit(active.as_ref().map(|(s, _)| s), &session), Admit::Busy);
                if (busy || sent_away) && !fresh {
                    // CCS/1 RULE:a refused session may only be delivering the phone's unpair notice: honour it (§12.10).
                    let (unpairing, notices) = (unpairing.clone(), loop_notices.clone());
                    tokio::spawn(async move {
                        if turn_away(&session, &mut events).await == TurnedAway::Unpaired
                            && unpairing.unpaired_by(&session).is_ok()
                        {
                            let _ = notices.send(crate::i18n::tr("pairing.phone_unpaired").into());
                        }
                    });
                    continue;
                }
                if dismissed.is_some_and(|(k, _)| k != session.peer().key) { dismissed = None; }
                let generation = indicator.searching();
                if let Some(control)=touchpad.take() {control.stop().await;}
                if let Some((previous, _)) = active.take() { previous.connection().close(ErrorCode::GoingAway, b"replaced"); }
                crate::files::attach(&app, session.clone(), files_input);
                crate::actions::serve(&app, trust.clone(), session.clone(), action_streams);
                crate::clipboard::attach(&app, session.clone(), clipboard_input);
                crate::sms::attach(&app, session.clone());
                crate::gallery::attach(&app, session.clone());
                snapshot.phone = shown_name(&store, &session);
                touchpad=Some(crate::touchpad::Control::start(session.clone(),session_touchpad,snapshot.phone.clone().unwrap_or_else(||crate::i18n::tr("common.phone_capital").into()),touchpad_allowed.load(std::sync::atomic::Ordering::SeqCst),caps_changed.clone()));
                // Pairing another phone stays on screen while a known one reconnects.
                let adding = (snapshot.phase == "waiting" || snapshot.phase.starts_with("sas-")) && !fresh;
                if !adding {
                    snapshot.phase = "connected".into(); snapshot.qr = None;
                    snapshot.message = crate::i18n::tr("pairing.connected").into();
                    listener.close_window();
                }
                if fresh { snapshot.pair_to_connected_ms = paired_at.map(|t| t.elapsed().as_millis() as u64); preferred = None; }
                indicator.connected(generation);
                battery_input = Some(session_battery);
                media_input = Some(session_media);
                media_commands = Some(session_commands);
                notify_input = Some(session_notify);
                let quiet = quiet_hours.clone();
                // WN-159: the phone's name in a toast only tells phones apart: with one paired phone the line is the app alone.
                let several = store.load().map(|l| l.devices.len() > 1).unwrap_or(false);
                let toast_phone = if several { snapshot.phone.as_deref().unwrap_or(crate::i18n::tr("common.phone_capital")) } else { "" };
                notifications = Some(crate::notify::Notifications::new(toasts.clone(), toast_phone)
                    .silenced_by(move || crate::notify::silenced_now(&quiet.lock().unwrap())));
                pc_media = None;
                if share_media.load(std::sync::atomic::Ordering::SeqCst) {
                    pc_media = crate::pc_media::PcMedia::start(session.clone(), tokio::runtime::Handle::current());
                }
                player = crate::media::Player::default();
                alerts = wind_core::battery::Alerts::default();
                snapshot.battery = None;
                // WN-48: the call state (idle comes first: a pause from a dropped session is settled), both volumes.
                call_input = Some(session_call);
                calls = Some(crate::calls::Calls::new(toasts.clone(), snapshot.phone.as_deref().unwrap_or(crate::i18n::tr("common.phone_capital"))));
                snapshot.phone_volume = None;
                tokio::spawn(crate::pc_volume::serve_phone(session_volume, pc_volume.clone(), phone_levels_tx.clone(), volume_remote.clone()));
                let (tick, ticks) = watch::channel(0u64);
                tokio::spawn(crate::pc_volume::send_levels(session.clone(), pc_level.subscribe(), ticks, volume_on.subscribe()));
                let (sets, sets_rx) = mpsc::channel(32);
                tokio::spawn(crate::pc_volume::send_sets(session.clone(), sets_rx));
                phone_sets = Some(sets);
                let changed = caps_changed.clone();
                tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        if matches!(event, wind_core::session::SessionEvent::Capabilities(_)) {
                            tick.send_modify(|n| *n += 1);
                            if changed.send(()).is_err() { break; }
                        }
                    }
                });
                crate::relay::track(&app, session.connection().clone());
                if fresh { crate::relay::rescan(&app); } // a newly paired phone gets its relay keeper now (WN-139)
                active = Some((session, ()));
            },
            // CCS/1 FLOW:Ring -> starting -> phone State 0 (sound playing) -> ringing -> final state or broken stream -> idle + last (WN-50).
            changed = async { match ring.as_mut() { Some((r, _)) => r.state.changed().await, None => std::future::pending().await } } => {
                let (r, asked) = ring.as_ref().expect("a ring's state changed");
                let now = *r.state.borrow();
                match now {
                    wind_core::find::Asked::Starting => {}
                    wind_core::find::Asked::Ringing => {
                        snapshot.find.state = "ringing";
                        snapshot.find.ring_ms = Some(asked.elapsed().as_millis() as u64);
                    }
                    wind_core::find::Asked::Ended(end) => {
                        snapshot.find.state = "idle";
                        snapshot.find.last = Some(find_outcome(end).into());
                        ring = None;
                    }
                }
                if changed.is_err() && ring.is_some() {
                    snapshot.find.state = "idle";
                    snapshot.find.last = Some(find_outcome(None).into());
                    ring = None;
                }
            },
            // The phone changed what it accepts (e.g. "Pozwól komputerowi dzwonić", WN-50): the loop recomputes.
            Some(()) = caps_changes.recv() => { if let Some(m) = &pc_media { m.resend(); } crate::sms::capabilities_changed(&app); crate::gallery::capabilities_changed(&app); },
            // CCS/1 FLOW:phone Command -> PcMedia (dropped outside the PC player's actions) -> SMTC (WN-47).
            message = async { match media_commands.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => { if let (Some(m), Some(action)) = (&pc_media, wind_core::media::Action::decode(&message.message)) { m.command(action); } }
                None => media_commands = None,
            },
            // CCS/1 FLOW:phone Now/Art/Gone -> Player (sanitised) -> snapshot.media, pushed (WN-46).
            message = async { match media_input.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => { if let Some(m) = wind_core::media::Message::decode(&message.message) { player.apply(m); } }
                None => media_input = None,
            },
            // CCS/1 FLOW:toast reply/close (WinRT, any thread) -> notify-action/1 to the phone (WN-44).
            Some(event) = toast_events.recv() => match event {
                crate::notify::ToastEvent::Reply { tag, text } => {
                    notify_reply(active.as_ref().map(|(s, _)| s), notifications.as_mut(), &tag, text).await;
                }
                crate::notify::ToastEvent::TooLong { tag, text } => {
                    if let Some(n) = notifications.as_mut() { n.too_long(&tag, text); }
                }
                crate::notify::ToastEvent::Dismissed { tag } => {
                    notify_dismiss(active.as_ref().map(|(s, _)| s), notifications.as_mut(), &tag).await;
                }
                // CCS/1 FLOW:WN-57 call toast "Odrzuć"/"Wycisz" -> call-action/1 (the phone acts only on the call it names).
                crate::notify::ToastEvent::Call(action) => {
                    if let Some((session, _)) = &active { let _ = wind_core::call::act(session, action).await; }
                }
            },
            // CCS/1 FLOW:phone call State -> the call-pause worker (WN-48).
            message = async { match call_input.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => match wind_core::call::Message::decode(&message.message) {
                    Some(wind_core::call::Message::State(call)) => {
                        if pause_on_call.load(std::sync::atomic::Ordering::SeqCst) { pauser.state(call.state); }
                        if let Some(c) = calls.as_mut() { c.state(&call, call_toasts.load(std::sync::atomic::Ordering::SeqCst)); }
                    }
                    Some(wind_core::call::Message::Missed(list)) => {
                        if let Some(c) = calls.as_mut() { c.missed(list, call_toasts.load(std::sync::atomic::Ordering::SeqCst)); }
                    }
                    None => {}
                },
                None => call_input = None,
            },
            Some(level) = pc_levels.recv() => { pc_level.send_replace(Some(level)); continue; },
            Some((level, muted)) = phone_levels.recv() => {
                if volume_remote.load(std::sync::atomic::Ordering::SeqCst) && active.is_some() {
                    snapshot.phone_volume = Some(crate::pc_volume::PhoneVolume { level, muted, controllable: false });
                }
            },
            // CCS/1 FLOW:phone Posted/Icon/Removed -> Notifications (popup or quiet) -> toast worker (WN-43).
            message = async { match notify_input.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => { if let (Some(n), Some(m)) = (notifications.as_mut(), wind_core::notify::Message::decode(&message.message)) { n.apply(m); } }
                None => notify_input = None,
            },
            // CCS/1 FLOW:phone Battery -> snapshot (panel, pushed) + tray badge/tooltip + alert toast (WN-49).
            message = async { match battery_input.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => {
                    if let Some(status) = wind_core::battery::Status::decode(&message.message) {
                        snapshot.battery = Some(status.into());
                        if let Some(alert) = alerts.update(status) {
                            crate::battery::notify(&app, alert, snapshot.phone.as_deref().unwrap_or(crate::i18n::tr("common.phone_capital")));
                        }
                    }
                }
                None => battery_input = None,
            },
            end = async { match &active { Some((session, _)) => session.closed().await, None => std::future::pending().await } } => {
                let (session, _) = active.take().expect("closed future implies a session");
                crate::clipboard::detach(&app); crate::sms::detach(&app); crate::gallery::detach(&app); indicator.unavailable(); snapshot.phase = "offline".into(); snapshot.message = crate::i18n::tr("pairing.disconnected").into();
                // CCS/1 SEC:the phone removed the pairing: forget the key of this session's TLS peer only.
                if close_code(&end) == Some(ErrorCode::Unpaired) && unpairing.unpaired_by(&session).is_ok() {
                    crate::sms::forget(&app, &session.peer().key);
                    crate::gallery::forget(&app, &session.peer().key);
                    let _ = toasts.send(crate::notify::Job::Clear);
                    snapshot.message = crate::i18n::trf("pairing.phone_unpaired_named", &[("name", &snapshot.phone.clone().unwrap_or_else(|| crate::i18n::tr("common.phone_capital").into()))]);
                    let left = store.load().map(|l| l.devices).unwrap_or_default();
                    snapshot.phone = left.last().and_then(|d| d.name.clone());
                    if let Some(d) = left.last() { crate::files::set_target(&app, *d.key.fingerprint().as_bytes(), d.name.clone()); }
                    if left.is_empty() { refresh = true; }
                }
            },
        }
    }
    listener.close_window();
    if let Some(control) = touchpad.take() {
        control.stop().await;
    }
    // The last published QR would stay in the watch until the app exits.
    state.send_modify(|s| s.qr = None);
    discovery.shutdown().await;
    endpoint.close();
    endpoint.wait_idle().await;
    Ok(())
}

#[tauri::command]
pub fn pairing_snapshot(state: tauri::State<'_, Runtime>) -> Snapshot {
    state.state.borrow().clone()
}
#[tauri::command]
pub fn pairing_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    action: &str,
) -> Result<(), String> {
    if !matches!(window.label(), "panel" | "main") {
        return Err("unsupported window".into());
    }
    let action = match action {
        "refresh" => Action::Refresh,
        "close" => Action::Close,
        "sas" => Action::Sas,
        "sas-accept" => Action::Decide(true),
        "sas-reject" => Action::Decide(false),
        "ring" => Action::Ring,
        "ring-stop" => Action::StopRing,
        "touchpad-on" => Action::Touchpad(true),
        "touchpad-off" => Action::Touchpad(false),
        _ => return Err("unsupported pairing action".into()),
    };
    state
        .commands
        .send(action)
        .map_err(|_| "pairing unavailable".into())
}
/// The device list's actions (WC-28): "rename" (with `name`), "unpair",
/// "disconnect"; `key` is a row's `key`.
#[tauri::command]
pub fn device_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    action: &str,
    key: &str,
    name: Option<String>,
) -> Result<(), String> {
    if !matches!(window.label(), "panel" | "main") {
        return Err("unsupported window".into());
    }
    let key = parse_key(key).ok_or("bad device key")?;
    let action = match (action, name) {
        ("rename", Some(name)) => Action::Rename(key, name),
        ("unpair", _) => Action::Unpair(key),
        ("disconnect", _) => Action::Disconnect(key),
        _ => return Err("unsupported device action".into()),
    };
    state
        .commands
        .send(action)
        .map_err(|_| "pairing unavailable".into())
}
/// WN-47: "Udostępnij odtwarzanie telefonowi" on or off (main window settings).
#[tauri::command]
pub fn share_media(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    enabled: bool,
) -> Result<(), String> {
    if !matches!(window.label(), "main" | "panel") {
        return Err("unsupported window".into());
    }
    state
        .commands
        .send(Action::ShareMedia(enabled))
        .map_err(|_| "pairing unavailable".into())
}
/// WN-44: a reply for one of the phone's notifications shown in this session; the text is never logged.
async fn notify_reply(
    session: Option<&Session>,
    notifications: Option<&mut crate::notify::Notifications>,
    tag: &str,
    text: String,
) {
    let (Some(session), Some(n)) = (session, notifications) else {
        return;
    };
    let Some(id) = n.known(tag) else { return };
    let sent = wind_core::notify::act(session, &wind_core::notify::Action::Reply { id, text })
        .await
        .unwrap_or(false);
    n.reply_sent(tag, sent);
}

/// WN-44: dismissed on the PC (toast ✕ or the panel): gone here, and on the phone when it takes `notify-action/1`.
async fn notify_dismiss(
    session: Option<&Session>,
    notifications: Option<&mut crate::notify::Notifications>,
    tag: &str,
) {
    let Some(n) = notifications else { return };
    let id = n.known(tag);
    n.dismiss(tag);
    if let (Some(session), Some(id)) = (session, id) {
        let _ = wind_core::notify::act(session, &wind_core::notify::Action::Dismiss { id }).await;
    }
}

/// WN-44: from the panel: "reply" (with `text`, 1–1024 bytes; never trimmed) or "dismiss", by the notification's tag.
#[tauri::command]
pub fn notify_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    action: &str,
    tag: String,
    text: Option<String>,
) -> Result<(), String> {
    if window.label() != "panel" {
        return Err("unsupported window".into());
    }
    if crate::notify::id_of(&tag).is_none() {
        return Err("unknown notification".into());
    }
    let action = match (action, text) {
        ("reply", Some(text)) => {
            let text = text.trim().to_owned();
            if text.is_empty() {
                return Err("empty reply".into());
            }
            if text.len() > wind_core::notify::MAX_REPLY {
                return Err("too-long".into());
            }
            Action::NotifyReply(tag, text)
        }
        ("dismiss", _) => Action::NotifyDismiss(tag),
        _ => return Err("unknown action".into()),
    };
    state
        .commands
        .send(action)
        .map_err(|_| "pairing unavailable".into())
}
/// WN-43: "Powiadomienia z telefonu" on or off (main window settings).
#[tauri::command]
pub fn phone_notifications(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    enabled: bool,
) -> Result<(), String> {
    if !matches!(window.label(), "main" | "panel") {
        return Err("unsupported window".into());
    }
    state
        .commands
        .send(Action::PhoneNotifications(enabled))
        .map_err(|_| "pairing unavailable".into())
}
/// WN-48: "Wstrzymaj odtwarzanie przy połączeniu" / "Głośność z telefonu" (main window); only what is given.
#[tauri::command]
pub fn call_volume_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    pause_on_call: Option<bool>,
    volume: Option<bool>,
    calls: Option<bool>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("unsupported window".into());
    }
    state
        .commands
        .send(Action::CallVolume {
            pause: pause_on_call,
            volume,
            calls,
        })
        .map_err(|_| "pairing unavailable".into())
}
/// WN-48: the panel's slider and mute for the phone's volume (0..=100; never clamped silently).
#[tauri::command]
pub fn phone_volume(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    level: Option<u8>,
    muted: Option<bool>,
) -> Result<(), String> {
    if window.label() != "panel" {
        return Err("unsupported window".into());
    }
    if level.is_some_and(|l| l > 100) || (level.is_none() && muted.is_none()) {
        return Err("invalid volume".into());
    }
    state
        .commands
        .send(Action::PhoneVolume(level, muted))
        .map_err(|_| "pairing unavailable".into())
}
/// WN-45: "Ciche godziny" (main window): on/off and from/to in minutes after local midnight, 15-minute steps.
#[tauri::command]
pub fn notify_quiet_hours(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    on: bool,
    from: u16,
    to: u16,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("unsupported window".into());
    }
    let quiet = crate::notify::QuietHours { on, from, to };
    if !quiet.valid() {
        return Err("invalid quiet hours".into());
    }
    state
        .commands
        .send(Action::QuietHours(quiet))
        .map_err(|_| "pairing unavailable".into())
}
/// WN-46: a button of the phone's player ("play", "pause", "toggle", "next", "previous",
/// "seek" with `position_ms`).
#[tauri::command]
pub fn media_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Runtime>,
    action: &str,
    position_ms: Option<u64>,
) -> Result<(), String> {
    if window.label() != "panel" {
        return Err("unsupported window".into());
    }
    let action = crate::media::action(action, position_ms).ok_or("unsupported media action")?;
    state
        .commands
        .send(Action::Media(action))
        .map_err(|_| "pairing unavailable".into())
}
#[tauri::command]
pub fn dev_pairing_address(state: tauri::State<'_, Runtime>) -> Result<String, String> {
    #[cfg(debug_assertions)]
    return state
        .main_address
        .lock()
        .unwrap()
        .map(|a| format!("127.0.0.1:{}", a.port()))
        .ok_or_else(|| "not ready".into());
    #[cfg(not(debug_assertions))]
    {
        let _ = state;
        Err("debug only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> qrcode::QrCode {
        qrcode::QrCode::with_error_correction_level(
            b"windconnect://pair?v=1&s=AAAA",
            qrcode::EcLevel::M,
        )
        .unwrap()
    }

    #[test]
    fn qr_bits_keep_every_module() {
        let reference = code();
        let width = reference.width();
        let bits = qr_bits(code());
        assert_eq!(bits.width, width);
        assert_eq!(bits.dark.len(), width * width);
        for y in 0..width {
            for x in 0..width {
                assert_eq!(
                    bits.dark[y * width + x],
                    reference[(x, y)] == qrcode::Color::Dark
                );
            }
        }
    }

    #[test]
    fn wipe_clears_the_modules() {
        let mut bits = qr_bits(code());
        assert!(bits.dark.iter().any(|d| *d));
        let len = bits.dark.len();
        bits.wipe();
        assert_eq!(bits.width, 0);
        assert!(bits.dark.is_empty());
        // Vec::zeroize also overwrote the spare capacity; the buffer is kept.
        assert!(bits.dark.capacity() >= len);
    }

    #[test]
    fn wipe_colors_overwrites_every_module() {
        let mut colors = code().into_colors();
        assert!(colors.contains(&qrcode::Color::Dark));
        wipe_colors(&mut colors);
        assert!(colors.iter().all(|c| *c == qrcode::Color::Light));
    }

    #[test]
    fn a_phone_owed_a_notice_is_not_paired() {
        // WN-113: after the last phone is removed (its notice not delivered
        // yet), the first window and the onboarding view both see no phone.
        let dir = std::env::temp_dir().join(format!("wc-wn113-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = PairedDevicesFile::in_dir(&dir);
        assert!(!has_paired_in(&store), "no file: nothing paired");
        use wind_core::pairing::{PairedDevice, PairedStore};
        let key = DeviceIdentity::generate().unwrap().public_key();
        store
            .add(&PairedDevice::now(key, Some("Pixel".into())))
            .unwrap();
        assert!(has_paired_in(&store));
        store.remove(&key).unwrap();
        let rows = device_rows(&store, None);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].owed, "the removed phone is listed as owed");
        assert!(!has_paired_in(&store), "an owed phone isn't paired");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_outcomes_say_what_happened() {
        use wind_core::find::State;
        assert_eq!(
            find_outcome(Some(State::StoppedHere)),
            "Zatrzymano na telefonie"
        );
        assert_eq!(find_outcome(Some(State::TimedOut)), "Telefon dzwonił 60 s");
        // A broken stream isn't "stopped": the phone may still ring (WN-50 review, point 5).
        assert!(find_outcome(None).contains("może nadal dzwonić"));
        assert_eq!(FindView::default().state, "idle");
    }

    #[test]
    fn closing_the_window_clears_the_published_qr() {
        // The watch keeps one value; replacing it drops (and wipes) the old.
        let (state, _) = watch::channel(Snapshot {
            phase: "waiting".into(),
            qr: Some(qr_bits(code())),
            ..Default::default()
        });
        let old = state.send_replace(Snapshot {
            phase: "offline".into(),
            ..Default::default()
        });
        assert!(old.qr.is_some());
        assert!(state.borrow().qr.is_none());
        state.send_modify(|s| s.qr = Some(qr_bits(code())));
        state.send_modify(|s| s.qr = None);
        assert!(state.borrow().qr.is_none());
    }
}
