//! The pairing proofs (PROTOCOL.md §12.5, §12.6): MACs over the one-time
//! secret, the SAS commitment and the 6-digit code. Every value binds both
//! device keys (from the TLS handshake) and the TLS session itself through
//! its exporter, so nothing carries over to another connection.
//!
//! HMAC-SHA256 and its constant-time check are `ring`'s; SHA-256 is
//! `sha2`'s.

use ring::hmac;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::wire::{NONCE_LEN, SECRET_LEN};
use crate::identity::PublicKey;

/// TLS exporter label (RFC 8446 §7.5) of a pairing connection.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-windconnect-pair";
/// Bytes taken from the exporter.
pub const EXPORTER_LEN: usize = 32;

const CLIENT_MAC: &[u8] = b"wc-pair-v1 client";
const SERVER_MAC: &[u8] = b"wc-pair-v1 server";
const COMMIT: &[u8] = b"wc-pair-v1 commit";
const SAS: &[u8] = b"wc-pair-v1 sas";

/// The exporter value `E` of `conn` (both sides get the same 32 bytes).
pub fn exporter(conn: &quinn::Connection) -> Option<Zeroizing<[u8; EXPORTER_LEN]>> {
    let mut out = Zeroizing::new([0u8; EXPORTER_LEN]);
    conn.export_keying_material(&mut out[..], EXPORTER_LABEL, b"")
        .ok()?;
    Some(out)
}

/// `label ‖ K_s ‖ K_c ‖ E`: every part has a fixed length.
fn mac_input(
    label: &[u8],
    server: &PublicKey,
    client: &PublicKey,
    exporter: &[u8; EXPORTER_LEN],
) -> Zeroizing<Vec<u8>> {
    let mut m = Zeroizing::new(Vec::with_capacity(label.len() + 96));
    m.extend_from_slice(label);
    m.extend_from_slice(server.as_bytes());
    m.extend_from_slice(client.as_bytes());
    m.extend_from_slice(exporter);
    m
}

fn tag(secret: &[u8; SECRET_LEN], input: &[u8]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let mut out = [0u8; 32];
    out.copy_from_slice(hmac::sign(&key, input).as_ref());
    out
}

fn verify(secret: &[u8; SECRET_LEN], input: &[u8], mac: &[u8; 32]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    hmac::verify(&key, input, mac).is_ok()
}

/// `mac_c = HMAC-SHA256(s, "wc-pair-v1 client" ‖ K_s ‖ K_c ‖ E)`.
pub fn client_mac(
    secret: &[u8; SECRET_LEN],
    server: &PublicKey,
    client: &PublicKey,
    exporter: &[u8; EXPORTER_LEN],
) -> [u8; 32] {
    tag(secret, &mac_input(CLIENT_MAC, server, client, exporter))
}

/// Checks `mac_c` in constant time.
pub fn verify_client_mac(
    secret: &[u8; SECRET_LEN],
    server: &PublicKey,
    client: &PublicKey,
    exporter: &[u8; EXPORTER_LEN],
    mac: &[u8; 32],
) -> bool {
    verify(
        secret,
        &mac_input(CLIENT_MAC, server, client, exporter),
        mac,
    )
}

/// `mac_s = HMAC-SHA256(s, "wc-pair-v1 server" ‖ K_s ‖ K_c ‖ E)`.
pub fn server_mac(
    secret: &[u8; SECRET_LEN],
    server: &PublicKey,
    client: &PublicKey,
    exporter: &[u8; EXPORTER_LEN],
) -> [u8; 32] {
    tag(secret, &mac_input(SERVER_MAC, server, client, exporter))
}

/// Checks `mac_s` in constant time.
pub fn verify_server_mac(
    secret: &[u8; SECRET_LEN],
    server: &PublicKey,
    client: &PublicKey,
    exporter: &[u8; EXPORTER_LEN],
    mac: &[u8; 32],
) -> bool {
    verify(
        secret,
        &mac_input(SERVER_MAC, server, client, exporter),
        mac,
    )
}

/// `C = SHA-256("wc-pair-v1 commit" ‖ K_c ‖ K_s ‖ N_c)`.
pub fn commit(client: &PublicKey, server: &PublicKey, nonce_c: &[u8; NONCE_LEN]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(COMMIT);
    h.update(client.as_bytes());
    h.update(server.as_bytes());
    h.update(nonce_c);
    h.finalize().into()
}

/// The 6-digit code: `be_u32(SHA-256("wc-pair-v1 sas" ‖ E ‖ K_c ‖ K_s ‖
/// N_c ‖ N_s)[0..4]) mod 1 000 000`.
pub fn sas(
    exporter: &[u8; EXPORTER_LEN],
    client: &PublicKey,
    server: &PublicKey,
    nonce_c: &[u8; NONCE_LEN],
    nonce_s: &[u8; NONCE_LEN],
) -> u32 {
    let mut h = Sha256::new();
    h.update(SAS);
    h.update(exporter);
    h.update(client.as_bytes());
    h.update(server.as_bytes());
    h.update(nonce_c);
    h.update(nonce_s);
    let d = h.finalize();
    u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000
}

/// A code as people read it: `042 917`.
pub fn format_sas(code: u32) -> String {
    let code = code % 1_000_000;
    format!("{:03} {:03}", code / 1000, code % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> PublicKey {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        PublicKey::from_bytes(&sk.verifying_key().to_bytes()).unwrap()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn macs_bind_secret_keys_and_session() {
        let (s, ks, kc, e) = ([1u8; 16], key(1), key(2), [3u8; 32]);
        let m = client_mac(&s, &ks, &kc, &e);
        assert!(verify_client_mac(&s, &ks, &kc, &e, &m));
        assert!(
            !verify_client_mac(&[2; 16], &ks, &kc, &e, &m),
            "other secret"
        );
        assert!(
            !verify_client_mac(&s, &ks, &key(9), &e, &m),
            "other client key"
        );
        assert!(
            !verify_client_mac(&s, &key(9), &kc, &e, &m),
            "other server key"
        );
        assert!(
            !verify_client_mac(&s, &ks, &kc, &[4; 32], &m),
            "other session"
        );
        // The two directions never stand in for each other.
        assert!(!verify_server_mac(&s, &ks, &kc, &e, &m));
        let ms = server_mac(&s, &ks, &kc, &e);
        assert!(verify_server_mac(&s, &ks, &kc, &e, &ms));
        assert_ne!(m, ms);
    }

    #[test]
    fn fixed_vectors() {
        // Pinned so that the spec's examples and other implementations can
        // be checked against this one (PROTOCOL.md §12.7).
        let (s, ks, kc, e) = ([0x11u8; 16], key(1), key(2), [0x33u8; 32]);
        let (nc, ns) = ([0x44u8; 32], [0x55u8; 32]);
        let vectors = [
            hex(&client_mac(&s, &ks, &kc, &e)),
            hex(&server_mac(&s, &ks, &kc, &e)),
            hex(&commit(&kc, &ks, &nc)),
            format_sas(sas(&e, &kc, &ks, &nc, &ns)),
        ];
        assert_eq!(vectors, FIXED_VECTORS.map(String::from));
    }

    const FIXED_VECTORS: [&str; 4] = [
        "34061136437d946a87b158b60bfb316f361cd3ad9ea721ef346fa3cf4dc67f6f",
        "b434849080e57e61e837b556759352bf6d34d0e7d67f157a930f654fff8a37b3",
        "649b9a0ba332e1b048f68641303bd9b699ada816d3079976c542d7caf4edac10",
        "020 627",
    ];

    #[test]
    fn sas_has_six_digits_and_depends_on_every_input() {
        let (e, kc, ks, nc, ns) = ([1u8; 32], key(1), key(2), [3u8; 32], [4u8; 32]);
        let code = sas(&e, &kc, &ks, &nc, &ns);
        assert!(code < 1_000_000);
        assert_eq!(code, sas(&e, &kc, &ks, &nc, &ns));
        assert_ne!(code, sas(&[2; 32], &kc, &ks, &nc, &ns));
        assert_ne!(code, sas(&e, &ks, &kc, &nc, &ns));
        assert_ne!(code, sas(&e, &kc, &ks, &[5; 32], &ns));
        assert_ne!(code, sas(&e, &kc, &ks, &nc, &[5; 32]));
        assert_eq!(format_sas(42_917), "042 917");
        assert_eq!(format_sas(7), "000 007");
    }
}
