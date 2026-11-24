use std::time::{Duration, Instant};
use wind_core::discovery::{Discovery, DiscoveryEvent};
use wind_core::identity::DeviceIdentity;

/// CCS/1 LIMIT:requires multicast-capable LAN; run explicitly on integration hosts.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LAN multicast (UDP 5353) and firewall permission"]
async fn two_devices_discover_each_other() {
    let a = DeviceIdentity::generate().unwrap();
    let b = DeviceIdentity::generate().unwrap();
    let first = Discovery::start(&a.public_key(), 47820).unwrap();
    let started = Instant::now();
    let second = Discovery::start(&b.public_key(), 47821).unwrap();
    async fn find(discovery: &Discovery, key: [u8; 32], port: u16) {
        loop {
            if let DiscoveryEvent::Resolved(peer) = discovery.next().await.unwrap() {
                if peer.fingerprint == key {
                    assert!(peer.addresses.iter().all(|a| a.port() == port));
                    break;
                }
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(
            find(&first, *b.fingerprint().as_bytes(), 47821),
            find(&second, *a.fingerprint().as_bytes(), 47820),
        );
    })
    .await
    .expect("both advertisements must resolve");
    println!("mutual mDNS discovery: {:?}", started.elapsed());
    // WN-108: the budget is probing plus the first announcement, not 1 s.
    // RFC 6762 §8.1: a random 0-250 ms start, then 3 probes 250 ms apart,
    // so up to ~1 s before the name is ours and announced (measured on
    // Windows: 834-1012 ms; 1 s failed now and then). 2 s holds that with
    // room for a loaded host; the 5 s timeout above stays the hard limit.
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn refuses_unbound_port() {
    let identity = DeviceIdentity::generate().unwrap();
    assert!(Discovery::start(&identity.public_key(), 0).is_err());
}

/// WN-82 (§12.4): while a SAS window is open, a browser on the LAN sees the
/// pairing port in TXT `pair`; it stays after a network change and is gone
/// once the window closes.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LAN multicast (UDP 5353) and firewall permission"]
async fn sas_window_advertises_the_pairing_port() {
    use std::sync::Arc;
    use wind_core::network::{DiscoveryOptions, NetworkChanges, NetworkDiscovery};
    use wind_core::pairing::{PairedDevicesFile, PairingConfig, PairingListener};
    use wind_core::transport::{Endpoint, PinnedKeys};

    let pc = DeviceIdentity::generate().unwrap();
    let endpoint = Endpoint::bind(
        "0.0.0.0:0".parse().unwrap(),
        &pc,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let changes = Arc::new(NetworkChanges::default());
    let mut discovery = NetworkDiscovery::start_with_options(
        pc.public_key(),
        endpoint.clone(),
        changes.clone(),
        DiscoveryOptions {
            mdns: true,
            broadcast: None,
            paired: None,
            announce: Default::default(),
        },
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let listener = PairingListener::bind(
        "0.0.0.0:0".parse().unwrap(),
        &pc,
        Some("PC"),
        Arc::new(PairedDevicesFile::in_dir(dir.path())),
        Arc::new(PinnedKeys::new()),
        PairingConfig::default(),
    )
    .unwrap();
    listener.advertise_with(discovery.pairing_advertiser());
    let pairing = listener.local_addr().unwrap().port();

    // No wait before browsing (WN-89): NetworkDiscovery no longer restarts
    // mDNS for the initial Windows network report, and none after a network
    // change either (WN-104: the restart re-advertises after its goodbye).
    let pc_fingerprint = *pc.fingerprint().as_bytes();
    async fn seen(browser: &Discovery, pc: [u8; 32], wanted: Option<u16>, step: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let DiscoveryEvent::Resolved(peer) = browser.next().await.unwrap() {
                    if peer.fingerprint == pc && peer.pairing_port == wanted {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{step}: the browser must see pair={wanted:?}"))
    }
    let phone = DeviceIdentity::generate().unwrap();
    let browser = Discovery::start(&phone.public_key(), 47821).unwrap();

    seen(&browser, pc_fingerprint, None, "before").await;
    listener.open_sas();
    seen(&browser, pc_fingerprint, Some(pairing), "open").await;

    // Unchanged records raise no new event; a browser started after the
    // network change must still learn the port.
    changes.notify();
    let tablet = DeviceIdentity::generate().unwrap();
    let late = Discovery::start(&tablet.public_key(), 47822).unwrap();
    seen(
        &late,
        pc_fingerprint,
        Some(pairing),
        "after a network change",
    )
    .await;

    // ~1 s after the restart advertised again: the TXT change waits for its
    // probe to settle (WN-104, PROBE_SETTLE) and still reaches both browsers.
    listener.close_window();
    seen(&browser, pc_fingerprint, None, "closed").await;
    seen(&late, pc_fingerprint, None, "closed, late browser").await;

    drop(late);
    drop(browser);
    discovery.shutdown().await;
    endpoint.close();
    endpoint.wait_idle().await;
}

/// WN-89: a browser started right after `NetworkDiscovery` sees the device
/// within 5 s, ten runs in a row (Windows reports the network once at
/// registration).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LAN multicast (UDP 5353) and firewall permission"]
async fn browser_started_right_after_network_discovery_sees_the_device() {
    use std::sync::Arc;
    use wind_core::network::{DiscoveryOptions, NetworkChanges, NetworkDiscovery};
    use wind_core::transport::{Endpoint, PinnedKeys};

    let mut failures = Vec::new();
    for run in 0..10 {
        let pc = DeviceIdentity::generate().unwrap();
        let endpoint = Endpoint::bind(
            "0.0.0.0:0".parse().unwrap(),
            &pc,
            Arc::new(PinnedKeys::new()),
        )
        .unwrap();
        let changes = Arc::new(NetworkChanges::default());
        let started = Instant::now();
        let mut discovery = NetworkDiscovery::start_with_options(
            pc.public_key(),
            endpoint.clone(),
            changes.clone(),
            DiscoveryOptions {
                mdns: true,
                broadcast: None,
                paired: None,
                announce: Default::default(),
            },
        )
        .unwrap();
        let phone = DeviceIdentity::generate().unwrap();
        let browser = Discovery::start(&phone.public_key(), 47821).unwrap();
        let pc_fingerprint = *pc.fingerprint().as_bytes();
        let mut removed = 0;
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match browser.next().await.unwrap() {
                    DiscoveryEvent::Resolved(peer) if peer.fingerprint == pc_fingerprint => break,
                    DiscoveryEvent::Removed(name)
                        if name.contains(&pc.fingerprint().to_hex()[..40]) =>
                    {
                        removed += 1
                    }
                    _ => {}
                }
            }
        })
        .await;
        println!(
            "run {run}: {} after {:?}, generation {}, removed {removed}",
            if found.is_ok() { "seen" } else { "NOT seen" },
            started.elapsed(),
            changes.generation(),
        );
        if found.is_err() {
            failures.push(run);
        }
        drop(browser);
        discovery.shutdown().await;
        endpoint.close();
        endpoint.wait_idle().await;
    }
    assert!(failures.is_empty(), "runs without the device: {failures:?}");
}

/// WN-104: a browser started right after a network change (`changes.notify()`)
/// sees the device within 5 s, ten runs in a row. Runs alternate between one
/// notification and a second one 300 or 800 ms later, while the first restart is
/// still under way (two monitors on one `NetworkChanges`, or a network settling
/// in steps); `WN104_SECOND_NOTIFY_MS` fixes the delay for every run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LAN multicast (UDP 5353) and firewall permission"]
async fn browser_started_right_after_a_network_change_sees_the_device() {
    use std::sync::Arc;
    use wind_core::network::{DiscoveryOptions, NetworkChanges, NetworkDiscovery};
    use wind_core::transport::{Endpoint, PinnedKeys};

    let fixed: Option<u64> = std::env::var("WN104_SECOND_NOTIFY_MS")
        .ok()
        .and_then(|value| value.parse().ok());
    let mut failures = Vec::new();
    for run in 0..10 {
        let second = fixed.or([None, Some(300), Some(800)][run % 3]);
        let pc = DeviceIdentity::generate().unwrap();
        let endpoint = Endpoint::bind(
            "0.0.0.0:0".parse().unwrap(),
            &pc,
            Arc::new(PinnedKeys::new()),
        )
        .unwrap();
        let changes = Arc::new(NetworkChanges::default());
        let mut discovery = NetworkDiscovery::start_with_options(
            pc.public_key(),
            endpoint.clone(),
            changes.clone(),
            DiscoveryOptions {
                mdns: true,
                broadcast: None,
                paired: None,
                announce: Default::default(),
            },
        )
        .unwrap();
        // The device has announced itself; then the network changes.
        tokio::time::sleep(Duration::from_secs(2)).await;
        changes.notify();
        let started = Instant::now();
        let phone = DeviceIdentity::generate().unwrap();
        let browser = Discovery::start(&phone.public_key(), 47821).unwrap();
        if let Some(ms) = second {
            let changes = changes.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms)).await;
                changes.notify();
            });
        }
        let pc_fingerprint = *pc.fingerprint().as_bytes();
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let DiscoveryEvent::Resolved(peer) = browser.next().await.unwrap() {
                    if peer.fingerprint == pc_fingerprint {
                        break;
                    }
                }
            }
        })
        .await;
        println!(
            "run {run}: {} after {:?}, generation {}, second notification {second:?} ms",
            if found.is_ok() { "seen" } else { "NOT seen" },
            started.elapsed(),
            changes.generation(),
        );
        if found.is_err() {
            failures.push(run);
        }
        drop(browser);
        discovery.shutdown().await;
        endpoint.close();
        endpoint.wait_idle().await;
    }
    assert!(failures.is_empty(), "runs without the device: {failures:?}");
}

/// WN-176 (§2.0.2): a device that vouches for no network isn't found by mDNS; a SAS window announces it everywhere
/// for as long as it is open, and closing it takes the record off the network again (a goodbye). A device that
/// trusts its LAN address is found at once.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LAN multicast (UDP 5353) and firewall permission"]
async fn quiet_on_untrusted_networks_announced_on_trusted_ones() {
    use std::sync::Arc;
    use wind_core::network::{
        local_nets, Announce, DiscoveryOptions, NetworkChanges, NetworkDiscovery,
    };
    use wind_core::pairing::{PairedDevicesFile, PairingConfig, PairingListener};
    use wind_core::transport::{Endpoint, PinnedKeys};

    let start = |id: &DeviceIdentity, announce: Announce| {
        let endpoint = Endpoint::bind(
            "0.0.0.0:0".parse().unwrap(),
            id,
            Arc::new(PinnedKeys::new()),
        )
        .unwrap();
        let discovery = NetworkDiscovery::start_with_options(
            id.public_key(),
            endpoint.clone(),
            Arc::new(NetworkChanges::default()),
            DiscoveryOptions {
                mdns: true,
                broadcast: None,
                paired: None,
                announce,
            },
        )
        .unwrap();
        (endpoint, discovery)
    };
    /// The first event about `who` within `limit`: `Some(pair port)` when resolved, `None` when removed.
    async fn next_about(
        browser: &Discovery,
        who: [u8; 32],
        name: &str,
        limit: Duration,
    ) -> Option<Option<Option<u16>>> {
        tokio::time::timeout(limit, async {
            loop {
                match browser.next().await.unwrap() {
                    DiscoveryEvent::Resolved(peer) if peer.fingerprint == who => {
                        return Some(peer.pairing_port)
                    }
                    DiscoveryEvent::Removed(gone) if gone.starts_with(name) => return None,
                    _ => {}
                }
            }
        })
        .await
        .ok()
    }

    let phone = DeviceIdentity::generate().unwrap();
    let browser = Discovery::start(&phone.public_key(), 47821).unwrap();

    let quiet = DeviceIdentity::generate().unwrap();
    let quiet_name = format!("wc-{}", &quiet.fingerprint().to_hex()[..40]);
    let (quiet_endpoint, mut quiet_discovery) =
        start(&quiet, Announce::Trusted(Arc::new(Vec::new)));
    let who = *quiet.fingerprint().as_bytes();
    assert_eq!(
        next_about(&browser, who, &quiet_name, Duration::from_secs(4)).await,
        None,
        "an untrusted network hears nothing from us"
    );

    let dir = tempfile::tempdir().unwrap();
    let listener = PairingListener::bind(
        "0.0.0.0:0".parse().unwrap(),
        &quiet,
        Some("PC"),
        Arc::new(PairedDevicesFile::in_dir(dir.path())),
        Arc::new(PinnedKeys::new()),
        PairingConfig::default(),
    )
    .unwrap();
    listener.advertise_with(quiet_discovery.pairing_advertiser());
    let port = listener.local_addr().unwrap().port();
    listener.open_sas();
    assert_eq!(
        next_about(&browser, who, &quiet_name, Duration::from_secs(5)).await,
        Some(Some(Some(port))),
        "a SAS window announces us"
    );
    listener.close_window();
    // Repeats of the open window's record may still arrive; the next state must be "gone", never a record without
    // `pair` (that would be our fingerprint still announced on an untrusted network).
    let gone = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match next_about(&browser, who, &quiet_name, Duration::from_secs(5)).await {
                Some(Some(Some(p))) if p == port => continue,
                other => return other,
            }
        }
    })
    .await
    .expect("closing the window says goodbye within 5 s");
    assert_eq!(gone, Some(None), "closing it says goodbye");

    let trusted = DeviceIdentity::generate().unwrap();
    let lan: Vec<std::net::IpAddr> = local_nets()
        .into_iter()
        .map(|n| n.addr)
        .filter(|a| !a.is_loopback())
        .collect();
    let (trusted_endpoint, mut trusted_discovery) =
        start(&trusted, Announce::Trusted(Arc::new(move || lan.clone())));
    let trusted_name = format!("wc-{}", &trusted.fingerprint().to_hex()[..40]);
    assert_eq!(
        next_about(
            &browser,
            *trusted.fingerprint().as_bytes(),
            &trusted_name,
            Duration::from_secs(5)
        )
        .await,
        Some(Some(None)),
        "a trusted network finds us"
    );

    drop(listener);
    drop(browser);
    quiet_discovery.shutdown().await;
    trusted_discovery.shutdown().await;
    quiet_endpoint.close();
    trusted_endpoint.close();
}
