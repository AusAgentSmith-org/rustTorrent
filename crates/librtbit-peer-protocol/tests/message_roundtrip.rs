//! Message serialization/deserialization round-trip tests.

use librtbit_core::hash_id::Id20;
use librtbit_peer_protocol::{Handshake, Message, Request};

const HANDSHAKE_LEN: usize = 68;

#[test]
fn test_handshake_encode_decode() {
    let info_hash = Id20::new([0xAA; 20]);
    let peer_id = Id20::new([0xBB; 20]);
    let hs = Handshake::new(info_hash, peer_id);

    let mut buf = [0u8; HANDSHAKE_LEN];
    let written = hs.serialize_unchecked_len(&mut buf);
    assert_eq!(written, HANDSHAKE_LEN);

    let (decoded, consumed) = Handshake::deserialize(&buf).unwrap();
    assert_eq!(consumed, HANDSHAKE_LEN);
    assert_eq!(decoded.info_hash, info_hash);
    assert_eq!(decoded.peer_id, peer_id);
}

#[test]
fn test_request_message_serialize() {
    let req = Request::new(5, 16384, 16384);
    let mut buf = [0u8; 128];
    let written = req.serialize_unchecked_len(&mut buf);
    assert!(written > 0, "serialization should produce bytes");
    // Request is 12 bytes: index(4) + begin(4) + length(4)
    assert_eq!(written, 12);
}

#[test]
fn test_message_keepalive_deserialize() {
    // KeepAlive is a 4-byte zero-length prefix: [0, 0, 0, 0]
    let buf = [0u8; 4];
    let (msg, consumed) = Message::deserialize(&buf, &[]).unwrap();
    assert_eq!(consumed, 4);
    assert!(matches!(msg, Message::KeepAlive));
}

#[test]
fn test_message_choke_deserialize() {
    // Choke: length=1, id=0 → [0, 0, 0, 1, 0]
    let buf = [0, 0, 0, 1, 0];
    let (msg, consumed) = Message::deserialize(&buf, &[]).unwrap();
    assert_eq!(consumed, 5);
    assert!(matches!(msg, Message::Choke));
}

#[test]
fn test_message_interested_deserialize() {
    // Interested: length=1, id=2 → [0, 0, 0, 1, 2]
    let buf = [0, 0, 0, 1, 2];
    let (msg, consumed) = Message::deserialize(&buf, &[]).unwrap();
    assert_eq!(consumed, 5);
    assert!(matches!(msg, Message::Interested));
}
