use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};
use wind_core::{
    broadcast::{BroadcastConfig, BroadcastDiscovery, Packet, PACKET_SIZE},
    identity::DeviceIdentity,
    network::{DiscoveryOptions, NetworkChanges, NetworkDiscovery},
    transport::{Endpoint, PinnedKeys},
};

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn config(port: u16) -> BroadcastConfig {
    BroadcastConfig {
        listen: SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port),
        destinations: vec![SocketAddrV4::new(Ipv4Addr::new(127, 255, 255, 255), port)],
        interval: Duration::from_millis(50),
        expiry: Duration::from_millis(250),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_discover_without_any_multicast_and_expire_stopped_peers() {
    let a = DeviceIdentity::generate().unwrap();
    let b = DeviceIdentity::generate().unwrap();
    let pins_a = Arc::new(PinnedKeys::new());
    let pins_b = Arc::new(PinnedKeys::new());
    let first = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &a, pins_a.clone()).unwrap();
    let second = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &b, pins_b.clone()).unwrap();
    let port = free_port();
    let options = DiscoveryOptions {
        mdns: false,
        broadcast: Some(config(port)),
        paired: None,
        announce: Default::default(),
    };
    let changes = Arc::new(NetworkChanges::default());
    let mut one = NetworkDiscovery::start_with_options(
        a.public_key(),
        first.clone(),
        changes.clone(),
        options.clone(),
    )
    .unwrap();
    let mut two =
        NetworkDiscovery::start_with_options(b.public_key(), second.clone(), changes, options)
            .unwrap();
    let mut state_a = one.subscribe();
    let mut state_b = two.subscribe();
    async fn found(
        state: &mut tokio::sync::watch::Receiver<wind_core::network::DiscoverySnapshot>,
        fingerprint: [u8; 32],
        port: u16,
    ) {
        loop {
            state.changed().await.unwrap();
            if let Some(peer) = state
                .borrow_and_update()
                .peers
                .iter()
                .find(|peer| peer.fingerprint == fingerprint)
            {
                assert!(peer.addresses.iter().all(|address| address.port() == port));
                break;
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(
            found(
                &mut state_a,
                *b.fingerprint().as_bytes(),
                second.local_addr().unwrap().port()
            ),
            found(
                &mut state_b,
                *a.fingerprint().as_bytes(),
                first.local_addr().unwrap().port()
            )
        );
    })
    .await
    .expect("broadcast must discover both nodes with mDNS entirely disabled");
    assert_eq!(state_a.borrow().peers.len(), 1);
    let discovered = state_a.borrow().peers[0].addresses[0];
    assert!(
        first.connect(discovered, b.public_key()).await.is_err(),
        "discovery cannot pin a key"
    );
    pins_a.pin(b.public_key());
    pins_b.pin(a.public_key());
    let (client, server) = tokio::join!(first.connect(discovered, b.public_key()), second.accept());
    let client = client.unwrap();
    let server = server.unwrap().unwrap();
    assert_eq!(client.peer_key(), b.public_key());
    assert_eq!(server.peer_key(), a.public_key());
    client.close(wind_core::protocol::ErrorCode::GoingAway, b"");
    server.close(wind_core::protocol::ErrorCode::GoingAway, b"");
    two.shutdown().await;
    drop(two);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            state_a.changed().await.unwrap();
            if state_a.borrow_and_update().peers.is_empty() {
                break;
            }
        }
    })
    .await
    .expect("stopped broadcast peer must expire");
    one.shutdown().await;
    first.close();
    second.close();
    first.wait_idle().await;
    second.wait_idle().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn network_change_sends_fresh_port_without_waiting_for_broadcast_interval() {
    let a = DeviceIdentity::generate().unwrap();
    let b = DeviceIdentity::generate().unwrap();
    let first = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &a,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let second = Endpoint::bind(
        "127.0.0.2:0".parse().unwrap(),
        &b,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let port = free_port();
    let mut cfg_a = config(port);
    cfg_a.listen = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port);
    cfg_a.destinations = vec![SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), port)];
    cfg_a.interval = Duration::from_secs(30);
    cfg_a.expiry = Duration::from_secs(60);
    let mut cfg_b = cfg_a.clone();
    cfg_b.listen = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), port);
    cfg_b.destinations = vec![cfg_a.listen];
    let changes = Arc::new(NetworkChanges::default());
    let mut one = NetworkDiscovery::start_with_options(
        a.public_key(),
        first.clone(),
        changes.clone(),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(cfg_a),
            paired: None,
            announce: Default::default(),
        },
    )
    .unwrap();
    let mut two = NetworkDiscovery::start_with_options(
        b.public_key(),
        second.clone(),
        Arc::new(NetworkChanges::default()),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(cfg_b),
            paired: None,
            announce: Default::default(),
        },
    )
    .unwrap();
    let mut state = two.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.borrow_and_update().peers.is_empty() {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let fresh = first.rebind("127.0.0.1:0".parse().unwrap()).unwrap();
    changes.notify();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            state.changed().await.unwrap();
            if state.borrow_and_update().peers.iter().any(|peer| {
                peer.fingerprint == *a.fingerprint().as_bytes()
                    && peer
                        .addresses
                        .iter()
                        .any(|addr| addr.port() == fresh.port())
            }) {
                break;
            }
        }
    })
    .await
    .expect("network event must bypass the 30-second broadcast timer");
    one.shutdown().await;
    two.shutdown().await;
    first.close();
    second.close();
    first.wait_idle().await;
    second.wait_idle().await;
}

#[tokio::test]
async fn oversized_or_invalid_datagrams_do_not_disable_receiver() {
    let port = free_port();
    let mut cfg = config(port);
    cfg.listen = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let receiver = BroadcastDiscovery::bind([1; 32], cfg).unwrap();
    let sender = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), port))
        .await
        .unwrap();
    let destination = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    for bytes in [vec![0; PACKET_SIZE + 50], vec![0; PACKET_SIZE], vec![0; 3]] {
        sender.send_to(&bytes, destination).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), receiver.receive())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
    }
    let packet = Packet {
        query: true,
        port: 47820,
        fingerprint: [2; 32],
    };
    sender.send_to(&packet.encode(), destination).await.unwrap();
    let (received, source) = tokio::time::timeout(Duration::from_secs(1), receiver.receive())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(received, packet);
    assert_eq!(*source.ip(), Ipv4Addr::new(127, 0, 0, 2));
}

/// A query to a peer that is gone (nobody on its port) must not break the
/// sender's receiver: on Windows the ICMP "port unreachable" comes back as
/// WSAECONNRESET on the next receive (WN-85).
#[tokio::test]
async fn sending_to_a_gone_peer_does_not_disable_receiving() {
    // One discovery port, as on a real network; three loopback addresses:
    // A, B, and a peer that is gone (nobody bound there).
    let port = free_port();
    let at = |last: u8| SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, last), port);
    let cfg = |listen: SocketAddrV4, to: SocketAddrV4| BroadcastConfig {
        listen,
        destinations: vec![to],
        interval: Duration::from_secs(30),
        expiry: Duration::from_secs(60),
    };
    let a = BroadcastDiscovery::bind([1; 32], cfg(at(1), at(3))).unwrap();
    a.query(47820).await.unwrap();
    // The bad news arrives first; it must be skipped, not returned.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let b = BroadcastDiscovery::bind([2; 32], cfg(at(2), at(1))).unwrap();
    b.query(47821).await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(found) = a.receive().await.expect("receiver still works") {
                break found;
            }
        }
    })
    .await
    .expect("the later query arrives");
    assert_eq!(got.0.fingerprint, [2; 32]);
    assert_eq!(got.0.port, 47821);
}

/// WN-87: one host on the LAN floods 128 made-up fingerprints every
/// interval; the paired device on another IP still shows up, and the
/// broadcast cache never holds more than 128.
#[tokio::test(flavor = "multi_thread")]
async fn flood_of_fake_fingerprints_from_one_ip_does_not_hide_paired_device() {
    let a = DeviceIdentity::generate().unwrap();
    let b = DeviceIdentity::generate().unwrap();
    let pins_a = Arc::new(PinnedKeys::new());
    pins_a.pin(b.public_key());
    let first = Endpoint::bind("127.0.0.1:0".parse().unwrap(), &a, pins_a.clone()).unwrap();
    let second = Endpoint::bind(
        "127.0.0.2:0".parse().unwrap(),
        &b,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let port = free_port();
    let at = move |last: u8| SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, last), port);
    let cfg = |listen: SocketAddrV4, to: SocketAddrV4| BroadcastConfig {
        listen,
        destinations: vec![to],
        interval: Duration::from_millis(50),
        expiry: Duration::from_millis(250),
    };
    let mut one = NetworkDiscovery::start_with_options(
        a.public_key(),
        first.clone(),
        Arc::new(NetworkChanges::default()),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(cfg(at(1), at(2))),
            paired: Some(pins_a.clone()),
            announce: Default::default(),
        },
    )
    .unwrap();
    let mut state = one.subscribe();
    let spammer = tokio::net::UdpSocket::bind(at(3)).await.unwrap();
    let flood = tokio::spawn(async move {
        loop {
            for i in 0..128u32 {
                let mut fingerprint = [0xEE; 32];
                fingerprint[..4].copy_from_slice(&i.to_be_bytes());
                let packet = Packet {
                    query: true,
                    port: 40000,
                    fingerprint,
                };
                let _ = spammer.send_to(&packet.encode(), at(1)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    // Let the flood run for several expiry periods before the phone appears.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!state.borrow().peers.is_empty(), "the flood is seen");
    let mut two = NetworkDiscovery::start_with_options(
        b.public_key(),
        second.clone(),
        Arc::new(NetworkChanges::default()),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(cfg(at(2), at(1))),
            paired: None,
            announce: Default::default(),
        },
    )
    .unwrap();
    let wanted = *b.fingerprint().as_bytes();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            {
                let snapshot = state.borrow_and_update();
                assert!(snapshot.peers.len() <= 128, "the limit holds");
                if snapshot.peers.iter().any(|peer| peer.fingerprint == wanted) {
                    break;
                }
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("the paired device must show up despite the flood");
    // It stays while the flood goes on, past several expiry periods.
    tokio::time::sleep(Duration::from_millis(600)).await;
    {
        let snapshot = state.borrow();
        assert!(snapshot.peers.len() <= 128);
        assert!(snapshot.peers.iter().any(|peer| peer.fingerprint == wanted));
    }
    flood.abort();
    one.shutdown().await;
    two.shutdown().await;
    first.close();
    second.close();
    first.wait_idle().await;
    second.wait_idle().await;
}

/// WN-90: a phone in a session holds no MulticastLock, so it may hear no
/// broadcast at all. It still finds another PC: its own query (sending needs
/// no lock) gets the PC's unicast answer. Here the PC's own queries go where
/// nobody listens, so the answer is the only way in; after a network change
/// (cache cleared) the PC is back within 5 s at the real 2 s query period.
#[tokio::test(flavor = "multi_thread")]
async fn query_is_answered_by_unicast_so_the_asker_needs_no_broadcast() {
    let port = free_port();
    let at = |last: u8| SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, last), port);
    let real = |listen: SocketAddrV4, destination: SocketAddrV4| BroadcastConfig {
        listen,
        destinations: vec![destination],
        interval: Duration::from_secs(2),
        expiry: Duration::from_secs(8),
    };
    let options = |config| DiscoveryOptions {
        mdns: false,
        broadcast: Some(config),
        paired: None,
        announce: Default::default(),
    };
    let (phone, pc) = (
        DeviceIdentity::generate().unwrap(),
        DeviceIdentity::generate().unwrap(),
    );
    let phone_endpoint = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &phone,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let pc_endpoint = Endpoint::bind(
        "127.0.0.2:0".parse().unwrap(),
        &pc,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let mut pc_discovery = NetworkDiscovery::start_with_options(
        pc.public_key(),
        pc_endpoint.clone(),
        Arc::new(NetworkChanges::default()),
        options(real(at(2), at(3))),
    )
    .unwrap();
    let phone_changes = Arc::new(NetworkChanges::default());
    let mut phone_discovery = NetworkDiscovery::start_with_options(
        phone.public_key(),
        phone_endpoint.clone(),
        phone_changes.clone(),
        options(real(at(1), at(2))),
    )
    .unwrap();
    let mut state = phone_discovery.subscribe();
    let pc_fingerprint = *pc.fingerprint().as_bytes();
    let pc_address = std::net::SocketAddr::new(
        Ipv4Addr::new(127, 0, 0, 2).into(),
        pc_endpoint.local_addr().unwrap().port(),
    );
    async fn found(
        state: &mut tokio::sync::watch::Receiver<wind_core::network::DiscoverySnapshot>,
        fingerprint: [u8; 32],
        address: std::net::SocketAddr,
    ) -> Duration {
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                {
                    let now = state.borrow_and_update();
                    if let Some(peer) = now.peers.iter().find(|p| p.fingerprint == fingerprint) {
                        assert_eq!(peer.addresses, vec![address]);
                        break;
                    }
                }
                state.changed().await.unwrap();
            }
        })
        .await
        .expect("the PC's unicast answer must reach the phone within 5 s");
        started.elapsed()
    }
    let first = found(&mut state, pc_fingerprint, pc_address).await;
    println!("found after start: {first:?}");
    // A network change clears the cache (a new generation, no peers).
    phone_changes.notify();
    tokio::time::timeout(Duration::from_secs(1), async {
        while state.borrow_and_update().generation < 1 {
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("the network change is handled");
    let again = found(&mut state, pc_fingerprint, pc_address).await;
    println!("found after a network change: {again:?}");
    phone_discovery.shutdown().await;
    pc_discovery.shutdown().await;
    phone_endpoint.close();
    pc_endpoint.close();
}

/// WN-176 (PROTOCOL.md §2.0.2): a node told to announce only on trusted networks says nothing on one it doesn't
/// trust: other nodes never learn its fingerprint, though it still hears them. Once a paired device is seen there,
/// that network opens and it answers.
#[tokio::test(flavor = "multi_thread")]
async fn silent_on_an_untrusted_network_until_a_paired_device_is_seen() {
    use wind_core::network::Announce;
    let quiet = DeviceIdentity::generate().unwrap();
    let loud = DeviceIdentity::generate().unwrap();
    let quiet_pins = Arc::new(PinnedKeys::new());
    let quiet_endpoint =
        Endpoint::bind("127.0.0.1:0".parse().unwrap(), &quiet, quiet_pins.clone()).unwrap();
    let loud_endpoint = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &loud,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let port = free_port();
    let changes = Arc::new(NetworkChanges::default());
    let mut quiet_node = NetworkDiscovery::start_with_options(
        quiet.public_key(),
        quiet_endpoint.clone(),
        changes.clone(),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(config(port)),
            paired: Some(quiet_pins.clone()),
            // The platform vouches for no network here.
            announce: Announce::Trusted(Arc::new(Vec::new)),
        },
    )
    .unwrap();
    let mut loud_node = NetworkDiscovery::start_with_options(
        loud.public_key(),
        loud_endpoint.clone(),
        changes,
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(config(port)),
            paired: None,
            announce: Announce::Everywhere,
        },
    )
    .unwrap();
    let mut heard = quiet_node.subscribe();
    let loud_state = loud_node.subscribe();
    let sees = |state: &tokio::sync::watch::Receiver<wind_core::network::DiscoverySnapshot>,
                id: &DeviceIdentity| {
        state
            .borrow()
            .peers
            .iter()
            .any(|p| p.fingerprint == *id.fingerprint().as_bytes())
    };
    // The quiet node listens: it learns the loud one from its queries.
    tokio::time::timeout(Duration::from_secs(3), async {
        while !sees(&heard, &loud) {
            heard.changed().await.unwrap();
        }
    })
    .await
    .expect("listening goes on everywhere");
    // Many query rounds (50 ms each) later, the loud node still doesn't know the quiet one.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !sees(&loud_state, &quiet),
        "no query or answer with our fingerprint on an untrusted network"
    );
    // The loud node turns out to be paired: its next query opens the network, and the quiet node answers.
    quiet_pins.pin(loud.public_key());
    let mut loud_watch = loud_state.clone();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !sees(&loud_watch, &quiet) {
            loud_watch.changed().await.unwrap();
        }
    })
    .await
    .expect("a paired device seen here opens the network");
    quiet_node.shutdown().await;
    loud_node.shutdown().await;
}

/// WN-176: a network the platform vouches for (here loopback) is announced on from the start.
#[tokio::test(flavor = "multi_thread")]
async fn a_trusted_network_is_announced_on_at_once() {
    use wind_core::network::Announce;
    let trusted = DeviceIdentity::generate().unwrap();
    let other = DeviceIdentity::generate().unwrap();
    let trusted_endpoint = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &trusted,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let other_endpoint = Endpoint::bind(
        "127.0.0.1:0".parse().unwrap(),
        &other,
        Arc::new(PinnedKeys::new()),
    )
    .unwrap();
    let port = free_port();
    let changes = Arc::new(NetworkChanges::default());
    let mut announcing = NetworkDiscovery::start_with_options(
        trusted.public_key(),
        trusted_endpoint,
        changes.clone(),
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(config(port)),
            paired: None,
            announce: Announce::Trusted(Arc::new(|| vec!["127.0.0.1".parse().unwrap()])),
        },
    )
    .unwrap();
    let mut listening = NetworkDiscovery::start_with_options(
        other.public_key(),
        other_endpoint,
        changes,
        DiscoveryOptions {
            mdns: false,
            broadcast: Some(config(port)),
            paired: None,
            announce: Announce::Trusted(Arc::new(Vec::new)),
        },
    )
    .unwrap();
    let mut state = listening.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !state
            .borrow()
            .peers
            .iter()
            .any(|p| p.fingerprint == *trusted.fingerprint().as_bytes())
        {
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("a trusted network hears our queries");
    announcing.shutdown().await;
    listening.shutdown().await;
}
