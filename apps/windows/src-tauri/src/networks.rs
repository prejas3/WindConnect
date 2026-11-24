//! CCS/1 M:networks (WN-176, PROTOCOL.md §2.0.2) | PURPOSE:which networks this PC may announce itself on.
//! SEC:the PC says who it is (mDNS record, broadcast queries) only on a network Windows marks Private or Domain, or on
//! one where a paired phone connected or a pairing completed before (remembered by its Network List Manager id, in
//! `known-networks.json`). Elsewhere (a café's Public network) it only listens. Nothing here grants trust.
use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const FILE: &str = "known-networks.json";

/// One connected network as Windows sees it, with the local addresses of its adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connected {
    /// The Network List Manager's network id (`{…}`), stable for a network across visits.
    pub id: String,
    /// Windows marks it Private or Domain (not Public).
    pub private: bool,
    /// The adapter has a default gateway (a real LAN, not a host-only virtual switch).
    pub gateway: bool,
    /// The adapter's addresses and their prefix lengths.
    pub addrs: Vec<(IpAddr, u8)>,
}

impl Connected {
    fn holds(&self, remote: IpAddr) -> bool {
        self.addrs
            .iter()
            .any(|(addr, prefix)| same_subnet(*addr, remote, *prefix))
    }
}

fn same_subnet(a: IpAddr, b: IpAddr, prefix: u8) -> bool {
    let (a, b): (Vec<u8>, Vec<u8>) = match (a, b) {
        (IpAddr::V4(a), IpAddr::V4(b)) => (a.octets().to_vec(), b.octets().to_vec()),
        (IpAddr::V6(a), IpAddr::V6(b)) => (a.octets().to_vec(), b.octets().to_vec()),
        _ => return false,
    };
    let prefix = usize::from(prefix).min(a.len() * 8);
    let (bytes, bits) = (prefix / 8, prefix % 8);
    a[..bytes] == b[..bytes] && (bits == 0 || (a[bytes] ^ b[bytes]) & (0xffu8 << (8 - bits)) == 0)
}

/// The networks where a paired phone was met, kept across starts.
#[derive(Clone)]
pub struct Known {
    path: Option<PathBuf>,
    ids: Arc<Mutex<BTreeSet<String>>>,
    /// Lists the connected networks (Windows; a fixed list in tests).
    list: Arc<dyn Fn() -> Vec<Connected> + Send + Sync>,
}

impl Known {
    /// Reads `known-networks.json` in `dir`. WN-176 migration: when there is no file yet and `seed` (devices were
    /// paired before this version), the networks the PC is on now are remembered: most likely its home, where it
    /// announced before; a PC upgraded elsewhere stays as it was there, which this version otherwise ends.
    pub fn load(dir: Option<PathBuf>, seed: bool) -> Self {
        Self::load_with(dir, seed, Arc::new(connected))
    }

    pub fn load_with(
        dir: Option<PathBuf>,
        seed: bool,
        list: Arc<dyn Fn() -> Vec<Connected> + Send + Sync>,
    ) -> Self {
        let path = dir.map(|d| d.join(FILE));
        let stored = path
            .as_ref()
            .map(|p| std::fs::read(p).map(|b| serde_json::from_slice::<Vec<String>>(&b)));
        let missing = matches!(&stored, Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound);
        let ids = match stored {
            Some(Ok(Ok(ids))) => ids.into_iter().collect(),
            // A damaged file: nothing remembered (Private networks still announce); rewritten on the next meeting.
            _ => BTreeSet::new(),
        };
        let known = Known {
            path,
            ids: Arc::new(Mutex::new(ids)),
            list,
        };
        if missing && seed {
            known.remember_current();
        }
        known
    }

    /// The local addresses of connected networks this PC may announce on (for `Announce::Trusted`).
    pub fn trusted_addresses(&self) -> Vec<IpAddr> {
        let ids = self.ids.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (self.list)()
            .into_iter()
            .filter(|c| c.private || ids.contains(&c.id))
            .flat_map(|c| c.addrs.into_iter().map(|(addr, _)| addr))
            .collect()
    }

    /// A paired phone connected from `remote`: remembers the network it came over.
    pub fn remember_for(&self, remote: IpAddr) {
        let ids: Vec<String> = (self.list)()
            .into_iter()
            .filter(|c| c.holds(remote))
            .map(|c| c.id)
            .collect();
        self.add(ids);
    }

    /// A pairing completed: remembers the LAN networks the PC is on now (the phone was on one of them).
    pub fn remember_current(&self) {
        let ids: Vec<String> = (self.list)()
            .into_iter()
            .filter(|c| c.gateway)
            .map(|c| c.id)
            .collect();
        self.add(ids);
    }

    fn add(&self, new: Vec<String>) {
        let mut ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
        let before = ids.len();
        ids.extend(new);
        if ids.len() == before {
            return;
        }
        if let Some(path) = &self.path {
            let list: Vec<&String> = ids.iter().collect();
            if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, bytes).is_ok() {
                    let _ = std::fs::rename(&tmp, path);
                }
            }
        }
    }
}

/// The connected networks now: Network List Manager connections joined with their adapters' addresses. Empty when
/// Windows can't tell (then only remembered networks would count, and they need addresses too: nothing announces).
#[cfg(windows)]
pub fn connected() -> Vec<Connected> {
    let adapters = adapters();
    with_com(|| {
        use windows::Win32::Networking::NetworkListManager::{
            INetworkConnection, INetworkListManager, NetworkListManager,
            NLM_NETWORK_CATEGORY_PUBLIC,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
        let mut out = Vec::new();
        // SAFETY: COM is initialised on this thread by with_com; every interface is released when dropped.
        unsafe {
            let Ok(nlm) =
                CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL)
            else {
                return out;
            };
            let Ok(connections) = nlm.GetNetworkConnections() else {
                return out;
            };
            loop {
                let mut one: [Option<INetworkConnection>; 1] = [None];
                let mut fetched = 0u32;
                if connections.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
                    break;
                }
                let Some(connection) = one[0].take() else {
                    break;
                };
                let (Ok(adapter), Ok(network)) =
                    (connection.GetAdapterId(), connection.GetNetwork())
                else {
                    continue;
                };
                let (Ok(id), Ok(category)) = (network.GetNetworkId(), network.GetCategory()) else {
                    continue;
                };
                let adapter = format!("{{{adapter:?}}}").to_ascii_uppercase();
                let Some((addrs, gateway)) = adapters
                    .iter()
                    .find(|a| a.name.eq_ignore_ascii_case(&adapter))
                    .map(|a| (a.addrs.clone(), a.gateway))
                else {
                    continue;
                };
                out.push(Connected {
                    id: format!("{{{id:?}}}").to_ascii_uppercase(),
                    private: category != NLM_NETWORK_CATEGORY_PUBLIC,
                    gateway,
                    addrs,
                });
            }
        }
        out
    })
    .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn connected() -> Vec<Connected> {
    Vec::new()
}

/// Runs `f` with COM initialised on this thread (multithreaded; an apartment already there is kept).
#[cfg(windows)]
fn with_com<T>(f: impl FnOnce() -> T) -> Option<T> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    // SAFETY: plain flags; balanced below only when this call initialised COM (S_OK or S_FALSE).
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let ours = hr.is_ok();
    // RPC_E_CHANGED_MODE: this thread is an STA already; NLM works there too.
    let out = f();
    if ours {
        // SAFETY: balances the successful CoInitializeEx above on this thread.
        unsafe { CoUninitialize() };
    }
    Some(out)
}

/// One adapter: its name (`{GUID}`), unicast addresses with prefix lengths, and whether it has a default gateway.
#[cfg(windows)]
struct Adapter {
    name: String,
    addrs: Vec<(IpAddr, u8)>,
    gateway: bool,
}

#[cfg(windows)]
fn adapters() -> Vec<Adapter> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6,
    };
    let flags = GAA_FLAG_INCLUDE_GATEWAYS
        | GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..4 {
        buffer = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: the buffer is `size` bytes, 8-aligned as the struct needs; Windows writes at most that.
        let status = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                flags,
                std::ptr::null(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        match status {
            0 => break,
            111 => continue, // ERROR_BUFFER_OVERFLOW: `size` now says how much is needed.
            _ => return Vec::new(),
        }
    }
    let mut out = Vec::new();
    let mut adapter = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    // SAFETY: a linked list Windows built inside `buffer`, which outlives this loop; each pointer is null or valid.
    unsafe {
        while !adapter.is_null() {
            let a = &*adapter;
            let name = std::ffi::CStr::from_ptr(a.AdapterName.cast())
                .to_string_lossy()
                .into_owned();
            let mut addrs = Vec::new();
            let mut unicast = a.FirstUnicastAddress;
            while !unicast.is_null() {
                let u = &*unicast;
                let sockaddr = u.Address.lpSockaddr;
                if !sockaddr.is_null() {
                    let ip = match (*sockaddr).sa_family {
                        AF_INET => {
                            let v4 = &*sockaddr.cast::<SOCKADDR_IN>();
                            Some(IpAddr::from(v4.sin_addr.S_un.S_addr.to_ne_bytes()))
                        }
                        AF_INET6 => {
                            let v6 = &*sockaddr.cast::<SOCKADDR_IN6>();
                            Some(IpAddr::from(v6.sin6_addr.u.Byte))
                        }
                        _ => None,
                    };
                    if let Some(ip) = ip {
                        addrs.push((ip, u.OnLinkPrefixLength));
                    }
                }
                unicast = u.Next;
            }
            out.push(Adapter {
                name,
                addrs,
                gateway: !a.FirstGatewayAddress.is_null(),
            });
            adapter = a.Next;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(id: &str, private: bool, gateway: bool, addr: &str, prefix: u8) -> Connected {
        Connected {
            id: id.into(),
            private,
            gateway,
            addrs: vec![(addr.parse().unwrap(), prefix)],
        }
    }

    /// A laptop at a café (Public) with a WSL switch (Public, no gateway) and, later, at home (Private).
    fn fixed(list: Vec<Connected>) -> Arc<dyn Fn() -> Vec<Connected> + Send + Sync> {
        Arc::new(move || list.clone())
    }

    #[test]
    fn private_networks_announce_public_ones_only_once_met() {
        let dir = tempfile::tempdir().unwrap();
        let cafe = vec![
            net("{CAFE}", false, true, "10.0.3.20", 16),
            net("{WSL}", false, false, "172.20.0.1", 20),
            net("{HOME}", true, true, "192.168.1.20", 24),
        ];
        let known = Known::load_with(Some(dir.path().into()), false, fixed(cafe.clone()));
        assert_eq!(
            known.trusted_addresses(),
            ["192.168.1.20".parse::<IpAddr>().unwrap()]
        );
        // A paired phone connects over the café network: remembered, and kept across starts.
        known.remember_for("10.0.9.9".parse().unwrap());
        assert!(known
            .trusted_addresses()
            .contains(&"10.0.3.20".parse().unwrap()));
        let again = Known::load_with(Some(dir.path().into()), false, fixed(cafe));
        assert!(again
            .trusted_addresses()
            .contains(&"10.0.3.20".parse().unwrap()));
        assert!(!again
            .trusted_addresses()
            .contains(&"172.20.0.1".parse().unwrap()));
    }

    #[test]
    fn upgrade_seeds_current_lan_networks_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = vec![
            net("{HOME}", false, true, "192.168.1.20", 24),
            net("{WSL}", false, false, "172.20.0.1", 20),
        ];
        // Devices were paired before (seed): the Public home network the PC is on now is remembered, the switch isn't.
        let known = Known::load_with(Some(dir.path().into()), true, fixed(home.clone()));
        assert_eq!(
            known.trusted_addresses(),
            ["192.168.1.20".parse::<IpAddr>().unwrap()]
        );
        // The file exists now: a later start elsewhere seeds nothing.
        let cafe = vec![net("{CAFE}", false, true, "10.0.3.20", 16)];
        let later = Known::load_with(Some(dir.path().into()), true, fixed(cafe));
        assert!(later.trusted_addresses().is_empty());
        // No paired devices before: nothing seeded.
        let fresh = tempfile::tempdir().unwrap();
        let new_install = Known::load_with(Some(fresh.path().into()), false, fixed(home));
        assert!(new_install.trusted_addresses().is_empty());
    }

    #[test]
    fn subnets_by_prefix() {
        let a: IpAddr = "192.168.1.20".parse().unwrap();
        assert!(same_subnet(a, "192.168.1.200".parse().unwrap(), 24));
        assert!(!same_subnet(a, "192.168.2.20".parse().unwrap(), 24));
        assert!(same_subnet(a, "192.168.3.20".parse().unwrap(), 22));
        assert!(!same_subnet(a, "192.168.4.20".parse().unwrap(), 22));
        assert!(!same_subnet(a, "fe80::1".parse().unwrap(), 24));
        assert!(same_subnet(
            "fe80::aa".parse().unwrap(),
            "fe80::1".parse().unwrap(),
            64
        ));
    }

    /// The real list on this PC: every entry has an id and addresses (no assumption about which networks exist).
    #[cfg(windows)]
    #[test]
    fn windows_lists_connected_networks() {
        let list = connected();
        eprintln!("connected networks: {list:?}");
        for c in list {
            assert!(c.id.starts_with('{') && c.id.ends_with('}'), "{c:?}");
        }
    }
}
