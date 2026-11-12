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
    /// A phone is paired: [`has_paired_in`], the rule the first window
    /// follows too (WN-113). A phone owed an unpair notice doesn't count.
    pub paired: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            phase: "loading".into(),
            phone: None,
            qr: None,
            message: "Przygotowywanie bezpiecznego połączenia…".into(),
            skipped_records: 0,
            pair_to_connected_ms: None,
            code: None,
            connected: false,
            devices: Vec::new(),
            battery: None,
            paired: false,
        }
    }
}
enum Action {
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
        return Err("Nie znaleziono adresu sieci Wi-Fi lub Ethernet. Połącz komputer z siecią i spróbuj ponownie.".into());
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
            publish(&handle, &state, Snapshot { phase: "failed".into(), message: "Nie udało się uruchomić parowania. Sprawdź tożsamość i dostęp do sieci, a następnie uruchom aplikację ponownie.".into(), ..Default::default() });
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
            name: d.name.clone().unwrap_or_else(|| "Telefon".into()),
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
                name: "Usunięty telefon".into(),
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
    let identity = DeviceIdentity::load_or_create(&key_store)?;
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
    let accept_unpairing = unpairing.clone();
    let _accept_task = AcceptTask(tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            if let Ok(connection) = incoming {
                // CCS/1 SEC:actions/1 accepted from pinned phones only; each request re-checks the pin (WN-54).
                let mut router = Router::new();
                let clipboard_input = wind_core::clipboard::Clipboard::route(&mut router);
                let action_streams = router.route_actions(8);
                // CCS/1 IO:battery/1 from the phone (§9.4, WN-49): status only, never acted on beyond display and alerts.
                let battery_input = wind_core::battery::route(&mut router);
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
                            let _ = notice_sender
                                .send("Usunięty telefon dostał informację o rozparowaniu.".into());
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
        message: "Telefon sparowany · czekam na połączenie".into(),
        ..Default::default()
    };
    let mut network = changes.subscribe();
    let mut paired_at = None::<Instant>;
    // The phone paired here last, and when: it may take over the session.
    let mut preferred: Option<(PublicKey, Instant)> = None;
    // The phone disconnected from the panel, and when (BUSY for DISMISSED_FOR).
    let mut dismissed: Option<(PublicKey, Instant)> = None;
    let mut active: Option<(Session, _)> = None;
    // The active session's battery messages and its alerts (WN-49); both end with the session.
    let mut battery_input: Option<mpsc::Receiver<wind_core::session::ChannelMessage>> = None;
    let mut alerts = wind_core::battery::Alerts::default();
    let mut refresh = loaded.devices.is_empty();
    loop {
        if refresh {
            refresh = false;
            match open_window(&listener) {
                Ok(qr) => {
                    snapshot.phase = "waiting".into();
                    snapshot.qr = Some(qr);
                    snapshot.message = "Zeskanuj kod w WindConnect na telefonie".into();
                }
                Err(_) => {
                    snapshot.phase = "no-network".into();
                    snapshot.qr = None;
                    snapshot.message =
                        "Połącz komputer z Wi-Fi lub Ethernet i wyświetl nowy kod.".into();
                }
            }
        }
        let active_key = active.as_ref().map(|(s, _): &(Session, _)| s.peer().key);
        snapshot.connected = active_key.is_some();
        if active.is_none() {
            battery_input = None;
            snapshot.battery = None;
        }
        crate::battery::set(&app, snapshot.battery);
        snapshot.devices = device_rows(&store, active_key.as_ref());
        snapshot.paired = has_paired_in(&store);
        publish(&app, &state, snapshot.clone());
        tokio::select! {
            _ = async { if let Some(start) = paired_at.filter(|_| snapshot.phase == "connecting") { tokio::time::sleep_until(tokio::time::Instant::from_std(start + std::time::Duration::from_secs(10))).await; } else { std::future::pending().await } } => {
                snapshot.phase = "offline".into(); snapshot.message = "Telefon sparowany, ale brak połączenia. Sprawdź wspólną sieć Wi-Fi.".into();
            },
            command = commands.recv() => match command {
                // Pairing another phone is allowed during a session (WC-28); the session stays.
                Some(Action::Refresh) => { if let Some(p) = prompt.take() { p.reject(); } snapshot.code = None; refresh = true; }
                Some(Action::Close) => {
                    if let Some(p) = prompt.take() { p.reject(); }
                    listener.close_window(); snapshot.qr = None; snapshot.code = None;
                    if active.is_some() { snapshot.phase = "connected".into(); snapshot.message = "Połączono · Wi-Fi".into(); }
                    else { snapshot.phase = "offline".into(); snapshot.message = "Parowanie anulowane".into(); }
                }
                Some(Action::Sas) => {
                    if let Some(p) = prompt.take() { p.reject(); }
                    listener.open_sas();
                    snapshot.qr = None; snapshot.code = None;
                    snapshot.phase = "sas-waiting".into();
                    snapshot.message = format!("Na telefonie wybierz „Paruj bez aparatu”, a potem {name} z listy.");
                }
                Some(Action::Decide(accept)) => {
                    if let Some(p) = prompt.take() {
                        if accept {
                            p.accept();
                            snapshot.phase = "sas-confirming".into();
                            snapshot.message = "Czekam na potwierdzenie na telefonie…".into();
                        } else {
                            p.reject();
                            snapshot.code = None;
                            snapshot.phase = "sas-waiting".into();
                            snapshot.message = "Kod odrzucony. Spróbuj ponownie z telefonu albo wróć do kodu QR.".into();
                        }
                    }
                }
                Some(Action::Rename(key, new_name)) => {
                    match store.rename(&key, &new_name) {
                        Ok(()) => { if active_key == Some(key) { snapshot.phone = clean_name(&new_name); } }
                        Err(_) => { snapshot.message = "Nie udało się zmienić nazwy. Nazwa nie może być pusta.".into(); }
                    }
                }
                // CCS/1 FLOW:unpair -> `-` + revoke now; Goodbye UNPAIRED in the session or on its next connection (§12.10).
                Some(Action::Unpair(key)) => {
                    let gone = store.load().ok().and_then(|l| l.get(&key).and_then(|d| d.name.clone())).unwrap_or_else(|| "Telefon".into());
                    let session = active.as_ref().filter(|(s, _)| s.peer().key == key).map(|(s, _)| s.clone());
                    match unpairing.unpair(key, session.as_ref()).await {
                        Ok(_) => {
                            if session.is_some() { crate::clipboard::detach(&app); active = None; indicator.unavailable(); snapshot.phase = "offline".into(); }
                            snapshot.message = format!("Usunięto parowanie z: {gone}.");
                            let left = store.load().map(|l| l.devices).unwrap_or_default();
                            match left.last() {
                                Some(d) => crate::files::set_target(&app, *d.key.fingerprint().as_bytes(), d.name.clone()),
                                None => { snapshot.phone = None; if active.is_none() { refresh = true; } }
                            }
                            if active.is_none() { snapshot.phone = left.last().and_then(|d| d.name.clone()); }
                        }
                        Err(_) => { snapshot.message = "Nie udało się usunąć parowania. Spróbuj ponownie.".into(); }
                    }
                }
                Some(Action::Disconnect(key)) => {
                    if let Some((session, _)) = active.take_if(|(s, _)| s.peer().key == key) {
                        dismissed = Some((key, Instant::now()));
                        // GOING_AWAY: the phone doesn't come back on its own (§7.7), so another one can connect.
                        session.goodbye(ErrorCode::GoingAway, None).await;
                        crate::clipboard::detach(&app); indicator.unavailable();
                        snapshot.phase = "offline".into(); snapshot.message = "Telefon rozłączony".into();
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
                    snapshot.message = format!("Czy {} pokazuje ten sam kod?", shown.peer_name().unwrap_or("telefon"));
                }
                Some(PairingEvent::Paired(device)) => {
                    prompt = None; snapshot.code = None;
                    preferred = Some((device.key, Instant::now()));
                    paired_at = Some(Instant::now()); snapshot.qr = None;
                    if active.as_ref().is_none_or(|(session, _)| session.peer().key != device.key) { snapshot.phase = "connecting".into(); snapshot.phone = device.name; snapshot.message = "Sparowano · czekam na bezpieczne połączenie telefonu".into(); }
                }
                Some(PairingEvent::WindowClosed(WindowEnd::Expired | WindowEnd::TooManyAttempts)) if snapshot.phase == "waiting" || snapshot.phase.starts_with("sas-") => { prompt = None; snapshot.code = None; snapshot.phase = "expired".into(); snapshot.qr = None; snapshot.message = "Parowanie wygasło lub osiągnięto limit prób. Zacznij od nowa.".into(); }
                Some(PairingEvent::AttemptFailed { failures, .. }) => {
                    prompt = None;
                    if snapshot.phase.starts_with("sas-") { snapshot.code = None; snapshot.phase = "sas-waiting".into(); }
                    snapshot.message = format!("Próba parowania odrzucona ({failures}/3). Sprawdź kod i sieć.");
                }
                _ => {},
            },
            incoming = sessions.recv() => {
                let Some((session, mut events, clipboard_input, action_streams, session_battery)) = incoming else { break };
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
                            let _ = notices.send("Telefon usunął parowanie z tym komputerem.".into());
                        }
                    });
                    continue;
                }
                if dismissed.is_some_and(|(k, _)| k != session.peer().key) { dismissed = None; }
                let generation = indicator.searching();
                if let Some((previous, _)) = active.take() { previous.connection().close(ErrorCode::GoingAway, b"replaced"); }
                crate::files::attach(&app, session.clone());
                crate::actions::serve(&app, trust.clone(), session.clone(), action_streams);
                crate::clipboard::attach(&app, session.clone(), clipboard_input);
                snapshot.phone = shown_name(&store, &session);
                // Pairing another phone stays on screen while a known one reconnects.
                let adding = (snapshot.phase == "waiting" || snapshot.phase.starts_with("sas-")) && !fresh;
                if !adding {
                    snapshot.phase = "connected".into(); snapshot.qr = None;
                    snapshot.message = "Połączono · Wi-Fi".into();
                    listener.close_window();
                }
                if fresh { snapshot.pair_to_connected_ms = paired_at.map(|t| t.elapsed().as_millis() as u64); preferred = None; }
                indicator.connected(generation);
                battery_input = Some(session_battery);
                alerts = wind_core::battery::Alerts::default();
                snapshot.battery = None;
                active = Some((session, events));
            },
            // CCS/1 FLOW:phone Battery -> snapshot (panel, pushed) + tray badge/tooltip + alert toast (WN-49).
            message = async { match battery_input.as_mut() { Some(rx) => rx.recv().await, None => std::future::pending().await } } => match message {
                Some(message) => {
                    if let Some(status) = wind_core::battery::Status::decode(&message.message) {
                        snapshot.battery = Some(status.into());
                        if let Some(alert) = alerts.update(status) {
                            crate::battery::notify(&app, alert, snapshot.phone.as_deref().unwrap_or("Telefon"));
                        }
                    }
                }
                None => battery_input = None,
            },
            end = async { match &active { Some((session, _)) => session.closed().await, None => std::future::pending().await } } => {
                let (session, _) = active.take().expect("closed future implies a session");
                crate::clipboard::detach(&app); indicator.unavailable(); snapshot.phase = "offline".into(); snapshot.message = "Telefon rozłączony".into();
                // CCS/1 SEC:the phone removed the pairing: forget the key of this session's TLS peer only.
                if close_code(&end) == Some(ErrorCode::Unpaired) && unpairing.unpaired_by(&session).is_ok() {
                    snapshot.message = format!("{} usunął parowanie z tym komputerem.", snapshot.phone.clone().unwrap_or_else(|| "Telefon".into()));
                    let left = store.load().map(|l| l.devices).unwrap_or_default();
                    snapshot.phone = left.last().and_then(|d| d.name.clone());
                    if let Some(d) = left.last() { crate::files::set_target(&app, *d.key.fingerprint().as_bytes(), d.name.clone()); }
                    if left.is_empty() { refresh = true; }
                }
            },
        }
    }
    listener.close_window();
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
