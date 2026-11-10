use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, watch};
use wind_core::{
    identity::{DeviceIdentity, DpapiKeyStore},
    network::{NetworkChanges, NetworkDiscovery},
    pairing::{
        clean_name, PairedDevicesFile, PairingConfig, PairingEvent, PairingListener, WindowEnd,
    },
    protocol::{DeviceKind, ErrorCode},
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};

#[derive(Clone, PartialEq, serde::Serialize)]
pub struct QrBits {
    pub width: usize,
    pub dark: Vec<bool>,
}
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct Snapshot {
    pub phase: String,
    pub phone: Option<String>,
    pub qr: Option<QrBits>,
    pub message: String,
    pub skipped_records: usize,
    pub pair_to_connected_ms: Option<u64>,
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
        }
    }
}
enum Action {
    Refresh,
    Close,
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
    let _ = app.emit_to("panel", "pairing-state", value);
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
    let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M)?;
    Ok(QrBits {
        width: code.width(),
        dark: code
            .into_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect(),
    })
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
    let endpoint = Endpoint::bind("0.0.0.0:0".parse()?, &identity, trust.clone())?;
    *app.state::<Runtime>().main_address.lock().unwrap() = Some(endpoint.local_addr()?);
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
    let (session_sender, mut sessions) = mpsc::channel(1);
    let accepting = endpoint.clone();
    let _accept_task = AcceptTask(tokio::spawn(async move {
        while let Some(incoming) = accepting.accept().await {
            if let Ok(connection) = incoming {
                if let Ok(Ok(session)) = tokio::time::timeout(
                    std::time::Duration::from_secs(6),
                    Session::accept(connection, local.clone(), Router::new()),
                )
                .await
                {
                    if session_sender.send(session).await.is_err() {
                        break;
                    }
                }
            }
        }
    }));
    let mut listener = PairingListener::bind(
        "0.0.0.0:0".parse()?,
        &identity,
        Some(&name),
        store,
        trust,
        PairingConfig::default(),
    )?;
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
    let mut active: Option<(Session, _)> = None;
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
        publish(&app, &state, snapshot.clone());
        tokio::select! {
            _ = async { if let Some(start) = paired_at.filter(|_| snapshot.phase == "connecting") { tokio::time::sleep_until(tokio::time::Instant::from_std(start + std::time::Duration::from_secs(10))).await; } else { std::future::pending().await } } => {
                snapshot.phase = "offline".into(); snapshot.message = "Telefon sparowany, ale brak połączenia. Sprawdź wspólną sieć Wi-Fi.".into();
            },
            command = commands.recv() => match command {
                Some(Action::Refresh) => { if active.is_none() { refresh = true; } }
                Some(Action::Close) => { if active.is_none() { listener.close_window(); snapshot.qr = None; snapshot.phase = "offline".into(); snapshot.message = "Parowanie anulowane".into(); } }
                None => break,
            },
            changed = network.changed() => {
                if changed.is_err() { break; }
                if snapshot.phase == "waiting" || snapshot.phase == "no-network" { refresh = true; }
            },
            event = listener.next_event() => match event {
                Some(PairingEvent::Paired(device)) => {
                    paired_at = Some(Instant::now()); snapshot.qr = None;
                    if active.as_ref().is_none_or(|(session, _)| session.peer().key != device.key) { snapshot.phase = "connecting".into(); snapshot.phone = device.name; snapshot.message = "Sparowano · czekam na bezpieczne połączenie telefonu".into(); }
                }
                Some(PairingEvent::WindowClosed(WindowEnd::Expired | WindowEnd::TooManyAttempts)) if snapshot.phase == "waiting" => { snapshot.phase = "expired".into(); snapshot.qr = None; snapshot.message = "Kod wygasł lub osiągnięto limit prób. Wyświetl nowy kod.".into(); }
                Some(PairingEvent::AttemptFailed { failures, .. }) => { snapshot.message = format!("Próba parowania odrzucona ({failures}/3). Sprawdź kod i sieć."); }
                _ => {},
            },
            incoming = sessions.recv() => {
                if let Some((session, events)) = incoming {
                    let generation = indicator.searching();
                            if let Some((previous, _)) = active.take() { previous.connection().close(ErrorCode::GoingAway, b"replaced"); }
                            snapshot.phone = clean_name(&session.peer().hello.name);
                            snapshot.phase = "connected".into(); snapshot.qr = None;
                            snapshot.message = "Połączono · Wi-Fi".into();
                            snapshot.pair_to_connected_ms = paired_at.map(|t| t.elapsed().as_millis() as u64);
                            indicator.connected(generation); listener.close_window();
                            active = Some((session, events));
                } else { break; }
            },
            _ = async { match &active { Some((session, _)) => { session.closed().await; }, None => std::future::pending().await } } => {
                active = None; indicator.unavailable(); snapshot.phase = "offline".into(); snapshot.message = "Telefon rozłączony".into();
            },
        }
    }
    listener.close_window();
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
        _ => return Err("unsupported pairing action".into()),
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
