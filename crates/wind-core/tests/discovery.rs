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
    assert!(started.elapsed() < Duration::from_secs(1));
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

    // CCS/1 LIMIT:Windows reports the network once at registration; let that mDNS restart settle.
    let settle = || tokio::time::sleep(Duration::from_secs(3));
    settle().await;
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
    settle().await;
    let tablet = DeviceIdentity::generate().unwrap();
    let late = Discovery::start(&tablet.public_key(), 47822).unwrap();
    seen(
        &late,
        pc_fingerprint,
        Some(pairing),
        "after a network change",
    )
    .await;

    listener.close_window();
    seen(&browser, pc_fingerprint, None, "closed").await;
    seen(&late, pc_fingerprint, None, "closed, late browser").await;

    drop(late);
    drop(browser);
    discovery.shutdown().await;
    endpoint.close();
    endpoint.wait_idle().await;
}
