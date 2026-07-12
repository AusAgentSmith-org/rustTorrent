//! Round-trip integration tests for bencode serialization/deserialization.

use librtbit_bencode::{BencodeValue, ByteBufOwned, bencode_serialize_to_writer, from_bytes};

#[test]
fn test_roundtrip_string() {
    let input = b"5:hello";
    let val: BencodeValue<ByteBufOwned> = from_bytes(input).unwrap();
    let mut out = Vec::new();
    bencode_serialize_to_writer(&val, &mut out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_roundtrip_integer() {
    let input = b"i42e";
    let val: BencodeValue<ByteBufOwned> = from_bytes(input).unwrap();
    let mut out = Vec::new();
    bencode_serialize_to_writer(&val, &mut out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_roundtrip_list() {
    let input = b"li1ei2ei3ee";
    let val: BencodeValue<ByteBufOwned> = from_bytes(input).unwrap();
    let mut out = Vec::new();
    bencode_serialize_to_writer(&val, &mut out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_roundtrip_dict() {
    // Bencode dicts are sorted by key.
    let input = b"d3:foo3:bare";
    let val: BencodeValue<ByteBufOwned> = from_bytes(input).unwrap();
    let mut out = Vec::new();
    bencode_serialize_to_writer(&val, &mut out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_roundtrip_nested_struct() {
    // Dict containing a list and an integer.
    let input = b"d4:listli1ei2ee3:numi99ee";
    let val: BencodeValue<ByteBufOwned> = from_bytes(input).unwrap();
    let mut out = Vec::new();
    bencode_serialize_to_writer(&val, &mut out).unwrap();
    assert_eq!(out, input);
}

#[test]
fn test_deserialize_malformed_returns_error() {
    // Truncated list.
    let input = b"li1ei2e";
    let result: Result<BencodeValue<ByteBufOwned>, _> = from_bytes(input);
    assert!(result.is_err(), "truncated input should return error");
}
