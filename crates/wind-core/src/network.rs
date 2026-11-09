//! CCS/1 FLOW:OS callback -> coalesced generation -> immediate discovery/retry
//! SEC:network events do not change peer trust.
use crate::{
    discovery::{DiscoveredPeer, Discovery, DiscoveryEvent},
    identity::PublicKey,
    transport::Endpoint,
};
use std::{collections::BTreeMap, sync::Arc};
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
        let mut network = changes.subscribe();
        #[cfg(windows)]
        let windows = WindowsNetworkMonitor::start(changes.clone())?;
        let discovery = Discovery::start(&key, endpoint.local_addr()?.port());
        let initial = DiscoverySnapshot {
            error: discovery.as_ref().err().map(ToString::to_string),
            ..Default::default()
        };
        let (sender, state) = watch::channel(initial);
        let task = tokio::spawn(async move {
            // CCS/1 RULE:offline startup remains subscribed; next OS event retries a failed daemon.
            let mut discovery = discovery.ok();
            let mut peers = BTreeMap::new();
            loop {
                tokio::select! {
                    changed = network.changed() => {
                        if changed.is_err() { break; }
                        let generation = *network.borrow_and_update();
                        peers.clear();
                        sender.send_replace(DiscoverySnapshot { generation, ..Default::default() });
                        let result = endpoint.local_addr().map_err(std::io::Error::other)
                            .and_then(|addr| Discovery::start(&key, addr.port()).map_err(std::io::Error::other));
                        match result {
                            Ok(new) => {
                                // CCS/1 RULE:new browse is sent before waiting for old goodbye cleanup.
                                let old = discovery.replace(new);
                                let _ = tokio::task::spawn_blocking(move || drop(old)).await;
                            }
                            Err(error) => { sender.send_modify(|state| state.error = Some(error.to_string())); }
                        }
                    }
                    event = async { match &discovery {
                        Some(discovery) => discovery.next().await,
                        None => std::future::pending().await,
                    }} => match event {
                        Ok(DiscoveryEvent::Resolved(peer)) => {
                            peers.insert(peer.service_name.clone(), peer);
                            sender.send_modify(|state| state.peers = peers.values().cloned().collect());
                        }
                        Ok(DiscoveryEvent::Removed(name)) => {
                            peers.remove(&name);
                            sender.send_modify(|state| state.peers = peers.values().cloned().collect());
                        }
                        Err(error) => { sender.send_modify(|state| state.error = Some(error.to_string())); discovery = None; }
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
