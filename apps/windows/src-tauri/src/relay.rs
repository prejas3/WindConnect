//! CCS/1 M:relay setting (WN-92) | IO:main window switch + URL -> relay-url file -> one RelayLink keeper per paired phone.
//! RULE:off by default (WN-60 D3); off aborts every keeper at once, so nothing more goes through the relay.
//! Same keeper as `windctl serve --relay`: reconnect with backoff whenever the link ends.
use std::collections::HashMap;
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

async fn keep(url: String, identity: Arc<DeviceIdentity>, peer: PublicKey, endpoint: Endpoint) {
    let mut backoff =
        wind_core::transport::Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    loop {
        match RelayLink::connect(&url, &identity, peer, endpoint.clone()).await {
            Ok(link) => {
                backoff.reset();
                let reason = link.closed().await;
                eprintln!("relay link ended: {reason}");
            }
            Err(error) => eprintln!("relay: {error}"),
        }
        tokio::time::sleep(backoff.next_delay()).await;
    }
}

/// The main window's setting: the URL, or null when off.
#[tauri::command]
pub fn relay_setting(state: tauri::State<'_, Relay>) -> Option<String> {
    state.url.lock().unwrap().clone()
}

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
        return Err("WindConnect jeszcze się uruchamia".into());
    }
    *state.url.lock().unwrap() = url.clone();
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
