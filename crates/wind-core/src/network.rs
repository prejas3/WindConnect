//! CCS/1 FLOW:OS callback -> coalesced generation -> immediate discovery/retry
//! SEC:network events do not change peer trust.
use crate::{
    broadcast::{BroadcastConfig, BroadcastDiscovery},
    discovery::{DiscoveredPeer, Discovery, DiscoveryEvent},
    identity::PublicKey,
    transport::{Endpoint, PinnedKeys},
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
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
    /// Paired devices: the broadcast cache always makes room for them
    /// (PROTOCOL.md §2.0.1). Only a priority; it grants no trust.
    pub paired: Option<Arc<PinnedKeys>>,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self {
            mdns: true,
            broadcast: Some(BroadcastConfig::default()),
            paired: None,
        }
    }
}

/// CCS/1 RULE:our name is advertised again no sooner than this after our last
/// goodbye for it; mdns-sd probing adds ~0.8 s, so the announcement lands well
/// after the second a receiver keeps the goodbye (WN-104, `Discovery::start_unadvertised`).
const GOODBYE_HOLD: Duration = Duration::from_secs(1);

/// CCS/1 RULE:a TXT change (pairing port) waits until this long after our last
/// registration. mdns-sd 0.21.7 announces a service whose earlier probe just
/// finished without the TXT still being probed, marks the interface announced,
/// and so never sends that TXT there (`probing_handler`): a SAS window closed
/// ~1 s after a restart stayed advertised as open (WN-104).
const PROBE_SETTLE: Duration = Duration::from_secs(2);

/// Most fingerprints the broadcast cache holds (§2.0.1).
pub const BROADCAST_CACHE_LIMIT: usize = 128;

struct BroadcastEntry {
    peer: DiscoveredPeer,
    source: Ipv4Addr,
    seen: Instant,
    paired: bool,
}

/// CCS/1 SEC:untrusted broadcast cache, bounded; floods cannot shut out paired peers (WN-87).
/// One unpaired fingerprint per source IP (a newer one replaces it); when
/// full, the least recently refreshed unpaired entry makes room. A paired
/// fingerprint is always taken; unpaired ones never displace paired ones.
#[derive(Default)]
struct BroadcastCache {
    entries: BTreeMap<[u8; 32], BroadcastEntry>,
}

impl BroadcastCache {
    /// True if the cache changed. `is_paired` is asked only for a new fingerprint.
    fn offer(
        &mut self,
        peer: DiscoveredPeer,
        source: Ipv4Addr,
        now: Instant,
        is_paired: impl FnOnce(&[u8; 32]) -> bool,
    ) -> bool {
        let fingerprint = peer.fingerprint;
        if let Some(entry) = self.entries.get_mut(&fingerprint) {
            entry.peer = peer;
            entry.source = source;
            entry.seen = now;
            if !entry.paired {
                self.entries.retain(|key, other| {
                    *key == fingerprint || other.paired || other.source != source
                });
            }
            return true;
        }
        let paired = is_paired(&fingerprint);
        if !paired {
            self.entries
                .retain(|_, other| other.paired || other.source != source);
        }
        if self.entries.len() >= BROADCAST_CACHE_LIMIT {
            let oldest = |paired: bool| {
                self.entries
                    .iter()
                    .filter(|(_, entry)| entry.paired == paired)
                    .min_by_key(|(_, entry)| entry.seen)
                    .map(|(key, _)| *key)
            };
            let victim = match (oldest(false), paired) {
                (Some(key), _) => key,
                (None, true) => match oldest(true) {
                    Some(key) => key,
                    None => return false,
                },
                (None, false) => return false,
            };
            self.entries.remove(&victim);
        }
        self.entries.insert(
            fingerprint,
            BroadcastEntry {
                peer,
                source,
                seen: now,
                paired,
            },
        );
        true
    }

    /// True if something expired.
    fn expire(&mut self, expiry: Duration) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|_, entry| entry.seen.elapsed() < expiry);
        before != self.entries.len()
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn peers(&self) -> impl Iterator<Item = &DiscoveredPeer> {
        self.entries.values().map(|entry| &entry.peer)
    }
}

fn merged_peers(
    mdns: &BTreeMap<String, DiscoveredPeer>,
    broadcast: &BroadcastCache,
) -> Vec<DiscoveredPeer> {
    let mut merged = BTreeMap::new();
    for peer in mdns.values().chain(broadcast.peers()) {
        let entry = merged
            .entry(peer.fingerprint)
            .or_insert_with(|| peer.clone());
        entry.addresses.extend(peer.addresses.iter().copied());
        entry.addresses.sort_unstable();
        entry.addresses.dedup();
        entry.pairing_port = entry.pairing_port.or(peer.pairing_port);
    }
    merged.into_values().collect()
}

/// Owns the discovery task and, on Windows, its OS callback registration.
pub struct NetworkDiscovery {
    _changes: Arc<NetworkChanges>,
    pairing: Arc<watch::Sender<Option<u16>>>,
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
        let pairing = Arc::new(watch::channel(None::<u16>).0);
        let mut pairing_changes = pairing.subscribe();
        // CCS/1 RULE:no initial report (WN-89). The daemons start below, after
        // registration, so every later change still arrives. Restarting mDNS
        // for the initial report stopped the first daemon while it was still
        // probing and started a second one with the same name; a browser
        // started meanwhile (same host, measured on Windows) then missed the
        // device for over 5 s in about half of the runs.
        #[cfg(windows)]
        let windows = WindowsNetworkMonitor::start_with(changes.clone(), false)?;
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
            let mut broadcast_peers = BroadcastCache::default();
            let mut replied: BTreeMap<std::net::Ipv4Addr, Instant> = BTreeMap::new();
            let period = options
                .broadcast
                .as_ref()
                .map(|config| config.interval)
                .unwrap_or(Duration::from_secs(2));
            // CCS/1 FLOW:network change -> new daemon browses now; advertises at last goodbye + GOODBYE_HOLD.
            let mut last_goodbye: Option<Instant> = None;
            let mut advertise_at: Option<Instant> = None;
            let mut registered_at: Option<Instant> = discovery.as_ref().map(|_| Instant::now());
            let mut pairing_due: Option<Instant> = None;
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
                            .and_then(|addr| Discovery::start_unadvertised(&key, addr.port(), *pairing_changes.borrow()).map_err(std::io::Error::other))
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
                                // A daemon replaced before it advertised retracts nothing: a burst of changes sends one goodbye.
                                let goodbye = old.as_ref().is_some_and(Discovery::is_advertised);
                                let _ = tokio::task::spawn_blocking(move || drop(old)).await;
                                if goodbye { last_goodbye = Some(Instant::now()); }
                                advertise_at = Some(last_goodbye.map_or_else(Instant::now, |at| at + GOODBYE_HOLD));
                                // The new daemon starts with the current pairing port; advertise() registers it.
                                registered_at = None;
                                pairing_due = None;
                            }
                            Err(error) if options.mdns => { sender.send_modify(|state| state.error = Some(error.to_string())); }
                            _ => {},
                        }
                    }
                    _ = async { match advertise_at {
                        Some(at) => tokio::time::sleep_until(at.into()).await,
                        None => std::future::pending().await,
                    }} => {
                        advertise_at = None;
                        if let Some(discovery) = &discovery {
                            match discovery.advertise() {
                                Ok(()) => registered_at = Some(Instant::now()),
                                Err(error) => sender.send_modify(|state| state.error = Some(error.to_string())),
                            }
                        }
                    }
                    changed = pairing_changes.changed() => {
                        if changed.is_err() { break; }
                        let port = *pairing_changes.borrow_and_update();
                        if let Some(discovery) = &discovery {
                            match registered_at {
                                Some(at) if discovery.is_advertised() && at.elapsed() < PROBE_SETTLE => pairing_due = Some(at + PROBE_SETTLE),
                                _ => {
                                    if let Err(error) = discovery.set_pairing_port(port) { sender.send_modify(|state| state.error = Some(error.to_string())); }
                                    if discovery.is_advertised() { registered_at = Some(Instant::now()); }
                                }
                            }
                        }
                    }
                    _ = async { match pairing_due {
                        Some(at) => tokio::time::sleep_until(at.into()).await,
                        None => std::future::pending().await,
                    }} => {
                        pairing_due = None;
                        if let Some(discovery) = &discovery {
                            // The latest value: several changes inside the window become one registration.
                            if let Err(error) = discovery.set_pairing_port(*pairing_changes.borrow()) { sender.send_modify(|state| state.error = Some(error.to_string())); }
                            registered_at = Some(Instant::now());
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
                            if broadcast_peers.expire(backend.config().expiry) { sender.send_modify(|state| state.peers = merged_peers(&peers, &broadcast_peers)); }
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
                                fingerprint: packet.fingerprint, addresses: vec![SocketAddr::new(IpAddr::V4(*source.ip()), packet.port)], pairing_port: None };
                            // CCS/1 SEC:bound untrusted cache and same-size unicast replies, at most one/IP/second.
                            let paired = |fingerprint: &[u8; 32]| options.paired.as_ref().is_some_and(|pins| pins.contains_fingerprint(fingerprint));
                            if broadcast_peers.offer(peer, *source.ip(), Instant::now(), paired) {
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
            pairing,
            state,
            task: Some(task),
            #[cfg(windows)]
            _windows: windows,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<DiscoverySnapshot> {
        self.state.clone()
    }

    /// Advertises (`Some`) or withdraws (`None`) TXT `pair` (§12.4); kept
    /// across network changes. Port 0 withdraws.
    pub fn set_pairing_port(&self, port: Option<u16>) {
        set_pairing(&self.pairing, port);
    }

    /// CCS/1 FLOW:PairingListener SAS window -> this advertiser -> TXT `pair`
    /// For [`crate::pairing::PairingListener::advertise_with`]; outlives nothing it needs.
    pub fn pairing_advertiser(&self) -> crate::pairing::PairingAdvertiser {
        let pairing = Arc::downgrade(&self.pairing);
        Arc::new(move |port| {
            if let Some(pairing) = pairing.upgrade() {
                set_pairing(&pairing, port);
            }
        })
    }

    pub async fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

/// Wakes the discovery task only when the advertised value changes.
fn set_pairing(pairing: &watch::Sender<Option<u16>>, port: Option<u16>) {
    let port = port.filter(|p| *p != 0);
    pairing.send_if_modified(|current| std::mem::replace(current, port) != port);
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
        /// Signals every connectivity change, and the current state once at
        /// registration.
        pub fn start(changes: Arc<NetworkChanges>) -> io::Result<Self> {
            Self::start_with(changes, true)
        }

        /// [`start`](Self::start); `report_current: false` signals only
        /// changes after registration.
        pub fn start_with(changes: Arc<NetworkChanges>, report_current: bool) -> io::Result<Self> {
            let context = changes;
            let mut handle: HANDLE = ptr::null_mut();
            // CCS/1 RULE:stable Arc context until cancellation drains in-flight callbacks.
            let status = unsafe {
                NotifyNetworkConnectivityHintChange(
                    Some(on_change),
                    Arc::as_ptr(&context).cast(),
                    report_current,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(fingerprint: [u8; 32], ip: Ipv4Addr) -> DiscoveredPeer {
        DiscoveredPeer {
            service_name: String::new(),
            fingerprint,
            addresses: vec![SocketAddr::new(IpAddr::V4(ip), 1)],
            pairing_port: None,
        }
    }
    fn fake(i: usize) -> [u8; 32] {
        let mut fingerprint = [0xEE; 32];
        fingerprint[..8].copy_from_slice(&(i as u64).to_be_bytes());
        fingerprint
    }
    fn ip(net: u8, i: usize) -> Ipv4Addr {
        Ipv4Addr::new(10, net, (i / 250) as u8, (i % 250) as u8 + 1)
    }
    const PHONE: [u8; 32] = [7; 32];

    /// Spoofed source IPs: every fake fingerprint from its own IP, refreshed
    /// each round before the phone; the paired phone still gets in, and
    /// the limit holds.
    #[test]
    fn paired_fingerprint_gets_in_when_spoofed_ips_fill_the_cache() {
        let mut cache = BroadcastCache::default();
        let start = Instant::now();
        let paired = |f: &[u8; 32]| *f == PHONE;
        let phone = Ipv4Addr::new(192, 168, 1, 20);
        for round in 0..3u64 {
            let now = start + Duration::from_secs(2 * round);
            for i in 0..200 {
                cache.offer(peer(fake(i), ip(0, i)), ip(0, i), now, paired);
                assert!(cache.entries.len() <= BROADCAST_CACHE_LIMIT);
            }
            assert!(cache.offer(peer(PHONE, phone), phone, now, paired));
            assert!(cache.entries.contains_key(&PHONE));
            assert_eq!(cache.entries.len(), BROADCAST_CACHE_LIMIT);
        }
        // Many more fakes never displace it.
        for i in 0..400 {
            let at = start + Duration::from_secs(9);
            cache.offer(peer(fake(1000 + i), ip(1, i)), ip(1, i), at, paired);
        }
        assert!(cache.entries.contains_key(&PHONE));
        assert_eq!(cache.entries.len(), BROADCAST_CACHE_LIMIT);
    }

    #[test]
    fn one_unpaired_fingerprint_per_source_ip() {
        let mut cache = BroadcastCache::default();
        let now = Instant::now();
        let spammer = Ipv4Addr::new(10, 0, 0, 66);
        for i in 0..128 {
            cache.offer(peer(fake(i), spammer), spammer, now, |_| false);
        }
        assert_eq!(cache.entries.len(), 1);
        assert!(
            cache.entries.contains_key(&fake(127)),
            "the newest replaces"
        );
        let laptop = Ipv4Addr::new(10, 0, 0, 5);
        assert!(cache.offer(peer([5; 32], laptop), laptop, now, |_| false));
        assert_eq!(cache.entries.len(), 2);
        // A paired fingerprint shares an IP; an unpaired one from that IP
        // replaces only the unpaired entry there.
        assert!(cache.offer(peer(PHONE, laptop), laptop, now, |f| *f == PHONE));
        assert!(cache.offer(peer(fake(999), laptop), laptop, now, |_| false));
        assert!(cache.entries.contains_key(&PHONE));
        assert!(!cache.entries.contains_key(&[5; 32]));
        assert_eq!(cache.entries.len(), 3);
    }

    #[test]
    fn full_of_paired_drops_unpaired_and_evicts_oldest_paired() {
        let mut cache = BroadcastCache::default();
        let start = Instant::now();
        for i in 0..BROADCAST_CACHE_LIMIT {
            let at = start + Duration::from_millis(i as u64);
            cache.offer(peer(fake(i), ip(0, i)), ip(0, i), at, |_| true);
        }
        let later = start + Duration::from_secs(1);
        let other = Ipv4Addr::new(10, 9, 9, 9);
        assert!(!cache.offer(peer([1; 32], other), other, later, |_| false));
        assert!(cache.offer(peer([2; 32], other), other, later, |_| true));
        assert!(
            !cache.entries.contains_key(&fake(0)),
            "oldest paired made room"
        );
        assert_eq!(cache.entries.len(), BROADCAST_CACHE_LIMIT);
    }
}
