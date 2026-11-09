//! Device identity (WC-07): the key survives a restart, the fingerprint is
//! stable and readable, and the certificate works in TLS 1.3.

use std::fs;
use std::io::{Read, Write};
use std::sync::Arc;

use wind_core::identity::{
    DeviceIdentity, FileKeyStore, IdentityError, KeyStore, PublicKey, CERT_SERVER_NAME,
};

/// RFC 8032 §7.1, test 1: a fixed key, so the fingerprint is a known answer.
const RFC8032_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const RFC8032_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn hex(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

/// PKCS#8 v1 PEM of an Ed25519 seed (RFC 8410 §7).
fn pem_of_seed(seed: &[u8; 32]) -> String {
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend_from_slice(seed);
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::new();
    for chunk in der.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                b64.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    format!("-----BEGIN PRIVATE KEY-----\n{b64}\n-----END PRIVATE KEY-----\n")
}

#[test]
fn same_key_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("identity.pem");

    let first = DeviceIdentity::load_or_create(&FileKeyStore::new(&path)).unwrap();
    assert!(path.exists());
    let stored = fs::read_to_string(&path).unwrap();
    assert!(stored.starts_with("-----BEGIN PRIVATE KEY-----"));

    // A "restart": a new store over the same file, nothing kept in memory.
    let (key, fp, cert) = (
        first.public_key(),
        first.fingerprint(),
        first.certificate_der(),
    );
    drop(first);
    let second = DeviceIdentity::load_or_create(&FileKeyStore::new(&path)).unwrap();
    assert_eq!(second.public_key(), key);
    assert_eq!(second.fingerprint(), fp);
    assert_eq!(second.fingerprint().to_string(), fp.to_string());
    // The certificate is the same too, byte for byte.
    assert_eq!(second.certificate_der(), cert);
    // Loading never rewrites the file.
    assert_eq!(fs::read_to_string(&path).unwrap(), stored);
}

#[test]
fn different_devices_have_different_keys() {
    let a = DeviceIdentity::generate().unwrap();
    let b = DeviceIdentity::generate().unwrap();
    assert_ne!(a.public_key(), b.public_key());
    assert_ne!(a.fingerprint(), b.fingerprint());
}

#[cfg(unix)]
#[test]
fn key_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.pem");
    DeviceIdentity::load_or_create(&FileKeyStore::new(&path)).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn corrupt_key_is_an_error_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.pem");
    fs::write(&path, "not a key").unwrap();
    let err = DeviceIdentity::load_or_create(&FileKeyStore::new(&path)).unwrap_err();
    assert!(matches!(err, IdentityError::Corrupt(_)), "{err:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "not a key");
}

#[test]
fn store_never_replaces_a_stored_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.pem");
    let store = FileKeyStore::new(&path);
    let winner = DeviceIdentity::generate().unwrap();
    assert!(store.store_new(&winner.to_pkcs8_pem().unwrap()).unwrap());
    let loser = DeviceIdentity::generate().unwrap();
    assert!(!store.store_new(&loser.to_pkcs8_pem().unwrap()).unwrap());
    let loaded = DeviceIdentity::load_or_create(&store).unwrap();
    assert_eq!(loaded.public_key(), winner.public_key());
    // No temporary files left behind.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// First start under contention: many threads of one process call
/// `load_or_create` on the same fresh path at once. Every call succeeds and
/// all end up with the one stored key; stale temporary files from an
/// earlier crash are left alone and block nothing.
#[test]
fn concurrent_first_start_converges_on_one_key() {
    use std::sync::Barrier;
    const THREADS: usize = 16;
    const ROUNDS: usize = 12;

    for round in 0..ROUNDS {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("identity-{round}.pem"));
        let stale = dir
            .path()
            .join(format!("identity-{round}.pem.{}.tmp", std::process::id()));
        fs::write(&stale, "left by a crashed write").unwrap();

        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let (barrier, path) = (barrier.clone(), path.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    DeviceIdentity::load_or_create(&FileKeyStore::new(path))
                        .map(|id| id.public_key())
                })
            })
            .collect();
        let keys: Vec<PublicKey> = handles
            .into_iter()
            .map(|h| h.join().unwrap().expect("every first start succeeds"))
            .collect();

        let stored = DeviceIdentity::load_or_create(&FileKeyStore::new(&path))
            .unwrap()
            .public_key();
        assert!(keys.iter().all(|k| *k == stored), "round {round}");
        assert_eq!(
            fs::read_to_string(&stale).unwrap(),
            "left by a crashed write"
        );
        // Only the key and the stale file: no temporary files of ours left.
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            2,
            "round {round}"
        );
    }
}

#[test]
fn fingerprint_known_answer() {
    let id = DeviceIdentity::from_pkcs8_pem(&pem_of_seed(&hex(RFC8032_SECRET))).unwrap();
    assert_eq!(id.public_key().as_bytes(), &hex(RFC8032_PUBLIC));
    let fp = id.fingerprint();
    // SHA-256 of the SPKI DER; computed independently.
    assert_eq!(
        fp.to_hex(),
        "06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9"
    );
    assert_eq!(fp.to_string(), "0VHZ V3YT 56XP 1ATS ANYY C7PV 1BPD P8RH");
    assert_eq!(fp.to_compact(), "0VHZV3YT56XP1ATSANYYC7PV1BPDP8RH");
    // The same key from raw bytes gives the same fingerprint.
    let raw = PublicKey::from_bytes(&hex(RFC8032_PUBLIC)).unwrap();
    assert_eq!(raw.fingerprint(), fp);
}

#[test]
fn fingerprint_is_readable() {
    for _ in 0..32 {
        let s = DeviceIdentity::generate()
            .unwrap()
            .fingerprint()
            .to_string();
        let groups: Vec<&str> = s.split(' ').collect();
        assert_eq!(groups.len(), 8, "{s}");
        for g in groups {
            assert_eq!(g.len(), 4, "{s}");
            // Crockford base32: digits and capitals, never I, L, O or U.
            assert!(
                g.chars()
                    .all(|c| c.is_ascii_digit() || (c.is_ascii_uppercase() && !"ILOU".contains(c))),
                "{s}"
            );
        }
    }
}

#[test]
fn certificate_carries_the_device_key() {
    let id = DeviceIdentity::generate().unwrap();
    let cert = id.certificate_der();
    let spki = id.public_key().spki_der();
    assert!(
        cert.windows(spki.len()).any(|w| w == spki.as_slice()),
        "certificate does not contain the device's SubjectPublicKeyInfo"
    );
}

/// Both devices present their certificates in a TLS 1.3 handshake (mutual
/// TLS), in memory. Each side trusts exactly the other's certificate here;
/// pinning by key is WC-10's verifier.
#[test]
fn certificates_complete_a_tls13_mutual_handshake() {
    use rustls::pki_types::ServerName;
    use rustls::server::WebPkiClientVerifier;
    use rustls::{ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection};

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_id = DeviceIdentity::generate().unwrap();
    let client_id = DeviceIdentity::generate().unwrap();

    let mut client_roots = RootCertStore::empty();
    client_roots.add(server_id.certificate_der()).unwrap();
    let mut server_roots = RootCertStore::empty();
    server_roots.add(client_id.certificate_der()).unwrap();

    let client_verifier =
        WebPkiClientVerifier::builder_with_provider(Arc::new(server_roots), provider.clone())
            .build()
            .unwrap();
    let server_config = ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(
            vec![server_id.certificate_der()],
            server_id.private_key_der().unwrap(),
        )
        .unwrap();
    let client_config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(client_roots)
        .with_client_auth_cert(
            vec![client_id.certificate_der()],
            client_id.private_key_der().unwrap(),
        )
        .unwrap();

    let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
    let mut client = ClientConnection::new(
        Arc::new(client_config),
        ServerName::try_from(CERT_SERVER_NAME).unwrap(),
    )
    .unwrap();

    for _ in 0..10 {
        if !client.is_handshaking() && !server.is_handshaking() {
            break;
        }
        let mut buf = Vec::new();
        client.write_tls(&mut buf).unwrap();
        server.read_tls(&mut buf.as_slice()).unwrap();
        server.process_new_packets().unwrap();
        let mut buf = Vec::new();
        server.write_tls(&mut buf).unwrap();
        client.read_tls(&mut buf.as_slice()).unwrap();
        client.process_new_packets().unwrap();
    }
    assert!(!client.is_handshaking() && !server.is_handshaking());
    assert_eq!(
        client.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    assert_eq!(
        client.peer_certificates().unwrap()[0],
        server_id.certificate_der()
    );
    assert_eq!(
        server.peer_certificates().unwrap()[0],
        client_id.certificate_der()
    );

    // And data flows.
    client.writer().write_all(b"hello").unwrap();
    let mut buf = Vec::new();
    client.write_tls(&mut buf).unwrap();
    server.read_tls(&mut buf.as_slice()).unwrap();
    server.process_new_packets().unwrap();
    let mut got = [0u8; 5];
    server.reader().read_exact(&mut got).unwrap();
    assert_eq!(&got, b"hello");
}
