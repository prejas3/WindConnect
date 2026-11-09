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
