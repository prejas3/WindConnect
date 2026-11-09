//! CCS/1 M:discovery | IO:mDNS -> untrusted QUIC address hints
//! SEC:TXT fingerprints never grant trust; transport still verifies pinned keys.

use std::net::{SocketAddr, SocketAddrV6};
use std::time::Duration;

use mdns_sd::{Receiver, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::identity::PublicKey;

pub const SERVICE_TYPE: &str = "_windconnect._udp.local.";

/// A resolved advertisement, not an authenticated device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub service_name: String,
    pub fingerprint: [u8; 32],
    pub addresses: Vec<SocketAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// Replaces the addresses previously reported for this service name.
    Resolved(DiscoveredPeer),
    Removed(String),
}

/// Owns one advertisement and browser. Drop withdraws and stops the daemon.
pub struct Discovery {
    daemon: ServiceDaemon,
    events: Receiver<ServiceEvent>,
    monitor: Receiver<mdns_sd::DaemonEvent>,
    fingerprint: [u8; 32],
    fullname: String,
}

impl Discovery {
    /// CCS/1 IO:device public key + actual bound QUIC port -> LAN discovery
    /// RULE:call after binding transport; port 0 is rejected. No pairing state changes.
    pub fn start(key: &PublicKey, port: u16) -> mdns_sd::Result<Self> {
        if port == 0 {
            return Err(mdns_sd::Error::Msg("QUIC port must be nonzero".into()));
        }
        let hex = key.fingerprint().to_hex();
        let instance = format!("wc-{}", &hex[..40]);
        let host = format!("{instance}.local.");
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            "",
            port,
            &[("fingerprint", hex.as_str())][..],
        )?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_owned();
        let daemon = ServiceDaemon::new()?;
        let result = (|| {
            let monitor = daemon.monitor()?;
            let events = daemon.browse(SERVICE_TYPE)?;
            daemon.register(service)?;
            Ok((events, monitor))
        })();
        match result {
            Ok((events, monitor)) => Ok(Self {
                daemon,
                events,
                monitor,
                fingerprint: *key.fingerprint().as_bytes(),
                fullname,
            }),
            Err(error) => {
                let _ = daemon.shutdown();
                Err(error)
            }
        }
    }

    /// Cancellation safe; ignores malformed records and our own advertisement.
    pub async fn next(&self) -> Result<DiscoveryEvent, mdns_sd::RecvError> {
        loop {
            match self.events.recv_async().await? {
                ServiceEvent::ServiceResolved(info) => {
                    if let Some(peer) = decode(&info) {
                        if peer.fingerprint != self.fingerprint {
                            return Ok(DiscoveryEvent::Resolved(peer));
                        }
                    }
                }
                ServiceEvent::ServiceRemoved(_, name) if name != self.fullname => {
                    return Ok(DiscoveryEvent::Removed(name));
                }
                _ => {}
            }
        }
    }

    /// Reports asynchronous registration/socket errors and name conflicts.
    pub fn monitor(&self) -> &Receiver<mdns_sd::DaemonEvent> {
        &self.monitor
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        // CCS/1 FLOW:goodbye acknowledged before shutdown | LIMIT:bounded waits
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
    }
}

fn decode(info: &ResolvedService) -> Option<DiscoveredPeer> {
    if !info.ty_domain.eq_ignore_ascii_case(SERVICE_TYPE) || info.port == 0 {
        return None;
    }
    let value = info.get_property_val_str("fingerprint")?;
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut fingerprint = [0; 32];
    for (i, byte) in fingerprint.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok()?;
    }
    let mut addresses: Vec<_> = info
        .addresses
        .iter()
        .filter_map(|ip| {
            if ip.to_ip_addr().is_unspecified() || ip.to_ip_addr().is_multicast() {
                return None;
            }
            match ip {
                ScopedIp::V4(ip) => Some(SocketAddr::new((*ip.addr()).into(), info.port)),
                ScopedIp::V6(ip) => {
                    Some(SocketAddrV6::new(*ip.addr(), info.port, 0, ip.scope_id().index).into())
                }
                _ => None,
            }
        })
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        return None;
    }
    Some(DiscoveredPeer {
        service_name: info.fullname.clone(),
        fingerprint,
        addresses,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn record(value: &str) -> ResolvedService {
        ServiceInfo::new(
            SERVICE_TYPE,
            "test",
            "test.local.",
            "192.168.1.2",
            47821,
            &[("fingerprint", value)][..],
        )
        .unwrap()
        .as_resolved_service()
    }

    #[test]
    fn full_fingerprint_and_actual_port() {
        let peer = decode(&record(&"aB".repeat(32))).unwrap();
        assert_eq!(peer.fingerprint, [0xab; 32]);
        assert_eq!(peer.addresses, ["192.168.1.2:47821".parse().unwrap()]);
    }

    #[test]
    fn rejects_malformed_and_unusable_advertisements() {
        for value in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "g".repeat(64),
            "é".repeat(32),
        ] {
            assert!(decode(&record(&value)).is_none());
        }
        let mut info = record(&"ab".repeat(32));
        info.port = 0;
        assert!(decode(&info).is_none());
        info.port = 47820;
        info.addresses.clear();
        assert!(decode(&info).is_none());
        info.addresses
            .insert(ScopedIp::from("0.0.0.0".parse::<IpAddr>().unwrap()));
        assert!(decode(&info).is_none());
        info.ty_domain = "_other._udp.local.".into();
        assert!(decode(&info).is_none());
    }
}
