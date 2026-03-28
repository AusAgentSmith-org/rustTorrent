use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use parking_lot::Mutex;

#[derive(Clone, Copy)]
struct ProgressSnapshot {
    progress_bytes: u64,
    instant: Instant,
}

/// Estimates download/upload speed in a sliding time window.
pub struct SpeedEstimator {
    latest_per_second_snapshots: Mutex<VecDeque<ProgressSnapshot>>,
    bytes_per_second: AtomicU64,
    time_remaining_millis: AtomicU64,
}

impl Default for SpeedEstimator {
    fn default() -> Self {
        Self::new(5)
    }
}

impl SpeedEstimator {
    pub fn new(window_seconds: usize) -> Self {
        assert!(window_seconds > 1);
        Self {
            latest_per_second_snapshots: Mutex::new(VecDeque::with_capacity(window_seconds)),
            bytes_per_second: Default::default(),
            time_remaining_millis: Default::default(),
        }
    }

    pub fn time_remaining(&self) -> Option<Duration> {
        let tr = self.time_remaining_millis.load(Ordering::Relaxed);
        if tr == 0 {
            return None;
        }
        Some(Duration::from_millis(tr))
    }

    pub fn bps(&self) -> u64 {
        self.bytes_per_second.load(Ordering::Relaxed)
    }

    pub fn mbps(&self) -> f64 {
        self.bps() as f64 / 1024f64 / 1024f64
    }

    pub fn add_snapshot(
        &self,
        progress_bytes: u64,
        remaining_bytes: Option<u64>,
        instant: Instant,
    ) {
        let first = {
            let mut g = self.latest_per_second_snapshots.lock();

            let current = ProgressSnapshot {
                progress_bytes,
                instant,
            };

            if g.is_empty() {
                g.push_back(current);
                return;
            } else if g.len() < g.capacity() {
                g.push_back(current);
                g.front().copied().unwrap()
            } else {
                let first = g.pop_front().unwrap();
                g.push_back(current);
                first
            }
        };

        let downloaded_bytes_diff = progress_bytes - first.progress_bytes;
        let elapsed = instant - first.instant;
        let bps = downloaded_bytes_diff as f64 / elapsed.as_secs_f64();

        let time_remaining_millis_rounded: u64 = if downloaded_bytes_diff > 0 {
            let time_remaining_secs = remaining_bytes.unwrap_or_default() as f64 / bps;
            (time_remaining_secs * 1000f64) as u64
        } else {
            0
        };
        self.time_remaining_millis
            .store(time_remaining_millis_rounded, Ordering::Relaxed);
        self.bytes_per_second.store(bps as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn test_speed_estimator_initial_zero() {
        let estimator = SpeedEstimator::default();
        assert_eq!(estimator.bps(), 0);
        assert_eq!(estimator.mbps(), 0.0);
        assert!(estimator.time_remaining().is_none());
    }

    #[test]
    fn test_speed_estimator_single_snapshot_no_speed() {
        let estimator = SpeedEstimator::default();
        let now = Instant::now();
        // First snapshot just initializes; speed remains zero.
        estimator.add_snapshot(1000, None, now);
        assert_eq!(estimator.bps(), 0);
    }

    #[test]
    fn test_speed_estimator_calculates_speed() {
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        // First snapshot: 0 bytes downloaded
        estimator.add_snapshot(0, None, start);

        // Second snapshot: 10_000 bytes downloaded, 1 second later
        estimator.add_snapshot(10_000, None, start + Duration::from_secs(1));

        // Speed should be approximately 10_000 bytes/sec
        let bps = estimator.bps();
        assert!(
            (9_000..=11_000).contains(&bps),
            "expected ~10000 bps, got {bps}"
        );
    }

    #[test]
    fn test_speed_estimator_time_remaining() {
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        estimator.add_snapshot(0, Some(20_000), start);
        estimator.add_snapshot(10_000, Some(10_000), start + Duration::from_secs(1));

        // We downloaded 10_000 bytes in 1 second => 10_000 bps
        // Remaining = 10_000 bytes => ~1 second remaining
        let remaining = estimator.time_remaining();
        assert!(remaining.is_some());
        let remaining_ms = remaining.unwrap().as_millis();
        assert!(
            (800..=1200).contains(&remaining_ms),
            "expected ~1000ms remaining, got {remaining_ms}ms"
        );
    }

    #[test]
    fn test_speed_estimator_no_progress_zero_remaining() {
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        estimator.add_snapshot(0, Some(10_000), start);
        // Same bytes, no progress
        estimator.add_snapshot(0, Some(10_000), start + Duration::from_secs(1));

        // No progress => time_remaining should be None (stored as 0)
        assert!(estimator.time_remaining().is_none());
        assert_eq!(estimator.bps(), 0);
    }

    #[test]
    fn test_speed_estimator_multiple_updates() {
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        // Feed 5 snapshots (window=3, so oldest get evicted)
        estimator.add_snapshot(0, None, start);
        estimator.add_snapshot(1_000, None, start + Duration::from_secs(1));
        estimator.add_snapshot(3_000, None, start + Duration::from_secs(2));
        estimator.add_snapshot(6_000, None, start + Duration::from_secs(3));
        estimator.add_snapshot(10_000, None, start + Duration::from_secs(4));

        // Window size is 3, so when we have 5 snapshots, the estimator
        // compares the newest (10_000 at t=4) with the popped first.
        // At snapshot 5: popped (1000, t=1), speed = (10000-1000)/(4-1) = 3000 bps
        let bps = estimator.bps();
        assert!(
            (2500..=3500).contains(&bps),
            "expected ~3000 bps, got {bps}"
        );
    }

    #[test]
    fn test_speed_estimator_mbps() {
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        estimator.add_snapshot(0, None, start);
        // 1 MiB in 1 second
        let one_mib = 1024 * 1024;
        estimator.add_snapshot(one_mib, None, start + Duration::from_secs(1));

        let mbps = estimator.mbps();
        assert!((0.9..1.1).contains(&mbps), "expected ~1.0 MBps, got {mbps}");
    }

    #[test]
    fn test_speed_estimator_window_eviction() {
        // Window of 3: can hold 3 snapshots.
        // Adding a 4th evicts the first.
        let estimator = SpeedEstimator::new(3);
        let start = Instant::now();

        // Fill the window
        estimator.add_snapshot(0, None, start);
        estimator.add_snapshot(100, None, start + Duration::from_secs(1));
        estimator.add_snapshot(200, None, start + Duration::from_secs(2));

        // At this point, first=(0,t=0), speed = (200-0)/2 = 100 bps
        let bps = estimator.bps();
        assert!((90..=110).contains(&bps), "expected ~100 bps, got {bps}");

        // Add a 4th: evicts snapshot(0, t=0).
        // The popped `first` is (0, t=0).
        // Speed = (1200-0)/(3-0) = 400 bps
        // Now window has: (100,t=1), (200,t=2), (1200,t=3)
        estimator.add_snapshot(1200, None, start + Duration::from_secs(3));
        let bps = estimator.bps();
        assert!((380..=420).contains(&bps), "expected ~400 bps, got {bps}");
    }
}
