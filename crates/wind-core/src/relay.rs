//! CCS/1 M:relay | PURPOSE:PROTOCOL.md §13 rules the relay client shares with the Worker in relay/
//! SEC:the relay carries QUIC packets already encrypted end to end (§2); it holds no device key.
//! Signing here only proves to the relay which device connects; trust stays with pinned keys.

use sha2::{Digest, Sha256};

use crate::identity::{DeviceIdentity, PublicKey};

/// The WebSocket path: `/v1/relay?me=<64 hex>&peer=<64 hex>`.
pub const PATH: &str = "/v1/relay";
/// Largest data frame the relay accepts (a QUIC packet; larger closes with 1009).
pub const MAX_FRAME: usize = 2048;
/// Length of the relay's challenge nonce.
pub const NONCE_LEN: usize = 32;

const PAIR_CONTEXT: &[u8] = b"wc-relay-v1 pair";
const AUTH_CONTEXT: &[u8] = b"wc-relay-v1 auth";

/// The pair's rendezvous name: SHA-256("wc-relay-v1 pair" ‖ lower key ‖
/// higher key). The same for both devices.
pub fn pair_name(a: &PublicKey, b: &PublicKey) -> [u8; 32] {
    let (low, high) = if a.as_bytes() <= b.as_bytes() {
        (a, b)
    } else {
        (b, a)
    };
    let mut hash = Sha256::new();
    hash.update(PAIR_CONTEXT);
    hash.update(low.as_bytes());
    hash.update(high.as_bytes());
    hash.finalize().into()
}

/// What a device signs to authenticate: "wc-relay-v1 auth" ‖ nonce ‖ own
/// key ‖ peer key. The context keeps it apart from the TLS signatures the
/// same key makes (§2).
pub fn auth_message(nonce: &[u8; NONCE_LEN], me: &PublicKey, peer: &PublicKey) -> Vec<u8> {
    [AUTH_CONTEXT, nonce, me.as_bytes(), peer.as_bytes()].concat()
}

/// This device's answer to the relay's challenge for a connection to `peer`.
pub fn sign_auth(identity: &DeviceIdentity, nonce: &[u8; NONCE_LEN], peer: &PublicKey) -> [u8; 64] {
    identity.sign(&auth_message(nonce, &identity.public_key(), peer))
}

/// The WebSocket path and query for `me` reaching `peer`.
pub fn request_path(me: &PublicKey, peer: &PublicKey) -> String {
    format!(
        "{PATH}?me={}&peer={}",
        hex(me.as_bytes()),
        hex(peer.as_bytes())
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};

    fn key(seed: u8) -> (DeviceIdentity, PublicKey) {
        let identity = DeviceIdentity::from_key(SigningKey::from_bytes(&[seed; 32])).unwrap();
        let public = identity.public_key();
        (identity, public)
    }

    #[test]
    fn pair_name_is_symmetric_and_distinct() {
        let (_, a) = key(7);
        let (_, b) = key(8);
        let (_, c) = key(9);
        assert_eq!(pair_name(&a, &b), pair_name(&b, &a));
        assert_ne!(pair_name(&a, &b), pair_name(&a, &c));
    }

    #[test]
    fn auth_signature_verifies_only_for_its_nonce_and_peer() {
        let (alice, a) = key(7);
        let (_, b) = key(8);
        let nonce = [1; NONCE_LEN];
        let sig = Signature::from_bytes(&sign_auth(&alice, &nonce, &b));
        let verifying = VerifyingKey::from_bytes(a.as_bytes()).unwrap();
        assert!(verifying
            .verify(&auth_message(&nonce, &a, &b), &sig)
            .is_ok());
        assert!(verifying
            .verify(&auth_message(&[2; NONCE_LEN], &a, &b), &sig)
            .is_err());
        assert!(verifying
            .verify(&auth_message(&nonce, &a, &a), &sig)
            .is_err());
    }

    /// Fixed vectors shared with relay/test/vectors.test.ts: both sides
    /// derive the same rendezvous and accept the same signature.
    #[test]
    fn vectors_match_the_worker() {
        let (alice, a) = key(7);
        let (_, b) = key(8);
        let nonce = [1; NONCE_LEN];
        assert_eq!(
            hex(a.as_bytes()),
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
        );
        assert_eq!(
            hex(b.as_bytes()),
            "1398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca"
        );
        assert_eq!(
            hex(&pair_name(&a, &b)),
            "890e36d8d7bef70886770d7b404e97b989fd1288e71d1147fef5384822b9a79f"
        );
        assert_eq!(hex(&sign_auth(&alice, &nonce, &b)), "242a37e26274de3a5a81acf4594ced30baa0b8d1a27baee55a1b27981c41235c83965a64a828f8e0caddb58f8bfc2c3b41abb8db245dc3ff365323ff897e4a04");
    }
}
