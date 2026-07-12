//! Functional tests for librtbit-core types.

use std::str::FromStr;

use librtbit_core::hash_id::{Id20, Id32};
use librtbit_core::magnet::Magnet;

#[test]
fn test_hash_id_from_hex() {
    let hex_str = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";
    let id = Id20::from_str(hex_str).unwrap();
    assert_eq!(id.as_string(), hex_str);
}

#[test]
fn test_hash_id_from_bytes() {
    let bytes = [0x42u8; 20];
    let id = Id20::new(bytes);
    assert_eq!(id.0, bytes);
    let hex = id.as_string();
    assert_eq!(hex, "4242424242424242424242424242424242424242");
}

#[test]
fn test_hash_id_equality() {
    let id1 = Id20::new([0xAA; 20]);
    let id2 = Id20::new([0xAA; 20]);
    let id3 = Id20::new([0xBB; 20]);
    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
}

#[test]
fn test_id32_from_hex() {
    let hex_str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let id = Id32::from_str(hex_str).unwrap();
    assert_eq!(id.as_string(), hex_str);
}

#[test]
fn test_magnet_link_parse() {
    let uri = "magnet:?xt=urn:btih:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d&dn=TestFile&tr=http://tracker.example.com/announce";
    let magnet = Magnet::parse(uri).unwrap();
    assert!(magnet.as_id20().is_some());
    assert_eq!(magnet.name.as_deref(), Some("TestFile"));
    assert_eq!(magnet.trackers.len(), 1);
    assert!(magnet.trackers[0].contains("tracker.example.com"));
}

#[test]
fn test_magnet_link_with_multiple_trackers() {
    let uri = "magnet:?xt=urn:btih:0000000000000000000000000000000000000000&tr=http://t1.com/a&tr=http://t2.com/a";
    let magnet = Magnet::parse(uri).unwrap();
    assert_eq!(magnet.trackers.len(), 2);
}
