//! CCS/1 M:discovery | IO:mDNS -> untrusted QUIC address hints
//! SEC:TXT fingerprints never grant trust; transport still verifies pinned keys.

use std::net::{SocketAddr, SocketAddrV6};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
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
    /// TXT `pair` (§12.4): the port of the device's pairing endpoint while
    /// its SAS window is open. An untrusted hint like the rest; §12.6
    /// authenticates.
    pub pairing_port: Option<u16>,
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
    instance: String,
    hex: String,
    port: u16,
    pairing: Mutex<Option<u16>>,
    advertised: AtomicBool,
}

impl Discovery {
    /// CCS/1 IO:device public key + actual bound QUIC port -> LAN discovery
    /// RULE:call after binding transport; port 0 is rejected. No pairing state changes.
    pub fn start(key: &PublicKey, port: u16) -> mdns_sd::Result<Self> {
        Self::start_with_pairing(key, port, None)
    }

    /// [`start`](Self::start), advertising `pairing` as TXT `pair` from the
    /// first announcement (see [`set_pairing_port`](Self::set_pairing_port)).
    pub fn start_with_pairing(
        key: &PublicKey,
        port: u16,
        pairing: Option<u16>,
    ) -> mdns_sd::Result<Self> {
        Self::start_inner(key, port, pairing, true)
    }

    /// Browses at once but advertises only on [`advertise`](Self::advertise).
    /// CCS/1 RULE:a receiver keeps our goodbye for 1 s (RFC 6762 §10.1). mdns-sd
    /// 0.21.7 caches a goodbye for an instance it never saw and treats an
    /// announcement inside that second as a TTL refresh: no ServiceFound or
    /// ServiceResolved, and the record then suppresses answers as a known
    /// answer. A browser started just before a restart missed the device for
    /// over 5 s (WN-104). The caller advertises once that second is over.
    pub fn start_unadvertised(
        key: &PublicKey,
        port: u16,
        pairing: Option<u16>,
    ) -> mdns_sd::Result<Self> {
        Self::start_inner(key, port, pairing, false)
    }

    fn start_inner(
        key: &PublicKey,
        port: u16,
        pairing: Option<u16>,
        advertise: bool,
    ) -> mdns_sd::Result<Self> {
        if port == 0 {
            return Err(mdns_sd::Error::Msg("QUIC port must be nonzero".into()));
        }
        let hex = key.fingerprint().to_hex();
        let instance = format!("wc-{}", &hex[..40]);
        let service = service_info(&instance, &hex, port, pairing)?;
        let fullname = service.get_fullname().to_owned();
        let daemon = ServiceDaemon::new()?;
        let result = (|| {
            let monitor = daemon.monitor()?;
            let events = daemon.browse(SERVICE_TYPE)?;
            if advertise {
                daemon.register(service)?;
            }
            Ok((events, monitor))
        })();
        match result {
            Ok((events, monitor)) => Ok(Self {
                daemon,
                events,
                monitor,
                fingerprint: *key.fingerprint().as_bytes(),
                fullname,
                instance,
                hex,
                port,
                pairing: Mutex::new(pairing),
                advertised: AtomicBool::new(advertise),
            }),
            Err(error) => {
                let _ = daemon.shutdown();
                Err(error)
            }
        }
    }

    /// CCS/1 IO:open SAS window port | None -> TXT `pair` re-announced (§12.4)
    /// RULE:port 0 is refused; the receiver treats the value as an untrusted hint.
    pub fn set_pairing_port(&self, pairing: Option<u16>) -> mdns_sd::Result<()> {
        if pairing == Some(0) {
            return Err(mdns_sd::Error::Msg("pairing port must be nonzero".into()));
        }
        let mut current = self.pairing.lock().unwrap();
        *current = pairing;
        if !self.advertised.load(Ordering::SeqCst) {
            return Ok(()); // advertise() uses it
        }
        // Registering the same name again replaces its records (TXT is a
        // unique record set, so peers drop the old value).
        self.daemon
            .register(service_info(&self.instance, &self.hex, self.port, pairing)?)
    }

    /// Starts advertising (see [`start_unadvertised`](Self::start_unadvertised)); later calls do nothing.
    pub fn advertise(&self) -> mdns_sd::Result<()> {
        let pairing = self.pairing.lock().unwrap();
        if self.advertised.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = service_info(&self.instance, &self.hex, self.port, *pairing)
            .and_then(|service| self.daemon.register(service));
        if result.is_err() {
            self.advertised.store(false, Ordering::SeqCst);
        }
        result
    }

    /// Whether this daemon advertises, and so sends a goodbye when dropped.
    pub fn is_advertised(&self) -> bool {
        self.advertised.load(Ordering::SeqCst)
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
        // RULE:nothing advertised, no goodbye (WN-104).
        if self.is_advertised() {
            if let Ok(done) = self.daemon.unregister(&self.fullname) {
                let _ = done.recv_timeout(Duration::from_secs(1));
            }
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
    }
}

fn service_info(
    instance: &str,
    hex: &str,
    port: u16,
    pairing: Option<u16>,
) -> mdns_sd::Result<ServiceInfo> {
    let pairing = pairing
        .filter(|port| *port != 0)
        .map(|port| port.to_string());
    let mut txt = vec![("fingerprint", hex)];
    if let Some(pairing) = &pairing {
        txt.push(("pair", pairing.as_str()));
    }
    Ok(ServiceInfo::new(
        SERVICE_TYPE,
        instance,
        &format!("{instance}.local."),
        "",
        port,
        &txt[..],
    )?
    .enable_addr_auto())
}

/// TXT `pair`: 1–5 decimal digits, 1–65535; anything else is no port.
fn pairing_port(info: &ResolvedService) -> Option<u16> {
    let value = info.get_property_val_str("pair")?;
    if value.is_empty() || value.len() > 5 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|port| *port != 0)
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
        pairing_port: pairing_port(info),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn record(value: &str) -> ResolvedService {
        record_with(&[("fingerprint", value)])
    }

    fn record_with(txt: &[(&str, &str)]) -> ResolvedService {
        ServiceInfo::new(
            SERVICE_TYPE,
            "test",
            "test.local.",
            "192.168.1.2",
            47821,
            txt,
        )
        .unwrap()
        .as_resolved_service()
    }

    #[test]
    fn full_fingerprint_and_actual_port() {
        let peer = decode(&record(&"aB".repeat(32))).unwrap();
        assert_eq!(peer.fingerprint, [0xab; 32]);
        assert_eq!(peer.addresses, ["192.168.1.2:47821".parse().unwrap()]);
        assert_eq!(peer.pairing_port, None);
    }

    #[test]
    fn pairing_port_is_read_and_a_bad_one_skipped() {
        let fingerprint = "ab".repeat(32);
        let pair = |value: &str| {
            decode(&record_with(&[
                ("fingerprint", fingerprint.as_str()),
                ("pair", value),
            ]))
            .expect("a bad pair value leaves the advertisement usable")
            .pairing_port
        };
        assert_eq!(pair("47830"), Some(47830));
        assert_eq!(pair("1"), Some(1));
        assert_eq!(pair("65535"), Some(65535));
        for bad in [
            "0", "00000", "", "abc", "+5", "-1", " 5", "65536", "123456", "4783o",
        ] {
            assert_eq!(pair(bad), None, "pair={bad:?}");
        }
    }

    #[test]
    fn advertisement_carries_pair_only_while_set() {
        let hex = "ab".repeat(32);
        let without = service_info("wc-test", &hex, 47820, None).unwrap();
        assert!(without.get_property("pair").is_none());
        let with = service_info("wc-test", &hex, 47820, Some(47830)).unwrap();
        assert_eq!(with.get_property_val_str("pair"), Some("47830"));
        let zero = service_info("wc-test", &hex, 47820, Some(0)).unwrap();
        assert!(zero.get_property("pair").is_none());
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
