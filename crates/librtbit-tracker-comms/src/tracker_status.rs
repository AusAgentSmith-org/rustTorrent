use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde_derive::Serialize;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Announce state of a single tracker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerAnnounceState {
    /// Tracker configured but no announce attempted yet.
    NotContacted,
    /// An announce is in flight (or being retried).
    Updating,
    /// Last announce succeeded.
    Working,
    /// Last announce failed.
    Error,
    /// Tracker skipped (unsupported scheme, proxy mode, ...).
    Disabled,
}

/// Live status of a single tracker for one torrent.
#[derive(Clone, Debug, Serialize)]
pub struct TrackerStatus {
    pub url: String,
    pub state: TrackerAnnounceState,
    /// Seeder count ("complete") reported by the last successful announce.
    pub seeders: Option<u32>,
    /// Leecher count ("incomplete") reported by the last successful announce.
    pub leechers: Option<u32>,
    /// Number of peer addresses returned by the last successful announce.
    pub peers_returned: Option<u32>,
    /// Unix timestamp of the last successful announce.
    pub last_announce_unix: Option<u64>,
    /// Announce interval (seconds) the tracker asked for.
    pub interval_secs: Option<u64>,
    pub last_error: Option<String>,
}

impl TrackerStatus {
    fn new(url: String) -> Self {
        Self {
            url,
            state: TrackerAnnounceState::NotContacted,
            seeders: None,
            leechers: None,
            peers_returned: None,
            last_announce_unix: None,
            interval_secs: None,
            last_error: None,
        }
    }
}

/// Per-torrent registry of tracker announce statuses, updated by the
/// announce loops and readable from the HTTP API.
#[derive(Default)]
pub struct TrackerStatusRegistry {
    inner: RwLock<HashMap<String, TrackerStatus>>,
}

impl TrackerStatusRegistry {
    fn with_entry(&self, url: &str, f: impl FnOnce(&mut TrackerStatus)) {
        let mut g = self.inner.write();
        let entry = g
            .entry(url.to_owned())
            .or_insert_with(|| TrackerStatus::new(url.to_owned()));
        f(entry);
    }

    /// Register a tracker without changing its state.
    pub fn ensure(&self, url: &str) {
        self.with_entry(url, |_| {});
    }

    pub fn record_updating(&self, url: &str) {
        self.with_entry(url, |e| {
            if e.state == TrackerAnnounceState::NotContacted {
                e.state = TrackerAnnounceState::Updating;
            }
        });
    }

    pub fn record_success(
        &self,
        url: &str,
        seeders: Option<u32>,
        leechers: Option<u32>,
        peers_returned: u32,
        interval_secs: u64,
    ) {
        self.with_entry(url, |e| {
            e.state = TrackerAnnounceState::Working;
            e.seeders = seeders.or(e.seeders);
            e.leechers = leechers.or(e.leechers);
            e.peers_returned = Some(peers_returned);
            e.last_announce_unix = Some(now_unix());
            e.interval_secs = Some(interval_secs);
            e.last_error = None;
        });
    }

    pub fn record_error(&self, url: &str, error: &str) {
        self.with_entry(url, |e| {
            e.state = TrackerAnnounceState::Error;
            e.last_error = Some(error.to_owned());
        });
    }

    pub fn record_disabled(&self, url: &str, reason: &str) {
        self.with_entry(url, |e| {
            e.state = TrackerAnnounceState::Disabled;
            e.last_error = Some(reason.to_owned());
        });
    }

    /// Sorted snapshot of all known trackers.
    pub fn snapshot(&self) -> Vec<TrackerStatus> {
        let mut v: Vec<TrackerStatus> = self.inner.read().values().cloned().collect();
        v.sort_by(|a, b| a.url.cmp(&b.url));
        v
    }
}
