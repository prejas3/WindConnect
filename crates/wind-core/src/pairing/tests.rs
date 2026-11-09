//! Pairing tests that need a scripted *server* (the client's order of
//! messages, §12.6). Attacks from the client side are in tests/pairing.rs.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use super::*;
use crate::transport::pairing_server_config;

const LOCALHOST: SocketAddr = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

struct NoStore;

impl PairedStore for NoStore {
    fn add(&self, _device: &PairedDevice) -> io::Result<()> {
        panic!("nothing may be recorded")
    }
}

/// A raw pairing server and the address to reach it.
fn scripted_server(identity: &DeviceIdentity) -> (quinn::Endpoint, SocketAddr) {
    let endpoint =
        quinn::Endpoint::server(pairing_server_config(identity).unwrap(), LOCALHOST).unwrap();
    let addr = endpoint.local_addr().unwrap();
    (endpoint, addr)
}

#[tokio::test]
async fn client_reveals_only_after_the_nonce_and_aborts_on_a_second_one() {
    let server_id = DeviceIdentity::generate().unwrap();
    let client_id = DeviceIdentity::generate().unwrap();
    let (endpoint, addr) = scripted_server(&server_id);
    let server_key = server_id.public_key();
    let config = PairingConfig {
        decision_timeout: Duration::from_secs(10),
        ..PairingConfig::default()
    };

    let script = tokio::spawn(async move {
        let conn = endpoint.accept().await.unwrap().await.unwrap();
        let client = handshake_peer_key(&conn).unwrap();
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();
        let hello = read_message(&mut recv).await.unwrap();
        let PairMessage::Hello {
            mode: Mode::Sas,
            proof,
            ..
        } = &hello
        else {
            panic!("expected a SAS hello, got {hello:?}");
        };
        let commitment = *proof;
        // (c) Nothing more comes before our nonce: the client is bound to
        // N_c by its commitment and must not reveal it yet.
        let mut probe = [0u8; 1];
        assert!(
            timeout(Duration::from_millis(400), recv.read(&mut probe))
                .await
                .is_err(),
            "the client sent something before the server nonce"
        );
        write_message(
            &mut send,
            &PairMessage::Nonce {
                nonce: [5; 32],
                name: Some("scripted".into()),
            },
        )
        .await
        .unwrap();
        let reveal = read_message(&mut recv).await.unwrap();
        let PairMessage::Reveal { nonce } = &reveal else {
            panic!("expected the reveal, got {reveal:?}");
        };
        assert_eq!(
            crypto::commit(&client, &server_key, nonce),
            commitment,
            "the reveal opens the commitment"
        );
        // (d) A second nonce after the reveal (an attacker trying to steer
        // the code once it knows N_c): the client must give up.
        write_message(
            &mut send,
            &PairMessage::Nonce {
                nonce: [6; 32],
                name: None,
            },
        )
        .await
        .unwrap();
        conn.closed().await
    });

    let pins = PinnedKeys::new();
    let (prompt_tx, prompt_rx) = std::sync::mpsc::channel();
    let result = pair_with_sas(
        &client_id,
        addr,
        Some("client"),
        &NoStore,
        &pins,
        &config,
        move |p| {
            // Keep the prompt unanswered: the client waits on the server.
            prompt_tx.send(p).unwrap();
        },
    )
    .await;
    assert!(matches!(result, Err(PairError::Protocol(_))), "{result:?}");
    let prompt = prompt_rx.recv().unwrap();
    assert_eq!(prompt.peer_name(), Some("scripted"));
    let closed = timeout(Duration::from_secs(5), script)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(&closed, quinn::ConnectionError::ApplicationClosed(c)
            if c.error_code.into_inner() == ErrorCode::ProtocolViolation.code()),
        "{closed:?}"
    );
    assert!(!crate::transport::PeerTrust::is_trusted(&pins, &server_key));
}
