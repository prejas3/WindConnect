//! CCS/1 FLOW:OS callback -> coalesced generation -> immediate discovery/retry
//! SEC:network events do not change peer trust.
use crate::{
    broadcast::{BroadcastConfig, BroadcastDiscovery},
    discovery::{DiscoveredPeer, Discovery, DiscoveryEvent},
    identity::PublicKey,
    transport::Endpoint,
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinHandle};

#[derive(Clone)]
pub struct NetworkChanges {
    generation: watch::Sender<u64>,
}

impl Default for NetworkChanges {
    fn default() -> Self {
        Self {
            generation: watch::channel(0).0,
        }
    }
}

impl NetworkChanges {
    pub fn notify(&self) {
        self.generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }
    pub fn generation(&self) -> u64 {
        *self.generation.borrow()
    }
}

#[derive(Clone, Debug, Default)]
pub struct DiscoverySnapshot {
    pub generation: u64,
    pub peers: Vec<DiscoveredPeer>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DiscoveryOptions {
    pub mdns: bool,
    pub broadcast: Option<BroadcastConfig>,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            mdns: true,
            broadcast: Some(BroadcastConfig::default()),
        }
    }
}

fn merged_peers(
    mdns: &BTreeMap<String, DiscoveredPeer>,
    broadcast: &BTreeMap<[u8; 32], (DiscoveredPeer, Instant)>,
) -> Vec<DiscoveredPeer> {
    let mut merged = BTreeMap::new();
    for peer in mdns
        .values()
        .chain(broadcast.values().map(|(peer, _)| peer))
    {
        let entry = merged
            .entry(peer.fingerprint)
            .or_insert_with(|| peer.clone());
        entry.addresses.extend(peer.addresses.iter().copied());
        entry.addresses.sort_unstable();
        entry.addresses.dedup();
    }
    merged.into_values().collect()
}

/// Owns the discovery task and, on Windows, its OS callback registration.
pub struct NetworkDiscovery {
    _changes: Arc<NetworkChanges>,
    state: watch::Receiver<DiscoverySnapshot>,
    task: Option<JoinHandle<()>>,
    #[cfg(windows)]
    _windows: WindowsNetworkMonitor,
}

impl NetworkDiscovery {
    /// Requires a Tokio runtime. Recreates sockets/interfaces on each network generation.
    pub fn start(
        key: PublicKey,
        endpoint: Endpoint,
        changes: Arc<NetworkChanges>,
    ) -> std::io::Result<Self> {
        Self::start_with_options(key, endpoint, changes, DiscoveryOptions::default())
    }

    pub fn start_with_options(
        key: PublicKey,
        endpoint: Endpoint,
        changes: Arc<NetworkChanges>,
        options: DiscoveryOptions,
    ) -> std::io::Result<Self> {
        if !options.mdns && options.broadcast.is_none() {
            return Err(std::io::Error::other("no discovery backend enabled"));
        }
        let mut network = changes.subscribe();
        #[cfg(windows)]
        let windows = WindowsNetworkMonitor::start(changes.clone())?;
        let port = endpoint.local_addr()?.port();
        let discovery = options.mdns.then(|| Discovery::start(&key, port));
        let broadcast = options
            .broadcast
            .as_ref()
            .map(|config| BroadcastDiscovery::bind(*key.fingerprint().as_bytes(), config.clone()));
        let initial = DiscoverySnapshot {
            error: discovery
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(ToString::to_string)
                .or_else(|| {
                    broadcast
                        .as_ref()
                        .and_then(|result| result.as_ref().err())
                        .map(ToString::to_string)
                }),
            ..Default::default()
        };
        let (sender, state) = watch::channel(initial);
        let task = tokio::spawn(async move {
            // CCS/1 RULE:offline startup remains subscribed; next OS event retries a failed daemon.
            let mut discovery = discovery.and_then(Result::ok);
            let mut broadcast = broadcast.and_then(Result::ok);
            let mut peers = BTreeMap::new();
            let mut broadcast_peers = BTreeMap::new();
            let mut replied: BTreeMap<std::net::Ipv4Addr, Instant> = BTreeMap::new();
            let period = options
                .broadcast
                .as_ref()
                .map(|config| config.interval)
                .unwrap_or(Duration::from_secs(2));
            let mut tick = tokio::time::interval(period.max(Duration::from_millis(10)));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    changed = network.changed() => {
                        if changed.is_err() { break; }
                        let generation = *network.borrow_and_update();
                        peers.clear();
                        broadcast_peers.clear();
                        replied.clear();
                        sender.send_replace(DiscoverySnapshot { generation, ..Default::default() });
                        let result = if options.mdns { endpoint.local_addr().map_err(std::io::Error::other)
                            .and_then(|addr| Discovery::start(&key, addr.port()).map_err(std::io::Error::other))
                            } else { Err(std::io::Error::other("mDNS disabled")) };
                        if let Some(config) = &options.broadcast {
                            if broadcast.is_none() { broadcast = BroadcastDiscovery::bind(*key.fingerprint().as_bytes(), config.clone()).ok(); }
                            if let Some(backend) = &broadcast {
                                if let Ok(address) = endpoint.local_addr() { let _ = backend.query(address.port()).await; }
                            }
                        }
                        match result {
                            Ok(new) => {
                                // CCS/1 RULE:new browse is sent before waiting for old goodbye cleanup.
                                let old = discovery.replace(new);
                                let _ = tokio::task::spawn_blocking(move || drop(old)).await;
                            }
                            Err(error) if options.mdns => { sender.send_modify(|state| state.error = Some(error.to_string())); }
                            _ => {},
                        }
                    }
                    event = async { match &discovery {
                        Some(discovery) => discovery.next().await,
                        None => std::future::pending().await,
                    }} => match event {
                        Ok(DiscoveryEvent::Resolved(peer)) => {
                            peers.insert(peer.service_name.clone(), peer);
                            sender.send_modify(|state| state.peers = merged_peers(&peers, &broadcast_peers));
                        }
                        Ok(DiscoveryEvent::Removed(name)) => {
                            peers.remove(&name);
                            sender.send_modify(|state| state.peers = merged_peers(&peers, &broadcast_peers));
                        }
                        Err(error) => { sender.send_modify(|state| state.error = Some(error.to_string())); discovery = None; }
                    },
                    _ = tick.tick(), if options.broadcast.is_some() => {
                        if let Some(backend) = &broadcast {
                            let before = broadcast_peers.len();
                            broadcast_peers.retain(|_, (_, seen): &mut (DiscoveredPeer, Instant)| seen.elapsed() < backend.config().expiry);
                            if before != broadcast_peers.len() { sender.send_modify(|state| state.peers = merged_peers(&peers, &broadcast_peers)); }
                            if let Ok(address) = endpoint.local_addr() {
                                if let Err(error) = backend.query(address.port()).await { sender.send_modify(|state| state.error = Some(error.to_string())); }
                            }
                        }
                    },
                    received = async { match &broadcast {
                        Some(backend) => backend.receive().await,
                        None => std::future::pending().await,
                    }} => match received {
                        Ok(Some((packet, source))) => {
                            let peer = DiscoveredPeer { service_name: format!("broadcast-{}", packet.fingerprint.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                                fingerprint: packet.fingerprint, addresses: vec![SocketAddr::new(IpAddr::V4(*source.ip()), packet.port)] };
                            // CCS/1 SEC:bound untrusted cache and same-size unicast replies, at most one/IP/second.
                            if broadcast_peers.len() < 128 || broadcast_peers.contains_key(&packet.fingerprint) {
                                broadcast_peers.insert(packet.fingerprint, (peer, Instant::now()));
                                sender.send_modify(|state| state.peers = merged_peers(&peers, &broadcast_peers));
                            }
                            replied.retain(|_, time| time.elapsed() < Duration::from_secs(1));
                            if packet.query && replied.len() < 128 && !replied.contains_key(source.ip()) {
                                replied.insert(*source.ip(), Instant::now());
                                if let (Some(backend), Ok(address)) = (&broadcast, endpoint.local_addr()) { let _ = backend.answer(address.port(), source).await; }
                            }
                        }
                        Ok(None) => {},
                        Err(error) => { sender.send_modify(|state| state.error = Some(error.to_string())); broadcast = None; }
                    },
                    event = async { match &discovery {
                        Some(discovery) => discovery.monitor().recv_async().await,
                        None => std::future::pending().await,
                    }} => match event {
                        Ok(mdns_sd::DaemonEvent::Error(error)) => { sender.send_modify(|state| state.error = Some(error.to_string())); }
                        Err(error) => { sender.send_modify(|state| state.error = Some(error.to_string())); discovery = None; }
                        _ => {},
                    },
                }
            }
        });
        Ok(Self {
            _changes: changes,
            state,
            task: Some(task),
            #[cfg(windows)]
            _windows: windows,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<DiscoverySnapshot> {
        self.state.clone()
    }

    pub async fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for NetworkDiscovery {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(windows)]
pub use windows::WindowsNetworkMonitor;

#[cfg(windows)]
mod windows {
    use super::NetworkChanges;
    use std::{ffi::c_void, io, ptr, sync::Arc};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        NetworkManagement::IpHelper::{
            CancelMibChangeNotify2, NotifyNetworkConnectivityHintChange,
        },
        Networking::WinSock::NL_NETWORK_CONNECTIVITY_HINT,
    };

    pub struct WindowsNetworkMonitor {
        handle: usize,
        context: Option<Arc<NetworkChanges>>,
    }

    impl WindowsNetworkMonitor {
        pub fn start(changes: Arc<NetworkChanges>) -> io::Result<Self> {
            let context = changes;
            let mut handle: HANDLE = ptr::null_mut();
            // CCS/1 RULE:stable Arc context until cancellation drains in-flight callbacks.
            let status = unsafe {
                NotifyNetworkConnectivityHintChange(
                    Some(on_change),
                    Arc::as_ptr(&context).cast(),
                    true,
                    &mut handle,
                )
            };
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            Ok(Self {
                handle: handle as usize,
                context: Some(context),
            })
        }
    }

    unsafe extern "system" fn on_change(context: *const c_void, _: NL_NETWORK_CONNECTIVITY_HINT) {
        // CCS/1 RULE:callback only signals; no network IO or self-unregistration.
        let changes = unsafe { &*context.cast::<NetworkChanges>() };
        changes.notify();
    }

    impl Drop for WindowsNetworkMonitor {
        fn drop(&mut self) {
            let status = unsafe { CancelMibChangeNotify2(self.handle as HANDLE) };
            if status != 0 {
                // CCS/1 SEC:preserve callback context if OS cancellation fails; prevents use-after-free.
                if let Some(context) = self.context.take() {
                    let _ = Arc::into_raw(context);
                }
            }
        }
    }
}
