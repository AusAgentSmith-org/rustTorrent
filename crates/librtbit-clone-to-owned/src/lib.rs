// These are helpers for objects that can be borrowed, but can be made owned while changing the type.
// The difference between e.g. Cow and CloneToOwned, is that we can implement it recursively for owned types.
//
// E.g. HashMap<&str, &str> can be converted to HashMap<String, String>.
//
// This lets us express types like TorrentMetaInfo<&[u8]> for zero-copy metadata about a bencode buffer in memory,
// but to have one-line conversion for it into TorrentMetaInfo<Vec<u8>> so that we can store it later somewhere.

use bytes::Bytes;
use std::collections::{BTreeMap, HashMap};

pub trait CloneToOwned {
    type Target;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target;
}

impl<T> CloneToOwned for Option<T>
where
    T: CloneToOwned,
{
    type Target = Option<<T as CloneToOwned>::Target>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        self.as_ref().map(|i| i.clone_to_owned(within_buffer))
    }
}

impl<T> CloneToOwned for Vec<T>
where
    T: CloneToOwned,
{
    type Target = Vec<<T as CloneToOwned>::Target>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        self.iter()
            .map(|i| i.clone_to_owned(within_buffer))
            .collect()
    }
}

impl CloneToOwned for u8 {
    type Target = u8;

    fn clone_to_owned(&self, _within_buffer: Option<&Bytes>) -> Self::Target {
        *self
    }
}

impl CloneToOwned for u32 {
    type Target = u32;

    fn clone_to_owned(&self, _within_buffer: Option<&Bytes>) -> Self::Target {
        *self
    }
}

impl<K, V> CloneToOwned for HashMap<K, V>
where
    K: CloneToOwned,
    <K as CloneToOwned>::Target: std::hash::Hash + Eq,
    V: CloneToOwned,
{
    type Target = HashMap<<K as CloneToOwned>::Target, <V as CloneToOwned>::Target>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        let mut result = HashMap::with_capacity(self.capacity());
        for (k, v) in self {
            result.insert(
                k.clone_to_owned(within_buffer),
                v.clone_to_owned(within_buffer),
            );
        }
        result
    }
}

impl<K, V> CloneToOwned for BTreeMap<K, V>
where
    K: CloneToOwned,
    <K as CloneToOwned>::Target: Ord,
    V: CloneToOwned,
{
    type Target = BTreeMap<<K as CloneToOwned>::Target, <V as CloneToOwned>::Target>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        self.iter()
            .map(|(k, v)| {
                (
                    k.clone_to_owned(within_buffer),
                    v.clone_to_owned(within_buffer),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn test_u8_clone_to_owned() {
        let val: u8 = 42;
        let owned: u8 = val.clone_to_owned(None);
        assert_eq!(owned, 42);
    }

    #[test]
    fn test_u32_clone_to_owned() {
        let val: u32 = 123_456;
        let owned: u32 = val.clone_to_owned(None);
        assert_eq!(owned, 123_456);
    }

    #[test]
    fn test_option_some_clone_to_owned() {
        let val: Option<u8> = Some(7);
        let owned: Option<u8> = val.clone_to_owned(None);
        assert_eq!(owned, Some(7));
    }

    #[test]
    fn test_option_none_clone_to_owned() {
        let val: Option<u8> = None;
        let owned: Option<u8> = val.clone_to_owned(None);
        assert_eq!(owned, None);
    }

    #[test]
    fn test_vec_clone_to_owned() {
        let val: Vec<u8> = vec![1, 2, 3, 4, 5];
        let owned: Vec<u8> = val.clone_to_owned(None);
        assert_eq!(owned, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_vec_empty_clone_to_owned() {
        let val: Vec<u8> = vec![];
        let owned: Vec<u8> = val.clone_to_owned(None);
        assert!(owned.is_empty());
    }

    #[test]
    fn test_hashmap_clone_to_owned() {
        let mut val: HashMap<u8, u8> = HashMap::new();
        val.insert(1, 10);
        val.insert(2, 20);
        val.insert(3, 30);
        let owned: HashMap<u8, u8> = val.clone_to_owned(None);
        assert_eq!(owned.len(), 3);
        assert_eq!(owned[&1], 10);
        assert_eq!(owned[&2], 20);
        assert_eq!(owned[&3], 30);
    }

    #[test]
    fn test_btreemap_clone_to_owned() {
        let mut val: BTreeMap<u8, u8> = BTreeMap::new();
        val.insert(3, 30);
        val.insert(1, 10);
        val.insert(2, 20);
        let owned: BTreeMap<u8, u8> = val.clone_to_owned(None);
        assert_eq!(owned.len(), 3);
        // BTreeMap preserves ordering
        let keys: Vec<u8> = owned.keys().copied().collect();
        assert_eq!(keys, vec![1, 2, 3]);
        assert_eq!(owned[&1], 10);
    }
}
