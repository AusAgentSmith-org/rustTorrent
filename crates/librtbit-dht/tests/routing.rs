//! Functional tests for DHT BEP-44 cryptography and mutable item store.

use librtbit_core::hash_id::Id20;
use librtbit_dht::bep44_crypto::{
    mutable_item_target, public_key_from_seed, sign_mutable_item, verify_mutable_item,
};
use librtbit_dht::mutable_item_store::{MutableItemStore, StoredMutableItem};

// ---------------------------------------------------------------------------
// BEP-44 crypto
// ---------------------------------------------------------------------------

#[test]
fn test_sign_and_verify_mutable_item() {
    let seed = [0x42u8; 32];
    let public_key = public_key_from_seed(&seed);
    let value = b"Hello, DHT!";

    let sig = sign_mutable_item(&seed, None, 1, value);
    assert!(
        verify_mutable_item(&public_key, &sig, None, 1, value),
        "valid signature should verify"
    );
}

#[test]
fn test_verify_rejects_wrong_key() {
    let seed = [0x42u8; 32];
    let wrong_key = [0xFF; 32];
    let value = b"test data";

    let sig = sign_mutable_item(&seed, None, 1, value);
    assert!(
        !verify_mutable_item(&wrong_key, &sig, None, 1, value),
        "wrong public key should not verify"
    );
}

#[test]
fn test_verify_rejects_tampered_data() {
    let seed = [0x42u8; 32];
    let public_key = public_key_from_seed(&seed);
    let value = b"original data";

    let sig = sign_mutable_item(&seed, None, 1, value);
    assert!(
        !verify_mutable_item(&public_key, &sig, None, 1, b"tampered data"),
        "tampered data should not verify"
    );
}

#[test]
fn test_sign_with_salt() {
    let seed = [0x01u8; 32];
    let public_key = public_key_from_seed(&seed);
    let value = b"salted value";
    let salt = b"mysalt";

    let sig = sign_mutable_item(&seed, Some(salt), 5, value);
    assert!(verify_mutable_item(&public_key, &sig, Some(salt), 5, value));

    // Different salt should fail
    assert!(!verify_mutable_item(
        &public_key,
        &sig,
        Some(b"wrong"),
        5,
        value
    ));
}

#[test]
fn test_mutable_item_target_deterministic() {
    let public_key = [0xAA; 32];
    let target1 = mutable_item_target(&public_key, None);
    let target2 = mutable_item_target(&public_key, None);
    assert_eq!(target1, target2, "target should be deterministic");
}

// ---------------------------------------------------------------------------
// Mutable item store
// ---------------------------------------------------------------------------

#[test]
fn test_mutable_item_store_put_get() {
    let store = MutableItemStore::new(100);
    let target = Id20::new([0xBB; 20]);
    let item = StoredMutableItem {
        k: [0x11; 32],
        sig: [0x22; 64],
        seq: 1,
        v: b"stored value".to_vec(),
        salt: None,
        last_updated: std::time::Instant::now(),
    };

    assert!(store.store(target, item));
    let retrieved = store.get(&target);
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().v, b"stored value");
}

#[test]
fn test_mutable_item_store_overwrites_higher_seq() {
    let store = MutableItemStore::new(100);
    let target = Id20::new([0xCC; 20]);

    let item1 = StoredMutableItem {
        k: [0x11; 32],
        sig: [0x22; 64],
        seq: 1,
        v: b"v1".to_vec(),
        salt: None,
        last_updated: std::time::Instant::now(),
    };
    store.store(target, item1);

    let item2 = StoredMutableItem {
        k: [0x11; 32],
        sig: [0x22; 64],
        seq: 5,
        v: b"v5".to_vec(),
        salt: None,
        last_updated: std::time::Instant::now(),
    };
    store.store(target, item2);

    let retrieved = store.get(&target).unwrap();
    assert_eq!(retrieved.seq, 5);
    assert_eq!(retrieved.v, b"v5");
}
