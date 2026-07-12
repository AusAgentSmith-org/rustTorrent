// This crate used for making working with &[u8] or Vec<u8> generic in other parts of librtbit,
// for nicer display of binary data etc.
//
// Not useful outside of librtbit.

use std::borrow::Borrow;

use bytes::Bytes;
use serde::{Deserializer, Serialize};
use serde_derive::Deserialize;

use clone_to_owned::CloneToOwned;

#[derive(Default, PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub struct ByteBufOwned(pub bytes::Bytes);

#[derive(Default, Deserialize, PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ByteBuf<'a>(pub &'a [u8]);

pub trait ByteBufT:
    AsRef<[u8]>
    + Default
    + std::hash::Hash
    + Serialize
    + Eq
    + core::fmt::Debug
    + CloneToOwned
    + Borrow<[u8]>
{
}

impl ByteBufT for ByteBufOwned {}

impl ByteBufT for ByteBuf<'_> {}

struct HexBytes<'a>(&'a [u8]);
impl std::fmt::Display for HexBytes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x?}")?;
        }
        Ok(())
    }
}

fn debug_bytes(b: &[u8], f: &mut std::fmt::Formatter<'_>, debug_strings: bool) -> std::fmt::Result {
    if b.is_empty() {
        return Ok(());
    }
    if b.iter().all(|b| *b == 0) {
        return write!(f, "<{} bytes, all zeroes>", b.len());
    }
    match std::str::from_utf8(b) {
        Ok(s) => {
            if debug_strings {
                return write!(f, "{s:?}");
            } else {
                return write!(f, "{s}");
            }
        }
        Err(_e) => {}
    };

    // up to 20 bytes, display hex
    if b.len() <= 20 {
        return write!(f, "<{} bytes, 0x{}>", b.len(), HexBytes(b));
    }

    write!(f, "<{} bytes>", b.len())
}

impl std::fmt::Debug for ByteBuf<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_bytes(self.0, f, true)
    }
}

impl std::fmt::Display for ByteBuf<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_bytes(self.0, f, false)
    }
}

impl std::fmt::Debug for ByteBufOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_bytes(&self.0, f, true)
    }
}

impl std::fmt::Display for ByteBufOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_bytes(&self.0, f, false)
    }
}

impl CloneToOwned for ByteBuf<'_> {
    type Target = ByteBufOwned;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        // Try zero-copy from the provided buffer.
        if let Some(within_buffer) = within_buffer {
            let haystack = within_buffer.as_ptr() as usize;
            let haystack_end = haystack + within_buffer.len();
            let needle = self.0.as_ptr() as usize;
            let needle_end = needle + self.0.len();

            if needle >= haystack && needle_end <= haystack_end {
                return ByteBufOwned(within_buffer.slice_ref(self.0.as_ref()));
            } else {
                #[cfg(debug_assertions)]
                panic!("bug: broken buffers! not inside within_buffer");
            }
        }

        ByteBufOwned(Bytes::copy_from_slice(self.0))
    }
}

impl CloneToOwned for ByteBufOwned {
    type Target = ByteBufOwned;

    fn clone_to_owned(&self, _within_buffer: Option<&Bytes>) -> Self::Target {
        ByteBufOwned(self.0.clone())
    }
}

impl std::convert::AsRef<[u8]> for ByteBuf<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl std::convert::AsRef<[u8]> for ByteBufOwned {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::borrow::Borrow<[u8]> for ByteBufOwned {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl std::borrow::Borrow<[u8]> for ByteBuf<'_> {
    fn borrow(&self) -> &[u8] {
        self.0
    }
}

impl<'a> From<&'a [u8]> for ByteBuf<'a> {
    fn from(b: &'a [u8]) -> Self {
        Self(b)
    }
}

impl<'a> From<&'a [u8]> for ByteBufOwned {
    fn from(b: &'a [u8]) -> Self {
        Self(b.to_owned().into())
    }
}

impl From<Vec<u8>> for ByteBufOwned {
    fn from(b: Vec<u8>) -> Self {
        Self(b.into())
    }
}

impl From<Bytes> for ByteBufOwned {
    fn from(b: Bytes) -> Self {
        Self(b)
    }
}

impl serde::ser::Serialize for ByteBuf<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(self.as_ref())
    }
}

impl serde::ser::Serialize for ByteBufOwned {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(self.as_ref())
    }
}

impl<'de> serde::de::Deserialize<'de> for ByteBufOwned {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = ByteBufOwned;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("byte string")
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(v.to_owned().into())
            }
        }
        deserializer.deserialize_byte_buf(Visitor {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_buf_owned_from_vec() {
        let v = vec![1u8, 2, 3, 4, 5];
        let buf = ByteBufOwned::from(v.clone());
        assert_eq!(buf.as_ref(), &v[..]);
    }

    #[test]
    fn test_byte_buf_owned_as_ref() {
        let data = vec![10u8, 20, 30];
        let buf = ByteBufOwned::from(data.clone());
        let slice: &[u8] = buf.as_ref();
        assert_eq!(slice, &[10, 20, 30]);
    }

    #[test]
    fn test_byte_buf_owned_clone() {
        let buf = ByteBufOwned::from(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let cloned = buf.clone();
        assert_eq!(buf.as_ref(), cloned.as_ref());
    }

    #[test]
    fn test_byte_buf_owned_empty() {
        let buf = ByteBufOwned::default();
        assert_eq!(buf.as_ref(), &[] as &[u8]);
        assert!(buf.as_ref().is_empty());
    }

    #[test]
    fn test_byte_buf_owned_from_slice() {
        let data: &[u8] = &[42, 43, 44];
        let buf = ByteBufOwned::from(data);
        assert_eq!(buf.as_ref(), data);
    }

    #[test]
    fn test_byte_buf_owned_from_bytes() {
        let b = Bytes::from_static(&[1, 2, 3]);
        let buf = ByteBufOwned::from(b.clone());
        assert_eq!(buf.as_ref(), b.as_ref());
    }

    #[test]
    fn test_byte_buf_owned_partial_eq() {
        let a = ByteBufOwned::from(vec![1, 2, 3]);
        let b = ByteBufOwned::from(vec![1, 2, 3]);
        let c = ByteBufOwned::from(vec![4, 5, 6]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_byte_buf_owned_borrow() {
        let buf = ByteBufOwned::from(vec![7, 8, 9]);
        let borrowed: &[u8] = std::borrow::Borrow::borrow(&buf);
        assert_eq!(borrowed, &[7, 8, 9]);
    }

    #[test]
    fn test_byte_buf_owned_debug_utf8() {
        // When bytes are valid UTF-8, debug should show the string
        let buf = ByteBufOwned::from(b"hello".to_vec());
        let debug = format!("{:?}", buf);
        assert!(debug.contains("hello"), "got: {}", debug);
    }

    #[test]
    fn test_byte_buf_owned_debug_hex() {
        // When bytes are not valid UTF-8 and <= 20 bytes, debug should show hex
        let buf = ByteBufOwned::from(vec![0xFF, 0xFE, 0xFD]);
        let debug = format!("{:?}", buf);
        assert!(debug.contains("bytes"), "got: {}", debug);
        assert!(debug.contains("fffefd"), "got: {}", debug);
    }

    #[test]
    fn test_byte_buf_owned_debug_all_zeroes() {
        let buf = ByteBufOwned::from(vec![0u8; 10]);
        let debug = format!("{:?}", buf);
        assert!(
            debug.contains("all zeroes"),
            "expected 'all zeroes' in debug output, got: {}",
            debug
        );
    }

    #[test]
    fn test_byte_buf_owned_display_utf8() {
        // Display should show the string without quotes
        let buf = ByteBufOwned::from(b"world".to_vec());
        let display = format!("{}", buf);
        assert_eq!(display, "world");
    }

    #[test]
    fn test_byte_buf_borrowed_from_slice() {
        let data: &[u8] = &[1, 2, 3];
        let buf = ByteBuf::from(data);
        assert_eq!(buf.as_ref(), data);
    }

    #[test]
    fn test_byte_buf_borrowed_default() {
        let buf = ByteBuf::default();
        assert!(buf.as_ref().is_empty());
    }

    #[test]
    fn test_byte_buf_borrowed_partial_eq() {
        let a = ByteBuf(&[1, 2]);
        let b = ByteBuf(&[1, 2]);
        let c = ByteBuf(&[3, 4]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_byte_buf_borrowed_borrow() {
        let buf = ByteBuf(&[10, 20]);
        let borrowed: &[u8] = std::borrow::Borrow::borrow(&buf);
        assert_eq!(borrowed, &[10, 20]);
    }

    #[test]
    fn test_byte_buf_owned_ord() {
        let a = ByteBufOwned::from(vec![1, 2, 3]);
        let b = ByteBufOwned::from(vec![1, 2, 4]);
        let c = ByteBufOwned::from(vec![1, 2, 3]);
        assert!(a < b);
        assert_eq!(a.cmp(&c), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_byte_buf_owned_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ByteBufOwned::from(vec![1, 2, 3]));
        set.insert(ByteBufOwned::from(vec![1, 2, 3])); // duplicate
        set.insert(ByteBufOwned::from(vec![4, 5, 6]));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_clone_to_owned_byte_buf_without_parent() {
        let data: &[u8] = &[100, 200];
        let buf = ByteBuf(data);
        let owned = buf.clone_to_owned(None);
        assert_eq!(owned.as_ref(), data);
    }

    #[test]
    fn test_clone_to_owned_byte_buf_owned() {
        let buf = ByteBufOwned::from(vec![5, 6, 7]);
        let cloned = buf.clone_to_owned(None);
        assert_eq!(cloned.as_ref(), buf.as_ref());
    }
}
