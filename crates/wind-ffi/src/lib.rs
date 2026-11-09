//! CCS/1 M:wind-ffi | PURPOSE:UniFFI boundary for Android
//! SEC:identity storage errors never generate replacement keys; explicit pinning, mTLS.

use std::{fmt, io, sync::Arc, time::Duration};
use wind_core::{
    identity::{DeviceIdentity, KeyStore, PublicKey},
    network::{DiscoverySnapshot, NetworkChanges, NetworkDiscovery},
    protocol::{DeviceKind, ErrorCode},
    session::{LocalInfo, Router, Session},
    transport::{Endpoint, PinnedKeys},
};
use zeroize::{Zeroize, Zeroizing};

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum CoreError {
    InvalidInput { reason: String },
    Storage { reason: String },
    Transport { reason: String },
    UntrustedPeer { reason: String },
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::InvalidInput { reason }
            | Self::Storage { reason }
            | Self::Transport { reason }
            | Self::UntrustedPeer { reason } => reason,
        };
        f.write_str(reason)
    }
}

impl std::error::Error for CoreError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for CoreError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Storage {
            reason: error.to_string(),
        }
    }
}

/// CCS/1 IO:encrypted platform store <-> PKCS#8 PEM bytes
/// RULE:store_new is atomic create-only; false means load the race winner.
#[uniffi::export(callback_interface)]
pub trait IdentityStorage: Send + Sync {
    fn load(&self) -> Result<Option<Vec<u8>>, CoreError>;
    fn store_new(&self, pem: Vec<u8>) -> Result<bool, CoreError>;
}

struct PlatformStore(Box<dyn IdentityStorage>);

impl KeyStore for PlatformStore {
    fn load(&self) -> io::Result<Option<Zeroizing<String>>> {
        let Some(mut bytes) = self.0.load().map_err(io::Error::other)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&bytes).map(|s| Zeroizing::new(s.to_owned()));
        bytes.zeroize();
        text.map(Some)
            .map_err(|_| io::Error::other("identity PEM is not UTF-8"))
    }

    fn store_new(&self, pem: &str) -> io::Result<bool> {
        self.0
            .store_new(pem.as_bytes().to_vec())
            .map_err(io::Error::other)
    }
}

fn public_key(bytes: &[u8]) -> Result<PublicKey, CoreError> {
    let bytes = bytes.try_into().map_err(|_| CoreError::InvalidInput {
        reason: "Ed25519 public key must contain 32 bytes".into(),
    })?;
    PublicKey::from_bytes(bytes).ok_or_else(|| CoreError::InvalidInput {
        reason: "invalid Ed25519 public key".into(),
    })
}

fn session_error(error: wind_core::session::SessionError) -> CoreError {
    if error.is_untrusted_peer() {
        CoreError::UntrustedPeer {
            reason: error.to_string(),
        }
    } else {
        CoreError::Transport {
            reason: error.to_string(),
        }
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct IdentityInfo {
    pub public_key: Vec<u8>,
    pub fingerprint: String,
    pub display_fingerprint: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PeerInfo {
    pub public_key: Vec<u8>,
    pub name: String,
    pub fingerprint: String,
}

#[derive(uniffi::Object)]
pub struct CoreClient {
    endpoint: Endpoint,
    trust: Arc<PinnedKeys>,
    identity: IdentityInfo,
    local: LocalInfo,
    bind_ip: std::net::IpAddr,
    network: Arc<NetworkChanges>,
    discovery: tokio::sync::Mutex<Option<NetworkDiscovery>>,
    discovered: tokio::sync::Mutex<Option<tokio::sync::watch::Receiver<DiscoverySnapshot>>>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DiscoveredDevice {
    pub fingerprint: String,
    pub addresses: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct NetworkSnapshot {
    pub generation: u64,
    pub peers: Vec<DiscoveredDevice>,
    pub error: Option<String>,
}

/// CCS/1 FLOW:Kotlin IO coroutine -> load/create identity -> Tokio QUIC endpoint
/// DEC:UniFFI's Tokio compatibility runtime drives every exported async operation.
#[uniffi::export(async_runtime = "tokio")]
pub async fn open_client(
    storage: Box<dyn IdentityStorage>,
    name: String,
    bind_address: String,
) -> Result<Arc<CoreClient>, CoreError> {
    if name.is_empty() || name.len() > 64 {
        return Err(CoreError::InvalidInput {
            reason: "device name must be 1–64 UTF-8 bytes".into(),
        });
    }
    let address: std::net::SocketAddr =
        bind_address.parse().map_err(|_| CoreError::InvalidInput {
            reason: "bind address must be an IP address and port".into(),
        })?;
    let identity = DeviceIdentity::load_or_create(&PlatformStore(storage)).map_err(|e| {
        CoreError::Storage {
            reason: e.to_string(),
        }
    })?;
    let trust = Arc::new(PinnedKeys::new());
    let endpoint =
        Endpoint::bind(address, &identity, trust.clone()).map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })?;
    Ok(Arc::new(CoreClient {
        endpoint,
        trust,
        bind_ip: address.ip(),
        network: Arc::new(NetworkChanges::default()),
        discovery: tokio::sync::Mutex::new(None),
        discovered: tokio::sync::Mutex::new(None),
        identity: IdentityInfo {
            public_key: identity.public_key().as_bytes().to_vec(),
            fingerprint: identity.fingerprint().to_hex(),
            display_fingerprint: identity.fingerprint().to_string(),
        },
        local: LocalInfo {
            name,
            kind: DeviceKind::Phone,
            app: Some(wind_core::VERSION.into()),
        },
    }))
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreClient {
    pub async fn start_discovery(&self) -> Result<(), CoreError> {
        let mut discovery = self.discovery.lock().await;
        if discovery.is_none() {
            let key = public_key(&self.identity.public_key)?;
            let watcher = NetworkDiscovery::start(key, self.endpoint.clone(), self.network.clone())
                .map_err(|error| CoreError::Transport {
                    reason: error.to_string(),
                })?;
            *self.discovered.lock().await = Some(watcher.subscribe());
            *discovery = Some(watcher);
        }
        Ok(())
    }

    /// Platform callbacks call this off their callback thread. Does not acquire discovery locks.
    pub async fn network_changed(&self) -> Result<(), CoreError> {
        let result = self
            .endpoint
            .rebind(std::net::SocketAddr::new(self.bind_ip, 0));
        self.network.notify();
        result.map(|_| ()).map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })
    }

    pub async fn next_network_snapshot(&self) -> Result<NetworkSnapshot, CoreError> {
        let mut receiver = self.discovered.lock().await;
        let receiver = receiver.as_mut().ok_or_else(|| CoreError::Transport {
            reason: "discovery is not started".into(),
        })?;
        receiver.changed().await.map_err(|e| CoreError::Transport {
            reason: e.to_string(),
        })?;
        let state = receiver.borrow_and_update().clone();
        Ok(NetworkSnapshot {
            generation: state.generation,
            error: state.error,
            peers: state
                .peers
                .into_iter()
                .map(|peer| DiscoveredDevice {
                    fingerprint: peer
                        .fingerprint
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                    addresses: peer.addresses.into_iter().map(|a| a.to_string()).collect(),
                })
                .collect(),
        })
    }
    pub fn identity(&self) -> IdentityInfo {
        self.identity.clone()
    }

    pub fn local_address(&self) -> Result<String, CoreError> {
        self.endpoint
            .local_addr()
            .map(|a| a.to_string())
            .map_err(|e| CoreError::Transport {
                reason: e.to_string(),
            })
    }

    /// Explicit caller authorization only; never call this from discovery.
    pub fn pin_peer(&self, key: Vec<u8>) -> Result<(), CoreError> {
        self.trust.pin(public_key(&key)?);
        Ok(())
    }

    pub async fn connect(
        &self,
        address: String,
        key: Vec<u8>,
    ) -> Result<Arc<CoreConnection>, CoreError> {
        let address = address.parse().map_err(|_| CoreError::InvalidInput {
            reason: "peer address must be an IP address and port".into(),
        })?;
        let mut changes = self.network.subscribe();
        let (session, _) = loop {
            let attempt = tokio::time::timeout(
            Duration::from_secs(10),
            Session::dial(
                &self.endpoint,
                address,
                public_key(&key)?,
                self.local.clone(),
                Router::new(),
            ),
        );
            tokio::select! {
                result = attempt => break result,
                changed = changes.changed() => {
                    if changed.is_err() { return Err(CoreError::Transport { reason: "network observer stopped".into() }); }
                }
            }
        }
        .map_err(|_| CoreError::Transport {
            reason: "connection timed out".into(),
        })?
        .map_err(session_error)?;
        Ok(Arc::new(CoreConnection { session }))
    }

    pub async fn shutdown(&self) {
        if let Some(mut discovery) = self.discovery.lock().await.take() {
            discovery.shutdown().await;
        }
        self.endpoint.close();
        self.endpoint.wait_idle().await;
    }
}

impl Drop for CoreClient {
    fn drop(&mut self) {
        self.endpoint.close();
    }
}

#[derive(uniffi::Object)]
pub struct CoreConnection {
    session: Session,
}

#[uniffi::export(async_runtime = "tokio")]
impl CoreConnection {
    pub fn peer(&self) -> PeerInfo {
        let peer = self.session.peer();
        PeerInfo {
            public_key: peer.key.as_bytes().to_vec(),
            name: peer.hello.name.clone(),
            fingerprint: peer.key.fingerprint().to_hex(),
        }
    }

    pub async fn ping(&self) -> Result<u64, CoreError> {
        self.session
            .ping()
            .await
            .map(|d| d.as_micros() as u64)
            .map_err(session_error)
    }

    pub async fn wait_closed(&self) -> String {
        self.session.closed().await.to_string()
    }

    pub async fn disconnect(&self) {
        self.session.goodbye(ErrorCode::GoingAway, None).await;
    }
}

impl Drop for CoreConnection {
    fn drop(&mut self) {
        self.session.connection().close(ErrorCode::GoingAway, b"");
    }
}
