//! Functional tests for ByteBuf and ByteBufOwned.

use std::collections::HashMap;

use librtbit_buffers::ByteBufOwned;

#[test]
fn test_byte_buf_owned_from_vec() {
    let data = vec![1, 2, 3, 4, 5];
    let buf = ByteBufOwned::from(data.clone());
    assert_eq!(buf.as_ref(), &data[..]);
}

#[test]
fn test_byte_buf_owned_as_key_in_hashmap() {
    let mut map: HashMap<ByteBufOwned, &str> = HashMap::new();
    let key1 = ByteBufOwned::from(b"key1".to_vec());
    let key2 = ByteBufOwned::from(b"key2".to_vec());

    map.insert(key1.clone(), "value1");
    map.insert(key2.clone(), "value2");

    assert_eq!(map[&key1], "value1");
    assert_eq!(map[&key2], "value2");
    assert_eq!(map.len(), 2);
}

#[test]
fn test_byte_buf_owned_display_binary() {
    // Non-UTF8 data should display as hex.
    let buf = ByteBufOwned::from(vec![0xFF, 0xFE, 0xFD]);
    let display = format!("{buf}");
    assert!(
        !display.is_empty(),
        "display should produce output for binary data"
    );
}
