//! Storage for BEP 44 mutable DHT items.
//!
//! Provides an in-memory store for mutable items indexed by their DHT target.
//! Items are stored with sequence number ordering: newer items (higher seq)
//! replace older ones.

use std::time::Instant;

use dashmap::DashMap;
use librtbit_core::hash_id::Id20;

/// A stored BEP 44 mutable item.
#[derive(Debug, Clone)]
pub struct StoredMutableItem {
    /// 32-byte Ed25519 public key.
    pub k: [u8; 32],
    /// 64-byte Ed25519 signature.
    pub sig: [u8; 64],
    /// Sequence number.
    pub seq: i64,
    /// The bencoded value (max 1000 bytes per BEP 44).
    pub v: Vec<u8>,
    /// Optional salt used in target computation.
    pub salt: Option<Vec<u8>>,
    /// When this item was last stored or updated.
    pub last_updated: Instant,
}

/// Thread-safe store for BEP 44 mutable DHT items.
///
/// Items are keyed by their DHT target (SHA-1 of public_key [+ salt]).
/// The store enforces a maximum capacity and sequence number ordering.
pub struct MutableItemStore {
    items: DashMap<Id20, StoredMutableItem>,
    max_items: usize,
}

impl MutableItemStore {
    /// Create a new store with the given maximum capacity.
    pub fn new(max_items: usize) -> Self {
        Self {
            items: DashMap::new(),
            max_items,
        }
    }

    /// Store a mutable item.
    ///
    /// Returns `true` if the item was stored (either new or had a higher seq).
    /// Returns `false` if an existing item has a seq >= the new item's seq.
    ///
    /// If the store is at capacity and the item is new, garbage collection
    /// is performed first to evict the oldest item.
    pub fn store(&self, target: Id20, item: StoredMutableItem) -> bool {
        // Check if there is an existing item with a higher or equal seq.
        if let Some(existing) = self.items.get(&target)
            && existing.seq >= item.seq
        {
            return false;
        }

        // If at capacity and this is a new item, evict the oldest.
        if !self.items.contains_key(&target) && self.items.len() >= self.max_items {
            self.evict_oldest();
        }

        self.items.insert(target, item);
        true
    }

    /// Retrieve a stored mutable item by its DHT target.
    pub fn get(&self, target: &Id20) -> Option<StoredMutableItem> {
        self.items.get(target).map(|entry| entry.clone())
    }

    /// Remove items that are oldest by `last_updated`, bringing the store
    /// back under its maximum capacity.
    pub fn garbage_collect(&self) {
        while self.items.len() > self.max_items {
            self.evict_oldest();
        }
    }

    /// Returns the number of items currently stored.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns true if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Evict the single oldest item (by `last_updated`).
    fn evict_oldest(&self) {
        let oldest = self
            .items
            .iter()
            .min_by_key(|entry| entry.value().last_updated)
            .map(|entry| *entry.key());

        if let Some(key) = oldest {
            self.items.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(seq: i64) -> StoredMutableItem {
        StoredMutableItem {
            k: [0xAA; 32],
            sig: [0xBB; 64],
            seq,
            v: b"5:hello".to_vec(),
            salt: None,
            last_updated: Instant::now(),
        }
    }

    fn make_item_with_time(seq: i64, time: Instant) -> StoredMutableItem {
        StoredMutableItem {
            k: [0xAA; 32],
            sig: [0xBB; 64],
            seq,
            v: b"5:hello".to_vec(),
            salt: None,
            last_updated: time,
        }
    }

    #[test]
    fn test_store_and_get() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);
        let item = make_item(1);

        assert!(store.store(target, item));

        let retrieved = store.get(&target).unwrap();
        assert_eq!(retrieved.seq, 1);
        assert_eq!(retrieved.v, b"5:hello");
    }

    #[test]
    fn test_get_nonexistent() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);
        assert!(store.get(&target).is_none());
    }

    #[test]
    fn test_store_higher_seq_overwrites() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);

        assert!(store.store(target, make_item(1)));
        assert!(store.store(target, make_item(5)));

        let retrieved = store.get(&target).unwrap();
        assert_eq!(retrieved.seq, 5);
    }

    #[test]
    fn test_store_lower_seq_rejected() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);

        assert!(store.store(target, make_item(5)));
        assert!(!store.store(target, make_item(3)));

        let retrieved = store.get(&target).unwrap();
        assert_eq!(retrieved.seq, 5);
    }

    #[test]
    fn test_store_equal_seq_rejected() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);

        assert!(store.store(target, make_item(5)));
        assert!(!store.store(target, make_item(5)));

        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_max_capacity_eviction() {
        let store = MutableItemStore::new(2);
        let now = Instant::now();

        let t1 = Id20::new([0x01; 20]);
        let t2 = Id20::new([0x02; 20]);
        let t3 = Id20::new([0x03; 20]);

        // Store two items; the first is oldest.
        assert!(store.store(t1, make_item_with_time(1, now)));
        assert!(store.store(
            t2,
            make_item_with_time(1, now + std::time::Duration::from_secs(1))
        ));
        assert_eq!(store.len(), 2);

        // Storing a third item should evict the oldest (t1).
        assert!(store.store(
            t3,
            make_item_with_time(1, now + std::time::Duration::from_secs(2))
        ));
        assert_eq!(store.len(), 2);
        assert!(store.get(&t1).is_none());
        assert!(store.get(&t2).is_some());
        assert!(store.get(&t3).is_some());
    }

    #[test]
    fn test_update_existing_does_not_evict() {
        let store = MutableItemStore::new(2);

        let t1 = Id20::new([0x01; 20]);
        let t2 = Id20::new([0x02; 20]);

        assert!(store.store(t1, make_item(1)));
        assert!(store.store(t2, make_item(1)));
        assert_eq!(store.len(), 2);

        // Updating an existing item (higher seq) should not trigger eviction.
        assert!(store.store(t1, make_item(2)));
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(&t1).unwrap().seq, 2);
        assert!(store.get(&t2).is_some());
    }

    #[test]
    fn test_garbage_collect() {
        let store = MutableItemStore::new(3);
        let now = Instant::now();

        for i in 0..5u8 {
            let target = Id20::new([i; 20]);
            // Bypass max_items by inserting directly for this test.
            store.items.insert(
                target,
                make_item_with_time(i as i64, now + std::time::Duration::from_secs(i as u64)),
            );
        }

        assert_eq!(store.len(), 5);

        store.garbage_collect();

        // Should be back to max_items (3), with the 2 oldest removed.
        assert_eq!(store.len(), 3);
        // The oldest items (i=0, i=1) should be evicted.
        assert!(store.get(&Id20::new([0; 20])).is_none());
        assert!(store.get(&Id20::new([1; 20])).is_none());
        // The newer items should remain.
        assert!(store.get(&Id20::new([2; 20])).is_some());
        assert!(store.get(&Id20::new([3; 20])).is_some());
        assert!(store.get(&Id20::new([4; 20])).is_some());
    }

    #[test]
    fn test_len_and_is_empty() {
        let store = MutableItemStore::new(10);
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        let target = Id20::new([0x11; 20]);
        store.store(target, make_item(1));
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn test_store_with_salt() {
        let store = MutableItemStore::new(100);
        let target = Id20::new([0x11; 20]);

        let mut item = make_item(1);
        item.salt = Some(b"my-salt".to_vec());
        assert!(store.store(target, item));

        let retrieved = store.get(&target).unwrap();
        assert_eq!(retrieved.salt.as_deref(), Some(b"my-salt".as_ref()));
    }
}
