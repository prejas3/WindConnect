//! CCS/1 M:relay setting (WN-92) | IO:main window switch + URL -> relay-url file -> one RelayLink keeper per paired phone.
//! RULE:off by default (WN-60 D3); off aborts every keeper at once and closes a session running over one
//! of their stand-ins (WN-139), so the phone goes back to the LAN or ends instead of idling out.
//! Same keeper as `windctl serve --relay`: reconnect with backoff whenever the link ends.
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tauri::Manager;
use tokio::sync::watch;
use wind_core::identity::{DeviceIdentity, PublicKey};
use wind_core::pairing::PairedDevicesFile;
use wind_core::relay::{check_url, RelayLink};
use wind_core::transport::Endpoint;

/// How often the paired list is read again, so a phone paired later gets its keeper.
const RESCAN: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Relay {
    url: Mutex<Option<String>>,
    path: Mutex<Option<PathBuf>>,
    wake: Mutex<Option<watch::Sender<Option<String>>>>,
    /// The live RelayLinks' local stand-in addresses: a session from one of them runs over the relay.
    stand_ins: Arc<Mutex<HashSet<SocketAddr>>>,
    /// Sessions the pairing loop adopted (closed ones are pruned on the next track).
    sessions: Mutex<Vec<wind_core::transport::Connection>>,
}

/// Same socket, whether the endpoint reports it as IPv4 or IPv4-mapped IPv6.
pub(crate) fn same_addr(a: SocketAddr, b: SocketAddr) -> bool {
    a.port() == b.port() && a.ip().to_canonical() == b.ip().to_canonical()
}

/// The pairing loop adopted `connection`; it is closed if the relay is switched off while it runs over it.
pub fn track(app: &tauri::AppHandle, connection: wind_core::transport::Connection) {
    let relay = app.state::<Relay>();
    let mut sessions = relay.sessions.lock().unwrap();
    sessions.retain(|c| c.quinn().close_reason().is_none());
    sessions.push(connection);
}

/// A phone was paired or unpaired: reconcile the keepers now, not at the next RESCAN.
pub fn rescan(app: &tauri::AppHandle) {
    if let Some(wake) = app.state::<Relay>().wake.lock().unwrap().as_ref() {
        wake.send_modify(|_| {});
    }
}

/// Off: closes every tracked session whose peer is a relay stand-in; returns how many.
fn close_relay_sessions(relay: &Relay) -> usize {
    let stand_ins: Vec<SocketAddr> = relay.stand_ins.lock().unwrap().drain().collect();
    let mut closed = 0;
    for c in relay.sessions.lock().unwrap().iter() {
        let remote = c.quinn().remote_address();
        if c.quinn().close_reason().is_none() && stand_ins.iter().any(|s| same_addr(*s, remote)) {
            c.close(wind_core::protocol::ErrorCode::GoingAway, b"relay-off");
            closed += 1;
        }
    }
    closed
}

/// The saved URL, or None (off). A file the core now refuses counts as off.
fn read(path: &PathBuf) -> Option<String> {
    let url = std::fs::read_to_string(path).ok()?.trim().to_string();
    check_url(&url).ok().map(|_| url)
}

/// Starts the keepers for the saved setting; called once the endpoint exists.
pub fn start(
    app: &tauri::AppHandle,
    dir: &std::path::Path,
    identity: Arc<DeviceIdentity>,
    endpoint: Endpoint,
    store: Arc<PairedDevicesFile>,
) {
    let relay = app.state::<Relay>();
    let path = dir.join("relay-url");
    let saved = read(&path);
    *relay.url.lock().unwrap() = saved.clone();
    *relay.path.lock().unwrap() = Some(path);
    let (wake, mut setting) = watch::channel(saved);
    *relay.wake.lock().unwrap() = Some(wake);
    let stand_ins = relay.stand_ins.clone();
    tauri::async_runtime::spawn(async move {
        let mut keepers: HashMap<[u8; 32], (String, JoinHandle<()>)> = HashMap::new();
        loop {
            let url = setting.borrow_and_update().clone();
            let peers: Vec<PublicKey> = match &url {
                Some(_) => store
                    .load()
                    .map(|l| l.devices.into_iter().map(|d| d.key).collect())
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            reconcile(&mut keepers, url.as_deref(), &peers, |url, peer| {
                tauri::async_runtime::spawn(keep(
                    url.to_string(),
                    identity.clone(),
                    peer,
                    endpoint.clone(),
                    stand_ins.clone(),
                ))
            });
            tokio::select! {
                changed = setting.changed() => if changed.is_err() { break },
                _ = tokio::time::sleep(RESCAN) => {}
            }
        }
        for (_, (_, task)) in keepers.drain() {
            task.abort();
        }
    });
}

/// Keeps exactly one keeper per paired peer for `url`; everything else is aborted.
pub(crate) fn reconcile<H: Abort>(
    keepers: &mut HashMap<[u8; 32], (String, H)>,
    url: Option<&str>,
    peers: &[PublicKey],
    mut spawn: impl FnMut(&str, PublicKey) -> H,
) {
    keepers.retain(|key, (kept, task)| {
        let keep = url == Some(kept.as_str()) && peers.iter().any(|p| p.as_bytes() == key);
        if !keep {
            task.abort();
        }
        keep
    });
    if let Some(url) = url {
        for peer in peers {
            keepers
                .entry(*peer.as_bytes())
                .or_insert_with(|| (url.to_string(), spawn(url, *peer)));
        }
    }
}

pub(crate) trait Abort {
    fn abort(&self);
}
impl Abort for JoinHandle<()> {
    fn abort(&self) {
        JoinHandle::abort(self)
    }
}

async fn keep(
    url: String,
    identity: Arc<DeviceIdentity>,
    peer: PublicKey,
    endpoint: Endpoint,
    stand_ins: Arc<Mutex<HashSet<SocketAddr>>>,
) {
    let mut backoff =
        wind_core::transport::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    loop {
        match RelayLink::connect(&url, &identity, peer, endpoint.clone()).await {
            Ok(link) => {
                backoff.reset();
                let addr = link.addr();
                stand_ins.lock().unwrap().insert(addr);
                let reason = link.closed().await;
                stand_ins.lock().unwrap().remove(&addr);
                eprintln!("relay link ended: {reason}");
            }
            Err(error) => eprintln!("relay: {error}"),
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// The main window's setting: the URL, or null when off. Before [`start`] has read the saved file it is
/// an error, not "off": the window keeps the switch disabled and asks again (WN-139, a saved "on" showed off).
#[tauri::command]
pub fn relay_setting(state: tauri::State<'_, Relay>) -> Result<Option<String>, String> {
    if state.path.lock().unwrap().is_none() {
        return Err(STARTING.into());
    }
    Ok(state.url.lock().unwrap().clone())
}

const STARTING: &str = "WindConnect jeszcze się uruchamia";

/// Turns the relay on with `url` or off (null); saved for the next start. A refused URL returns its error.
#[tauri::command]
pub fn set_relay(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Relay>,
    url: Option<String>,
) -> Result<Option<String>, String> {
    if window.label() != "main" {
        return Err("unsupported window".into());
    }
    let url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    if let Some(url) = &url {
        check_url(url).map_err(|e| e.to_string())?;
    }
    if let Some(path) = state.path.lock().unwrap().as_ref() {
        let saved = match &url {
            Some(url) => std::fs::write(path, url),
            None => std::fs::remove_file(path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            }),
        };
        saved.map_err(|e| format!("nie udało się zapisać ustawienia: {e}"))?;
    } else {
        return Err(STARTING.into());
    }
    *state.url.lock().unwrap() = url.clone();
    if url.is_none() {
        // Before the keepers are aborted: their stand-ins still name the relay sessions.
        let closed = close_relay_sessions(&state);
        if closed > 0 {
            eprintln!("relay off: closed {closed} session(s) running over the relay");
        }
    }
    if let Some(wake) = state.wake.lock().unwrap().as_ref() {
        wake.send_replace(url.clone());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Probe(Rc<Cell<u32>>);
    impl Abort for Probe {
        fn abort(&self) {
            self.0.set(self.0.get() + 1)
        }
    }

    fn key() -> PublicKey {
        DeviceIdentity::generate().unwrap().public_key()
    }

    /// CCS/1 TEST:one keeper per paired phone; a new URL replaces them; off aborts all of them at once.
    #[test]
    fn keepers_follow_url_and_paired_list() {
        let aborted = Rc::new(Cell::new(0));
        let spawned = Rc::new(Cell::new(0));
        let mut keepers = HashMap::new();
        let (a, b) = (key(), key());
        let spawn = |_: &str, _: PublicKey| {
            spawned.set(spawned.get() + 1);
            Probe(aborted.clone())
        };
        reconcile(&mut keepers, Some("wss://r.example"), &[a], spawn);
        assert_eq!((keepers.len(), spawned.get()), (1, 1));
        reconcile(&mut keepers, Some("wss://r.example"), &[a, b], |u, p| {
            spawn(u, p)
        });
        assert_eq!((keepers.len(), spawned.get(), aborted.get()), (2, 2, 0));
        reconcile(&mut keepers, Some("wss://r.example"), &[b], |u, p| {
            spawn(u, p)
        });
        assert_eq!((keepers.len(), aborted.get()), (1, 1));
        reconcile(&mut keepers, Some("wss://other.example"), &[b], |u, p| {
            spawn(u, p)
        });
        assert_eq!((keepers.len(), spawned.get(), aborted.get()), (1, 3, 2));
        reconcile(&mut keepers, None, &[a, b], |u, p| spawn(u, p));
        assert_eq!((keepers.len(), spawned.get(), aborted.get()), (0, 3, 3));
    }

    #[test]
    fn stand_in_matches_v4_and_v4_mapped_v6() {
        let v4: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        assert!(same_addr(v4, "[::ffff:127.0.0.1]:50000".parse().unwrap()));
        assert!(!same_addr(v4, "127.0.0.1:50001".parse().unwrap()));
        assert!(!same_addr(v4, "192.168.1.5:50000".parse().unwrap()));
    }

    /// CCS/1 CHECK:WN-139 off closes a live session over the relay at once (not at the QUIC idle timeout).
    /// Needs `npm run dev` in relay/, then
    /// `RELAY_URL=ws://127.0.0.1:8787 cargo test --bin windconnect-windows relay::tests::off_closes -- --ignored`.
    #[test]
    #[ignore = "needs the relay Worker running locally (wrangler dev) and RELAY_URL"]
    fn off_closes_a_session_running_over_the_relay() {
        use wind_core::protocol::DeviceKind;
        use wind_core::relay::RelayLink;
        use wind_core::session::{LocalInfo, Router, Session};
        use wind_core::transport::PinnedKeys;
        let info = |name: &str| LocalInfo { name: name.into(), kind: DeviceKind::Computer, app: None };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async {
            let url = std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:8787".into());
            let device = || {
                let id = DeviceIdentity::generate().unwrap();
                let pins = Arc::new(PinnedKeys::new());
                let endpoint = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &id, pins.clone()).unwrap();
                (id, pins, endpoint)
            };
            let ((pc, pc_pins, pc_end), (phone, phone_pins, phone_end)) = (device(), device());
            pc_pins.pin(phone.public_key());
            phone_pins.pin(pc.public_key());
            let pc_link = RelayLink::connect(&url, &pc, phone.public_key(), pc_end.clone()).await.unwrap();
            let accepting = pc_end.clone();
            let accepted = tokio::spawn(async move {
                let conn = accepting.accept().await.unwrap().unwrap();
                Session::accept(conn, info("pc"), Router::new()).await.unwrap().0
            });
            let phone_link = RelayLink::connect(&url, &phone, pc.public_key(), phone_end.clone()).await.unwrap();
            let t = Duration::from_secs(10);
            let (phone_session, _events) = tokio::time::timeout(
                t, Session::dial(&phone_end, phone_link.addr(), pc.public_key(), info("phone"), Router::new()),
            ).await.unwrap().unwrap();
            let pc_session = tokio::time::timeout(t, accepted).await.unwrap().unwrap();

            let relay = Relay::default();
            relay.stand_ins.lock().unwrap().insert(pc_link.addr());
            relay.sessions.lock().unwrap().push(pc_session.connection().clone());
            let off = std::time::Instant::now();
            assert_eq!(close_relay_sessions(&relay), 1, "the session's peer is the PC link's stand-in");
            tokio::time::timeout(Duration::from_secs(2), phone_session.connection().quinn().closed())
                .await
                .expect("the phone sees the session end at once");
            println!("relay off -> phone session closed in {:?}", off.elapsed());
            assert!(relay.stand_ins.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn saved_url_is_read_back_and_a_refused_one_is_off() {
        let dir = std::env::temp_dir().join(format!("wn92-relay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("relay-url");
        std::fs::write(&path, "wss://windconnect-relay.example.workers.dev\n").unwrap();
        assert_eq!(
            read(&path).as_deref(),
            Some("wss://windconnect-relay.example.workers.dev")
        );
        std::fs::write(&path, "http://nope").unwrap();
        assert_eq!(read(&path), None);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read(&path), None);
        let _ = std::fs::remove_dir(&dir);
    }
}
