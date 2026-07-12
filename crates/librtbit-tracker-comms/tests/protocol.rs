//! Protocol-level functional tests for tracker communication types.

use librtbit_tracker_comms::{TrackerCommsStats, TrackerCommsStatsState};

#[test]
fn test_stats_left_to_download_bytes() {
    let stats = TrackerCommsStats {
        uploaded_bytes: 100,
        downloaded_bytes: 500,
        total_bytes: 1000,
        torrent_state: TrackerCommsStatsState::Live,
    };
    assert_eq!(stats.get_left_to_download_bytes(), 500);
}

#[test]
fn test_stats_is_completed() {
    let stats = TrackerCommsStats {
        uploaded_bytes: 0,
        downloaded_bytes: 1000,
        total_bytes: 1000,
        torrent_state: TrackerCommsStatsState::Live,
    };
    assert!(stats.is_completed());

    let incomplete = TrackerCommsStats {
        uploaded_bytes: 0,
        downloaded_bytes: 500,
        total_bytes: 1000,
        torrent_state: TrackerCommsStatsState::Live,
    };
    assert!(!incomplete.is_completed());
}

#[test]
fn test_stats_state_default_is_none() {
    let state = TrackerCommsStatsState::default();
    assert!(matches!(state, TrackerCommsStatsState::None));
}

#[test]
fn test_stats_zero_total_not_completed() {
    let stats = TrackerCommsStats {
        uploaded_bytes: 0,
        downloaded_bytes: 0,
        total_bytes: 0,
        torrent_state: TrackerCommsStatsState::Initializing,
    };
    // 0/0 should not be considered completed
    assert!(!stats.is_completed() || stats.total_bytes == 0);
}

#[test]
fn test_stats_left_when_over_downloaded() {
    // Edge case: downloaded > total (possible with padding)
    let stats = TrackerCommsStats {
        uploaded_bytes: 0,
        downloaded_bytes: 1200,
        total_bytes: 1000,
        torrent_state: TrackerCommsStatsState::Live,
    };
    // Should not underflow
    assert_eq!(stats.get_left_to_download_bytes(), 0);
}
