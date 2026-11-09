use std::{sync::Arc, time::Duration};
use wind_core::network::NetworkChanges;

#[tokio::test]
async fn changes_are_retained_coalesced_and_broadcast_without_a_timer() {
    let changes = NetworkChanges::default();
    let mut first = changes.subscribe();
    let mut second = changes.subscribe();
    for _ in 0..100 {
        changes.notify();
    }
    first.changed().await.unwrap();
    second.changed().await.unwrap();
    assert_eq!(*first.borrow_and_update(), 100);
    assert_eq!(*second.borrow_and_update(), 100);
    assert!(!first.has_changed().unwrap());
    let pending = first.changed();
    changes.notify();
    tokio::time::timeout(Duration::from_millis(100), pending)
        .await
        .unwrap()
        .unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn windows_registers_receives_initial_callback_and_unregisters() {
    use wind_core::network::WindowsNetworkMonitor;
    let changes = Arc::new(NetworkChanges::default());
    let mut receiver = changes.subscribe();
    let monitor = WindowsNetworkMonitor::start(changes.clone()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), receiver.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(changes.generation() > 0);
    drop(monitor);
    let generation = changes.generation();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(changes.generation(), generation);
}

#[tokio::test]
#[ignore = "requires multicast-capable host"]
async fn event_restarts_discovery_immediately_with_the_current_endpoint_port() {
    use wind_core::{
        identity::DeviceIdentity,
        network::NetworkDiscovery,
        transport::{Endpoint, PinnedKeys},
    };
    let identity = DeviceIdentity::generate().unwrap();
    let endpoint = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &identity,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let changes = Arc::new(NetworkChanges::default());
    let mut discovery =
        NetworkDiscovery::start(identity.public_key(), endpoint.clone(), changes.clone()).unwrap();
    let mut state = discovery.subscribe();
    let observer_identity = DeviceIdentity::generate().unwrap();
    let observer =
        wind_core::discovery::Discovery::start(&observer_identity.public_key(), 47821).unwrap();
    let new_address = endpoint.rebind("127.0.0.1:0".parse().unwrap()).unwrap();
    changes.notify();
    let expected = changes.generation();
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            state.changed().await.unwrap();
            if state.borrow_and_update().generation >= expected {
                break;
            }
        }
    })
    .await
    .expect("no polling interval may delay processing");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let wind_core::discovery::DiscoveryEvent::Resolved(peer) =
                observer.next().await.unwrap()
            {
                if peer.fingerprint == *identity.fingerprint().as_bytes()
                    && peer
                        .addresses
                        .iter()
                        .all(|address| address.port() == new_address.port())
                {
                    break;
                }
            }
        }
    })
    .await
    .expect("a separate mDNS browser must resolve the new QUIC port");
    drop(observer);
    discovery.shutdown().await;
    endpoint.close();
    endpoint.wait_idle().await;
}
