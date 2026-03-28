use std::{collections::VecDeque, net::SocketAddr, str::FromStr, sync::atomic::AtomicU32};

use bencode::ByteBufOwned;
use chrono::{DateTime, TimeDelta, Utc};
use librtbit_core::{compact_ip::CompactSocketAddr, hash_id::Id20};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{
    Deserialize, Serialize,
    ser::{SerializeMap, SerializeStruct},
};
use tracing::{debug, trace};

use crate::bprotocol::{AnnouncePeer, Want};

#[derive(Serialize, Deserialize)]
struct StoredToken {
    token: [u8; 4],
    #[serde(serialize_with = "crate::utils::serialize_id20")]
    node_id: Id20,
    addr: SocketAddr,
    #[serde(default = "Utc::now")]
    time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct StoredPeer {
    addr: SocketAddr,
    time: DateTime<Utc>,
}

pub struct PeerStore {
    self_id: Id20,
    max_remembered_tokens: u32,
    max_remembered_peers: u32,
    max_distance: Id20,
    tokens: RwLock<VecDeque<StoredToken>>,
    peers: dashmap::DashMap<Id20, Vec<StoredPeer>>,
    peers_len: AtomicU32,
}

impl Serialize for PeerStore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        struct SerializePeers<'a> {
            peers: &'a dashmap::DashMap<Id20, Vec<StoredPeer>>,
        }

        impl Serialize for SerializePeers<'_> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                let mut m = serializer.serialize_map(None)?;
                for entry in self.peers.iter() {
                    m.serialize_entry(&entry.key().as_string(), &entry.value())?;
                }
                m.end()
            }
        }

        let mut s = serializer.serialize_struct("PeerStore", 7)?;
        s.serialize_field("self_id", &self.self_id.as_string())?;
        s.serialize_field("max_remembered_tokens", &self.max_remembered_tokens)?;
        s.serialize_field("max_remembered_peers", &self.max_remembered_peers)?;
        s.serialize_field("max_distance", &self.max_distance.as_string())?;
        s.serialize_field("tokens", &*self.tokens.read())?;
        s.serialize_field("peers", &SerializePeers { peers: &self.peers })?;
        s.serialize_field(
            "peers_len",
            &self.peers_len.load(std::sync::atomic::Ordering::SeqCst),
        )?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for PeerStore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Tmp {
            self_id: Id20,
            max_remembered_tokens: u32,
            max_remembered_peers: u32,
            max_distance: Id20,
            tokens: VecDeque<StoredToken>,
            peers: dashmap::DashMap<Id20, Vec<StoredPeer>>,
        }

        Tmp::deserialize(deserializer).map(|tmp| Self {
            self_id: tmp.self_id,
            max_remembered_tokens: tmp.max_remembered_tokens,
            max_remembered_peers: tmp.max_remembered_peers,
            max_distance: tmp.max_distance,
            tokens: RwLock::new(tmp.tokens),
            peers_len: AtomicU32::new(tmp.peers.iter().map(|e| e.value().len() as u32).sum()),
            peers: tmp.peers,
        })
    }
}

impl PeerStore {
    pub fn new(self_id: Id20) -> Self {
        Self {
            self_id,
            max_remembered_tokens: 1000,
            max_remembered_peers: 1000,
            max_distance: Id20::from_str("00000fffffffffffffffffffffffffffffffffff").unwrap(),
            tokens: RwLock::new(VecDeque::new()),
            peers: dashmap::DashMap::new(),
            peers_len: AtomicU32::new(0),
        }
    }

    pub fn gen_token_for(&self, node_id: Id20, addr: SocketAddr) -> [u8; 4] {
        let mut token = [0u8; 4];
        rand::rng().fill_bytes(&mut token);
        let mut tokens = self.tokens.write();
        tokens.push_back(StoredToken {
            token,
            addr,
            node_id,
            time: Utc::now(),
        });
        if tokens.len() > self.max_remembered_tokens as usize {
            tokens.pop_front();
        }
        token
    }

    pub fn store_peer(&self, announce: &AnnouncePeer<ByteBufOwned>, mut addr: SocketAddr) -> bool {
        // If the info_hash in announce is too far away from us, don't store it.
        // If the token doesn't match, don't store it.
        // If we are out of capacity, don't store it.
        // Otherwise, store it.
        if announce.info_hash.distance(&self.self_id) > self.max_distance {
            trace!("peer store: info_hash too far to store");
            return false;
        }
        if !self.tokens.read().iter().any(|t| {
            t.token[..] == announce.token.as_ref()[..] && t.addr == addr && t.node_id == announce.id
        }) {
            trace!("peer store: can't find this token / addr combination");
            return false;
        }

        if announce.implied_port == 0 {
            addr.set_port(announce.port);
        }

        use dashmap::mapref::entry::Entry;
        let peers_entry = self.peers.entry(announce.info_hash);
        let peers_len = self.peers_len.load(std::sync::atomic::Ordering::SeqCst);
        match peers_entry {
            Entry::Occupied(mut occ) => {
                if let Some(s) = occ.get_mut().iter_mut().find(|s| s.addr == addr) {
                    s.time = Utc::now();
                    return true;
                }
                if peers_len >= self.max_remembered_peers {
                    trace!("peer store: out of capacity");
                    return false;
                }
                occ.get_mut().push(StoredPeer {
                    addr,
                    time: Utc::now(),
                });
            }
            Entry::Vacant(vac) => {
                if peers_len >= self.max_remembered_peers {
                    trace!("peer store: out of capacity");
                    return false;
                }
                vac.insert(vec![StoredPeer {
                    addr,
                    time: Utc::now(),
                }]);
            }
        }

        self.peers_len
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }

    pub fn get_for_info_hash(&self, info_hash: Id20, want: Want) -> Vec<CompactSocketAddr> {
        if let Some(stored_peers) = self.peers.get(&info_hash) {
            return stored_peers
                .iter()
                .filter(|p| {
                    matches!(
                        (p.addr, want),
                        (SocketAddr::V6(..), Want::V6 | Want::Both)
                            | (SocketAddr::V4(..), Want::V4 | Want::Both)
                    )
                })
                .map(|p| p.addr.into())
                .collect();
        }
        Vec::new()
    }

    /// Maximum number of peers to keep per info_hash.
    const MAX_PEERS_PER_INFO_HASH: usize = 100;

    /// Peers older than this are considered stale and removed.
    const PEER_TTL: TimeDelta = TimeDelta::minutes(15);

    /// Tokens older than this are removed.
    const TOKEN_TTL: TimeDelta = TimeDelta::minutes(10);

    /// Create a PeerStore with custom limits for testing.
    #[cfg(test)]
    fn with_limits(self_id: Id20, max_tokens: u32, max_peers: u32) -> Self {
        Self {
            self_id,
            max_remembered_tokens: max_tokens,
            max_remembered_peers: max_peers,
            max_distance: Id20::from_str("ffffffffffffffffffffffffffffffffffffffff").unwrap(),
            tokens: RwLock::new(VecDeque::new()),
            peers: dashmap::DashMap::new(),
            peers_len: AtomicU32::new(0),
        }
    }

    /// Run garbage collection on the peer store.
    ///
    /// This performs:
    /// 1. Token cleanup: removes tokens older than 10 minutes.
    /// 2. Peer TTL: removes peers not seen in the last 15 minutes.
    /// 3. Per-info_hash cap: keeps only the most recent peers (up to 100) per hash.
    /// 4. Global cap: if still over `max_remembered_peers`, evicts oldest peers first.
    pub fn garbage_collect_peers(&self) {
        let now = Utc::now();

        // 1. Clean up expired tokens.
        {
            let mut tokens = self.tokens.write();
            let token_cutoff = now - Self::TOKEN_TTL;
            let before = tokens.len();
            tokens.retain(|t| t.time > token_cutoff);
            let removed = before - tokens.len();
            if removed > 0 {
                debug!("peer store GC: removed {removed} expired tokens");
            }
        }

        // 2. Remove stale peers (older than PEER_TTL) and enforce per-info_hash cap.
        let peer_cutoff = now - Self::PEER_TTL;
        let mut total_removed: u32 = 0;
        let mut empty_hashes = Vec::new();

        for mut entry in self.peers.iter_mut() {
            let info_hash = *entry.key();
            let peers = entry.value_mut();
            let before = peers.len();

            // Remove peers older than the TTL.
            peers.retain(|p| p.time > peer_cutoff);

            // Enforce per-info_hash cap: keep only the most recent peers.
            if peers.len() > Self::MAX_PEERS_PER_INFO_HASH {
                peers.sort_by(|a, b| b.time.cmp(&a.time));
                peers.truncate(Self::MAX_PEERS_PER_INFO_HASH);
            }

            let removed = before - peers.len();
            total_removed += removed as u32;

            if peers.is_empty() {
                empty_hashes.push(info_hash);
            }
        }

        // Remove empty info_hash entries.
        for hash in &empty_hashes {
            self.peers.remove(hash);
        }

        // 3. Enforce global cap by evicting oldest peers first.
        //    Recompute the actual count after TTL/per-hash cleanup.
        let actual_count: u32 = self.peers.iter().map(|e| e.value().len() as u32).sum();
        let max = self.max_remembered_peers;

        if actual_count > max {
            let to_evict = actual_count - max;
            let mut evicted = 0u32;

            // Collect (info_hash, oldest_time) pairs so we can evict from the
            // entries with the oldest peers first.
            let mut entries_by_oldest: Vec<(Id20, DateTime<Utc>)> = self
                .peers
                .iter()
                .filter_map(|entry| {
                    entry
                        .value()
                        .iter()
                        .map(|p| p.time)
                        .min()
                        .map(|oldest| (*entry.key(), oldest))
                })
                .collect();
            entries_by_oldest.sort_by_key(|(_hash, oldest)| *oldest);

            for (hash, _) in entries_by_oldest {
                if evicted >= to_evict {
                    break;
                }
                if let Some(mut entry) = self.peers.get_mut(&hash) {
                    let peers = entry.value_mut();
                    peers.sort_by(|a, b| a.time.cmp(&b.time));
                    while !peers.is_empty() && evicted < to_evict {
                        peers.remove(0);
                        evicted += 1;
                    }
                    if peers.is_empty() {
                        drop(entry);
                        self.peers.remove(&hash);
                    }
                }
            }
            total_removed += evicted;
        }

        // Update the atomic counter to the true count.
        let final_count: u32 = self.peers.iter().map(|e| e.value().len() as u32).sum();
        self.peers_len
            .store(final_count, std::sync::atomic::Ordering::SeqCst);

        if total_removed > 0 {
            debug!(
                "peer store GC: removed {total_removed} peers, {remaining} remaining, {hashes} info_hashes",
                remaining = final_count,
                hashes = self.peers.len(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

    /// Helper: create a deterministic Id20 from a single byte prefix.
    fn id_from_byte(b: u8) -> Id20 {
        let mut data = [0u8; 20];
        data[0] = b;
        Id20::new(data)
    }

    /// Helper: create a deterministic IPv4 SocketAddr.
    fn addr_v4(port: u16) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port))
    }

    /// Helper: create a deterministic IPv6 SocketAddr.
    fn addr_v6(port: u16) -> SocketAddr {
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0))
    }

    /// Helper: build an AnnouncePeer message.
    fn make_announce(
        node_id: Id20,
        info_hash: Id20,
        token: &[u8],
        port: u16,
        implied_port: u8,
    ) -> AnnouncePeer<ByteBufOwned> {
        AnnouncePeer {
            id: node_id,
            implied_port,
            info_hash,
            port,
            token: ByteBufOwned::from(token),
        }
    }

    // -----------------------------------------------------------------------
    // test_new_peer_store
    // -----------------------------------------------------------------------
    #[test]
    fn test_new_peer_store() {
        let id = id_from_byte(0x42);
        let store = PeerStore::new(id);
        assert_eq!(store.self_id, id);
        assert_eq!(store.max_remembered_tokens, 1000);
        assert_eq!(store.max_remembered_peers, 1000);
        assert!(store.tokens.read().is_empty());
        assert!(store.peers.is_empty());
        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    // -----------------------------------------------------------------------
    // test_gen_token_and_store_peer
    // -----------------------------------------------------------------------
    #[test]
    fn test_gen_token_and_store_peer() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id = id_from_byte(0x01);
        let info_hash = id_from_byte(0x02);
        let addr = addr_v4(6881);

        // Generate a token for this node.
        let token = store.gen_token_for(node_id, addr);
        assert_eq!(store.tokens.read().len(), 1);

        // Store a peer using that token.
        let announce = make_announce(node_id, info_hash, &token, 6882, 0);
        assert!(store.store_peer(&announce, addr));

        // Verify the peer was stored.
        let peers = store.get_for_info_hash(info_hash, Want::Both);
        assert_eq!(peers.len(), 1);
        // The port should be from the announce (implied_port=0 means use announce.port).
        assert_eq!(peers[0].0.port(), 6882);
    }

    // -----------------------------------------------------------------------
    // test_store_peer_rejects_invalid_token
    // -----------------------------------------------------------------------
    #[test]
    fn test_store_peer_rejects_invalid_token() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id = id_from_byte(0x01);
        let info_hash = id_from_byte(0x02);
        let addr = addr_v4(6881);

        // Generate a valid token but use a different one.
        let _valid_token = store.gen_token_for(node_id, addr);
        let bad_token = [0xDE, 0xAD, 0xBE, 0xEF];

        let announce = make_announce(node_id, info_hash, &bad_token, 6882, 0);
        assert!(!store.store_peer(&announce, addr));

        // Nothing should be stored.
        let peers = store.get_for_info_hash(info_hash, Want::Both);
        assert!(peers.is_empty());
    }

    // -----------------------------------------------------------------------
    // test_store_peer_rejects_far_info_hash
    // -----------------------------------------------------------------------
    #[test]
    fn test_store_peer_rejects_far_info_hash() {
        let self_id = id_from_byte(0x00);
        // Use the default PeerStore (which has max_distance = 00000fffff...).
        let store = PeerStore::new(self_id);

        let node_id = id_from_byte(0x01);
        // info_hash that is very far from self_id (high bits set).
        let info_hash = id_from_byte(0xFF);
        let addr = addr_v4(6881);

        let token = store.gen_token_for(node_id, addr);
        let announce = make_announce(node_id, info_hash, &token, 6882, 0);
        // Should be rejected because distance(info_hash, self_id) > max_distance.
        assert!(!store.store_peer(&announce, addr));
    }

    // -----------------------------------------------------------------------
    // test_store_peer_updates_existing
    // -----------------------------------------------------------------------
    #[test]
    fn test_store_peer_updates_existing() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id = id_from_byte(0x01);
        let info_hash = id_from_byte(0x02);
        let addr = addr_v4(6881);

        let token = store.gen_token_for(node_id, addr);

        // Store a peer (implied_port=1 means use the addr's port, not announce.port).
        let announce = make_announce(node_id, info_hash, &token, 6882, 1);
        assert!(store.store_peer(&announce, addr));
        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Store the same peer again — should update, not add a new one.
        assert!(store.store_peer(&announce, addr));
        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 1);

        let peers = store.get_for_info_hash(info_hash, Want::Both);
        assert_eq!(peers.len(), 1);
    }

    // -----------------------------------------------------------------------
    // test_store_peer_capacity_limit
    // -----------------------------------------------------------------------
    #[test]
    fn test_store_peer_capacity_limit() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 1000, 3);

        let info_hash = id_from_byte(0x02);

        // Store 3 peers (the max).
        for i in 0u16..3 {
            let node_id = {
                let mut d = [0u8; 20];
                d[0] = (i + 10) as u8;
                Id20::new(d)
            };
            let addr = addr_v4(7000 + i);
            let token = store.gen_token_for(node_id, addr);
            let announce = make_announce(node_id, info_hash, &token, 8000 + i, 0);
            assert!(store.store_peer(&announce, addr));
        }
        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 3);

        // The 4th peer should be rejected.
        let node_id = id_from_byte(0x20);
        let addr = addr_v4(9000);
        let token = store.gen_token_for(node_id, addr);
        let announce = make_announce(node_id, info_hash, &token, 9001, 0);
        assert!(!store.store_peer(&announce, addr));

        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    // -----------------------------------------------------------------------
    // test_get_for_info_hash
    // -----------------------------------------------------------------------
    #[test]
    fn test_get_for_info_hash() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let info_hash_a = id_from_byte(0x10);
        let info_hash_b = id_from_byte(0x20);

        // Add a peer to info_hash_a.
        let node = id_from_byte(0x01);
        let addr = addr_v4(5000);
        let token = store.gen_token_for(node, addr);
        let ann = make_announce(node, info_hash_a, &token, 5001, 0);
        assert!(store.store_peer(&ann, addr));

        // info_hash_a should have 1 peer.
        assert_eq!(store.get_for_info_hash(info_hash_a, Want::Both).len(), 1);
        // info_hash_b should have 0 peers.
        assert!(store.get_for_info_hash(info_hash_b, Want::Both).is_empty());
    }

    // -----------------------------------------------------------------------
    // test_get_for_info_hash_want_filter
    // -----------------------------------------------------------------------
    #[test]
    fn test_get_for_info_hash_want_filter() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let info_hash = id_from_byte(0x05);

        // Add a v4 peer.
        let node_v4 = id_from_byte(0x01);
        let addr_v4_ = addr_v4(5000);
        let token_v4 = store.gen_token_for(node_v4, addr_v4_);
        let ann_v4 = make_announce(node_v4, info_hash, &token_v4, 5001, 0);
        assert!(store.store_peer(&ann_v4, addr_v4_));

        // Add a v6 peer.
        let node_v6 = id_from_byte(0x02);
        let addr_v6_ = addr_v6(6000);
        let token_v6 = store.gen_token_for(node_v6, addr_v6_);
        let ann_v6 = make_announce(node_v6, info_hash, &token_v6, 6001, 0);
        assert!(store.store_peer(&ann_v6, addr_v6_));

        // Want::Both should return 2.
        assert_eq!(store.get_for_info_hash(info_hash, Want::Both).len(), 2);
        // Want::V4 should return only the v4 peer.
        let v4_peers = store.get_for_info_hash(info_hash, Want::V4);
        assert_eq!(v4_peers.len(), 1);
        assert!(v4_peers[0].0.is_ipv4());
        // Want::V6 should return only the v6 peer.
        let v6_peers = store.get_for_info_hash(info_hash, Want::V6);
        assert_eq!(v6_peers.len(), 1);
        assert!(v6_peers[0].0.is_ipv6());
    }

    // -----------------------------------------------------------------------
    // test_garbage_collect_removes_stale_peers
    // -----------------------------------------------------------------------
    #[test]
    fn test_garbage_collect_removes_stale_peers() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let info_hash = id_from_byte(0x05);

        // Manually insert a peer with a stale timestamp.
        let stale_time = Utc::now() - TimeDelta::minutes(20);
        store.peers.insert(
            info_hash,
            vec![StoredPeer {
                addr: addr_v4(5000),
                time: stale_time,
            }],
        );
        store
            .peers_len
            .store(1, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(store.get_for_info_hash(info_hash, Want::Both).len(), 1);

        // GC should remove this stale peer.
        store.garbage_collect_peers();

        assert!(store.get_for_info_hash(info_hash, Want::Both).is_empty());
        assert_eq!(store.peers_len.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    // -----------------------------------------------------------------------
    // test_garbage_collect_removes_stale_tokens
    // -----------------------------------------------------------------------
    #[test]
    fn test_garbage_collect_removes_stale_tokens() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        // Manually insert a stale token.
        {
            let mut tokens = store.tokens.write();
            tokens.push_back(StoredToken {
                token: [1, 2, 3, 4],
                node_id: id_from_byte(0x01),
                addr: addr_v4(5000),
                time: Utc::now() - TimeDelta::minutes(15), // older than TOKEN_TTL (10 min)
            });
        }
        assert_eq!(store.tokens.read().len(), 1);

        // Also insert a fresh token.
        let _ = store.gen_token_for(id_from_byte(0x02), addr_v4(5001));
        assert_eq!(store.tokens.read().len(), 2);

        store.garbage_collect_peers();

        // The stale token should be removed, the fresh one kept.
        assert_eq!(store.tokens.read().len(), 1);
    }

    // -----------------------------------------------------------------------
    // test_garbage_collect_per_hash_cap
    // -----------------------------------------------------------------------
    #[test]
    fn test_garbage_collect_per_hash_cap() {
        let self_id = id_from_byte(0x00);
        // Use large global cap so it doesn't interfere.
        let store = PeerStore::with_limits(self_id, 1000, 10000);

        let info_hash = id_from_byte(0x05);

        // Insert 150 peers for a single info_hash (more than MAX_PEERS_PER_INFO_HASH = 100).
        let mut peers = Vec::new();
        for i in 0u16..150 {
            peers.push(StoredPeer {
                addr: addr_v4(5000 + i),
                time: Utc::now() - TimeDelta::seconds(i as i64), // newer peers first
            });
        }
        store.peers.insert(info_hash, peers);
        store
            .peers_len
            .store(150, std::sync::atomic::Ordering::SeqCst);

        store.garbage_collect_peers();

        // Should be truncated to MAX_PEERS_PER_INFO_HASH (100).
        let remaining = store.peers.get(&info_hash).unwrap().len();
        assert_eq!(remaining, PeerStore::MAX_PEERS_PER_INFO_HASH);
        assert_eq!(
            store.peers_len.load(std::sync::atomic::Ordering::SeqCst),
            PeerStore::MAX_PEERS_PER_INFO_HASH as u32
        );
    }

    // -----------------------------------------------------------------------
    // test_garbage_collect_global_cap
    // -----------------------------------------------------------------------
    #[test]
    fn test_garbage_collect_global_cap() {
        let self_id = id_from_byte(0x00);
        // Global cap = 5, but insert more.
        let store = PeerStore::with_limits(self_id, 1000, 5);

        // Insert peers across different info_hashes.
        for i in 0u8..10 {
            let info_hash = {
                let mut d = [0u8; 20];
                d[0] = i;
                Id20::new(d)
            };
            store.peers.insert(
                info_hash,
                vec![StoredPeer {
                    addr: addr_v4(5000 + i as u16),
                    time: Utc::now() - TimeDelta::seconds(i as i64),
                }],
            );
        }
        store
            .peers_len
            .store(10, std::sync::atomic::Ordering::SeqCst);

        store.garbage_collect_peers();

        let final_count = store.peers_len.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            final_count <= 5,
            "expected at most 5 peers after GC, got {final_count}"
        );
    }

    // -----------------------------------------------------------------------
    // test_garbage_collect_empty_hashes_removed
    // -----------------------------------------------------------------------
    #[test]
    fn test_garbage_collect_empty_hashes_removed() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let info_hash = id_from_byte(0x05);

        // Insert a single stale peer.
        store.peers.insert(
            info_hash,
            vec![StoredPeer {
                addr: addr_v4(5000),
                time: Utc::now() - TimeDelta::minutes(20),
            }],
        );
        store
            .peers_len
            .store(1, std::sync::atomic::Ordering::SeqCst);

        assert!(store.peers.contains_key(&info_hash));

        store.garbage_collect_peers();

        // The info_hash entry itself should be removed.
        assert!(!store.peers.contains_key(&info_hash));
    }

    // -----------------------------------------------------------------------
    // test_implied_port_uses_sender_addr_port
    // -----------------------------------------------------------------------
    #[test]
    fn test_implied_port_uses_sender_addr_port() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id = id_from_byte(0x01);
        let info_hash = id_from_byte(0x02);
        let addr = addr_v4(9999);

        let token = store.gen_token_for(node_id, addr);

        // implied_port=1 means the peer's listen port is the same as the UDP source port.
        let announce = make_announce(node_id, info_hash, &token, 1234, 1);
        assert!(store.store_peer(&announce, addr));

        let peers = store.get_for_info_hash(info_hash, Want::Both);
        assert_eq!(peers.len(), 1);
        // Should use the sender's port (9999), not the announce port (1234).
        assert_eq!(peers[0].0.port(), 9999);
    }

    // -----------------------------------------------------------------------
    // test_token_requires_matching_addr
    // -----------------------------------------------------------------------
    #[test]
    fn test_token_requires_matching_addr() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id = id_from_byte(0x01);
        let info_hash = id_from_byte(0x02);
        let addr_a = addr_v4(6881);
        let addr_b = addr_v4(6882);

        // Token generated for addr_a.
        let token = store.gen_token_for(node_id, addr_a);

        // Try to use it from addr_b — should fail.
        let announce = make_announce(node_id, info_hash, &token, 6882, 0);
        assert!(!store.store_peer(&announce, addr_b));
    }

    // -----------------------------------------------------------------------
    // test_token_requires_matching_node_id
    // -----------------------------------------------------------------------
    #[test]
    fn test_token_requires_matching_node_id() {
        let self_id = id_from_byte(0x00);
        let store = PeerStore::with_limits(self_id, 100, 100);

        let node_id_a = id_from_byte(0x01);
        let node_id_b = id_from_byte(0x02);
        let info_hash = id_from_byte(0x03);
        let addr = addr_v4(6881);

        // Token generated for node_id_a.
        let token = store.gen_token_for(node_id_a, addr);

        // Announce claims to be node_id_b — should fail.
        let announce = make_announce(node_id_b, info_hash, &token, 6882, 0);
        assert!(!store.store_peer(&announce, addr));
    }
}
