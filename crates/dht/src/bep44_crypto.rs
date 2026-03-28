//! BEP 44 cryptographic utilities for mutable DHT items.
//!
//! Provides Ed25519 signing/verification and target computation for BEP 44
//! (Storing arbitrary data in the DHT) mutable items.

use librtbit_core::hash_id::Id20;
use sha1w::ISha1;

/// Compute the DHT target for a BEP 44 mutable item.
///
/// For mutable items, the target is SHA-1 of the public key concatenated
/// with the optional salt:
/// - `target = SHA-1(public_key)` if no salt
/// - `target = SHA-1(public_key + salt)` if salt is provided
pub fn mutable_item_target(public_key: &[u8; 32], salt: Option<&[u8]>) -> Id20 {
    let mut hasher = sha1w::Sha1::new();
    hasher.update(public_key);
    if let Some(s) = salt {
        hasher.update(s);
    }
    Id20::new(hasher.finish())
}

/// Construct the signable data for a BEP 44 mutable item.
///
/// The data to be signed is constructed per BEP 44:
/// ```text
/// [4:salt{len}:{salt}]3:seqi{seq}e1:v{bencoded_v}
/// ```
///
/// The salt prefix is only included when salt is present.
/// `v` should be the bencoded representation of the value.
pub fn signable_data(salt: Option<&[u8]>, seq: i64, v: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();

    // Optional salt prefix
    if let Some(s) = salt {
        buf.extend_from_slice(format!("4:salt{}:", s.len()).as_bytes());
        buf.extend_from_slice(s);
    }

    // Sequence number
    buf.extend_from_slice(format!("3:seqi{seq}e").as_bytes());

    // Value
    buf.extend_from_slice(b"1:v");
    buf.extend_from_slice(v);

    buf
}

/// Verify a BEP 44 mutable item Ed25519 signature.
///
/// Returns `true` if the signature is valid for the given parameters.
pub fn verify_mutable_item(
    public_key: &[u8; 32],
    signature: &[u8; 64],
    salt: Option<&[u8]>,
    seq: i64,
    v: &[u8],
) -> bool {
    use aws_lc_rs::signature;

    let msg = signable_data(salt, seq, v);

    let peer_public_key = signature::UnparsedPublicKey::new(&signature::ED25519, public_key);
    peer_public_key.verify(&msg, signature).is_ok()
}

/// Sign a BEP 44 mutable item with an Ed25519 private key seed.
///
/// `private_key_seed` is the 32-byte Ed25519 seed (private key material).
/// Returns the 64-byte Ed25519 signature.
///
/// # Panics
///
/// Panics if the seed is invalid (should not happen with a valid 32-byte seed).
pub fn sign_mutable_item(
    private_key_seed: &[u8; 32],
    salt: Option<&[u8]>,
    seq: i64,
    v: &[u8],
) -> [u8; 64] {
    use aws_lc_rs::signature;

    let msg = signable_data(salt, seq, v);

    // Ed25519 key pair from the 32-byte seed.
    let key_pair = signature::Ed25519KeyPair::from_seed_unchecked(private_key_seed)
        .expect("valid Ed25519 seed");
    let sig = key_pair.sign(&msg);
    let sig_bytes = sig.as_ref();

    let mut result = [0u8; 64];
    result.copy_from_slice(sig_bytes);
    result
}

/// Extract the 32-byte Ed25519 public key from a 32-byte seed.
///
/// Useful for deriving the public key to compute the DHT target.
pub fn public_key_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    use aws_lc_rs::signature::{self, KeyPair};

    let key_pair =
        signature::Ed25519KeyPair::from_seed_unchecked(seed).expect("valid Ed25519 seed");
    let pub_key = key_pair.public_key().as_ref();
    let mut result = [0u8; 32];
    result.copy_from_slice(pub_key);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signable_data_no_salt() {
        let v = b"12:Hello World!";
        let data = signable_data(None, 1, v);
        assert_eq!(&data, b"3:seqi1e1:v12:Hello World!");
    }

    #[test]
    fn test_signable_data_with_salt() {
        let v = b"12:Hello World!";
        let salt = b"foobar";
        let data = signable_data(Some(salt), 4, v);
        assert_eq!(&data, b"4:salt6:foobar3:seqi4e1:v12:Hello World!");
    }

    #[test]
    fn test_signable_data_negative_seq() {
        let v = b"i42e";
        let data = signable_data(None, -1, v);
        assert_eq!(&data, b"3:seqi-1e1:vi42e");
    }

    #[test]
    fn test_signable_data_zero_seq() {
        let v = b"i0e";
        let data = signable_data(None, 0, v);
        assert_eq!(&data, b"3:seqi0e1:vi0e");
    }

    #[test]
    fn test_signable_data_empty_salt() {
        let v = b"i1e";
        // Empty salt is still included per BEP 44 spec
        let data = signable_data(Some(b""), 1, v);
        assert_eq!(&data, b"4:salt0:3:seqi1e1:vi1e");
    }

    #[test]
    fn test_sign_and_verify_roundtrip() {
        let seed: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        let public_key = public_key_from_seed(&seed);
        let v = b"12:Hello World!";
        let seq = 1i64;

        let sig = sign_mutable_item(&seed, None, seq, v);
        assert!(verify_mutable_item(&public_key, &sig, None, seq, v));
    }

    #[test]
    fn test_sign_and_verify_with_salt() {
        let seed: [u8; 32] = [0xAA; 32];
        let public_key = public_key_from_seed(&seed);
        let salt = b"test-salt";
        let v = b"5:hello";
        let seq = 42i64;

        let sig = sign_mutable_item(&seed, Some(salt), seq, v);
        assert!(verify_mutable_item(&public_key, &sig, Some(salt), seq, v));
    }

    #[test]
    fn test_verify_rejects_wrong_key() {
        let seed: [u8; 32] = [0x01; 32];
        let wrong_key: [u8; 32] = [0x02; 32];
        let v = b"i42e";
        let seq = 1i64;

        let sig = sign_mutable_item(&seed, None, seq, v);
        // Wrong key should fail verification (wrong_key is random bytes, not a valid public key
        // for the signing seed, so verification must fail).
        assert!(!verify_mutable_item(&wrong_key, &sig, None, seq, v));
    }

    #[test]
    fn test_verify_rejects_wrong_seq() {
        let seed: [u8; 32] = [0x01; 32];
        let public_key = public_key_from_seed(&seed);
        let v = b"i42e";

        let sig = sign_mutable_item(&seed, None, 1, v);
        // Different seq should fail
        assert!(!verify_mutable_item(&public_key, &sig, None, 2, v));
    }

    #[test]
    fn test_verify_rejects_wrong_value() {
        let seed: [u8; 32] = [0x01; 32];
        let public_key = public_key_from_seed(&seed);

        let sig = sign_mutable_item(&seed, None, 1, b"i42e");
        // Different value should fail
        assert!(!verify_mutable_item(&public_key, &sig, None, 1, b"i99e"));
    }

    #[test]
    fn test_verify_rejects_wrong_salt() {
        let seed: [u8; 32] = [0x01; 32];
        let public_key = public_key_from_seed(&seed);
        let v = b"i42e";

        let sig = sign_mutable_item(&seed, Some(b"salt1"), 1, v);
        // Different salt should fail
        assert!(!verify_mutable_item(
            &public_key,
            &sig,
            Some(b"salt2"),
            1,
            v
        ));
    }

    #[test]
    fn test_mutable_item_target_no_salt() {
        let public_key: [u8; 32] = [0xAA; 32];
        let target = mutable_item_target(&public_key, None);
        // Verify it is the SHA-1 of just the public key
        let mut hasher = sha1w::Sha1::new();
        hasher.update(&public_key);
        let expected = Id20::new(hasher.finish());
        assert_eq!(target, expected);
    }

    #[test]
    fn test_mutable_item_target_with_salt() {
        let public_key: [u8; 32] = [0xBB; 32];
        let salt = b"my-salt";
        let target = mutable_item_target(&public_key, Some(salt));
        // Verify it is SHA-1 of public_key + salt
        let mut hasher = sha1w::Sha1::new();
        hasher.update(&public_key);
        hasher.update(salt);
        let expected = Id20::new(hasher.finish());
        assert_eq!(target, expected);
    }

    #[test]
    fn test_public_key_from_seed_deterministic() {
        let seed: [u8; 32] = [0x42; 32];
        let pk1 = public_key_from_seed(&seed);
        let pk2 = public_key_from_seed(&seed);
        assert_eq!(pk1, pk2);
    }

    #[test]
    fn test_sign_deterministic() {
        let seed: [u8; 32] = [0x42; 32];
        let v = b"3:foo";
        let sig1 = sign_mutable_item(&seed, None, 1, v);
        let sig2 = sign_mutable_item(&seed, None, 1, v);
        // Ed25519 is deterministic (no random nonce)
        assert_eq!(sig1, sig2);
    }
}
