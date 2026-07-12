use serde::{Deserialize, Serialize};

/// RSS feed configuration for automatic torrent downloading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeedConfig {
    /// Display name for the feed
    pub name: String,
    /// Feed URL (RSS 2.0 or Atom)
    pub url: String,
    /// How often to poll, in seconds (default 900 = 15 minutes)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Category to assign to downloaded torrents
    #[serde(default)]
    pub category: Option<String>,
    /// Regex pattern to filter feed entries by title
    #[serde(default)]
    pub filter_regex: Option<String>,
    /// Whether this feed is active
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Auto-download all items from this feed (no rules needed).
    /// Ignored when filter_regex is set (use download rules instead).
    #[serde(default)]
    pub auto_download: bool,
}

fn default_poll_interval() -> u64 {
    900
}

fn default_true() -> bool {
    true
}
