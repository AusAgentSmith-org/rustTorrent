//! Hash correctness integration tests against known test vectors.

use librtbit_sha1_wrapper::{ISha1, Sha1};

#[test]
fn test_sha1_empty_input() {
    let hasher = Sha1::new();
    let hash = hasher.finish();
    let hex = hex::encode(hash);
    assert_eq!(hex, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
}

#[test]
fn test_sha1_abc() {
    let mut hasher = Sha1::new();
    hasher.update(b"abc");
    let hash = hasher.finish();
    let hex = hex::encode(hash);
    assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
}

#[test]
fn test_sha1_incremental_matches_oneshot() {
    // Multiple update() calls should equal a single update().
    let data = b"The quick brown fox jumps over the lazy dog";

    let mut oneshot = Sha1::new();
    oneshot.update(data);
    let hash_oneshot = oneshot.finish();

    let mut incremental = Sha1::new();
    incremental.update(&data[..10]);
    incremental.update(&data[10..30]);
    incremental.update(&data[30..]);
    let hash_incremental = incremental.finish();

    assert_eq!(hash_oneshot, hash_incremental);
}

#[test]
fn test_sha1_large_input() {
    // 1 MB of zeros — verify consistent hash.
    let data = vec![0u8; 1_048_576];
    let mut hasher = Sha1::new();
    hasher.update(&data);
    let hash1 = hasher.finish();

    let mut hasher2 = Sha1::new();
    hasher2.update(&data);
    let hash2 = hasher2.finish();

    assert_eq!(hash1, hash2, "same input must produce same hash");
    assert_ne!(hash1, [0u8; 20], "hash of zeros should not be all zeros");
}
