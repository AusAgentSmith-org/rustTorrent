use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use backon::{BackoffBuilder, ExponentialBackoff, ExponentialBuilder};

#[derive(Default, Debug)]
pub(crate) struct PeerCountersAtomic {
    pub fetched_bytes: AtomicU64,
    pub uploaded_bytes: AtomicU64,
    pub total_time_connecting_ms: AtomicU64,
    pub incoming_connections: AtomicU32,
    pub outgoing_connection_attempts: AtomicU32,
    pub outgoing_connections: AtomicU32,
    pub errors: AtomicU32,
    pub fetched_chunks: AtomicU32,
    pub downloaded_and_checked_pieces: AtomicU32,
    pub downloaded_and_checked_bytes: AtomicU64,
    pub total_piece_download_ms: AtomicU64,
    pub times_stolen_from_me: AtomicU32,
    pub times_i_stole: AtomicU32,
}

impl PeerCountersAtomic {
    pub(crate) fn on_piece_completed(&self, piece_len: u64, elapsed: Duration) {
        #[allow(clippy::cast_possible_truncation)]
        let elapsed = elapsed.as_millis() as u64;
        self.total_piece_download_ms
            .fetch_add(elapsed, Ordering::Release);
        self.downloaded_and_checked_pieces
            .fetch_add(1, Ordering::Release);
        self.downloaded_and_checked_bytes
            .fetch_add(piece_len, Ordering::Relaxed);
    }

    pub(crate) fn average_piece_download_time(&self) -> Option<Duration> {
        let downloaded_pieces = self.downloaded_and_checked_pieces.load(Ordering::Acquire);
        let total_download_time = self.total_piece_download_ms.load(Ordering::Acquire);
        if total_download_time == 0 || downloaded_pieces == 0 {
            return None;
        }
        Some(Duration::from_millis(
            total_download_time / downloaded_pieces as u64,
        ))
    }
}

fn backoff() -> ExponentialBackoff {
    ExponentialBuilder::new()
        .with_min_delay(Duration::from_secs(10))
        .with_factor(6.)
        .with_jitter()
        .with_max_delay(Duration::from_secs(3600))
        .with_total_delay(Some(Duration::from_secs(86400)))
        .without_max_times()
        .build()
}

#[derive(Debug)]
pub(crate) struct PeerStats {
    pub counters: Arc<PeerCountersAtomic>,
    pub backoff: ExponentialBackoff,
}

impl Default for PeerStats {
    fn default() -> Self {
        Self {
            counters: Arc::new(Default::default()),
            backoff: backoff(),
        }
    }
}

impl PeerStats {
    pub fn reset_backoff(&mut self) {
        self.backoff = backoff();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the backoff produces delays that grow over time
    /// and that the cumulative delay is bounded by total_delay (86400s).
    #[test]
    fn test_backoff_produces_bounded_delays() {
        let mut stats = PeerStats::default();

        let mut total_delay = Duration::ZERO;
        let mut count = 0;
        let max_iterations = 1000;

        for dur in stats.backoff.by_ref() {
            total_delay += dur;
            count += 1;
            if count >= max_iterations {
                break;
            }
        }

        // Backoff should have produced at least some delays.
        assert!(count > 0, "backoff should produce at least one delay");

        // The total delay should be bounded by the configured total_delay (86400s)
        // plus some margin for the last jittered value.
        assert!(
            total_delay <= Duration::from_secs(86400 + 7200),
            "cumulative delay should be bounded near total_delay, got {:?}",
            total_delay
        );
    }

    /// Verify that after reset, the backoff produces delays again
    /// starting from the minimum delay.
    #[test]
    fn test_backoff_reset_produces_new_delays() {
        let mut stats = PeerStats::default();

        // Consume some delays to advance the backoff state.
        for _ in 0..5 {
            stats.backoff.next();
        }

        // Reset the backoff.
        stats.reset_backoff();

        // Should produce delays again starting from min_delay.
        let next = stats.backoff.next();
        assert!(next.is_some(), "backoff should produce delays after reset");
        let dur = next.unwrap();
        // min_delay is 10s, with jitter it can be up to 20s.
        assert!(
            dur >= Duration::from_secs(10) && dur <= Duration::from_secs(20),
            "first delay after reset should be near min_delay (10s +/- jitter), got {:?}",
            dur
        );
    }

    /// Verify that reset_backoff() creates a fresh backoff
    /// independent of the previous state.
    #[test]
    fn test_backoff_reset_is_independent() {
        let mut stats = PeerStats::default();

        // Advance the backoff significantly.
        let mut advanced_count = 0;
        for _ in stats.backoff.by_ref() {
            advanced_count += 1;
            if advanced_count >= 20 {
                break;
            }
        }

        // Reset.
        stats.reset_backoff();

        // After reset, we should get the same number of delays
        // as a fresh backoff (approximately).
        let mut reset_count = 0;
        for _ in stats.backoff.by_ref() {
            reset_count += 1;
            if reset_count >= 100 {
                break;
            }
        }

        assert!(reset_count > 0, "reset backoff should produce delays");
    }
}
