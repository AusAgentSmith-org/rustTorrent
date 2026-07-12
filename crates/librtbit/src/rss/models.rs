use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A discovered item from an RSS feed, persisted in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssItem {
    /// Feed entry ID (from the RSS feed)
    pub id: String,
    /// Name of the feed this came from
    pub feed_name: String,
    /// Title of the entry
    pub title: String,
    /// Torrent download URL (torrent file or magnet link)
    pub url: Option<String>,
    /// When the entry was published (from feed)
    pub published_at: Option<DateTime<Utc>>,
    /// When we first saw this item
    pub first_seen_at: DateTime<Utc>,
    /// Whether this item has been downloaded
    pub downloaded: bool,
    /// When it was downloaded (if applicable)
    pub downloaded_at: Option<DateTime<Utc>>,
    /// Category used when downloaded
    pub category: Option<String>,
    /// Size in bytes (if available from feed)
    pub size_bytes: u64,
}

/// A download rule that automatically enqueues matching RSS feed items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssRule {
    /// Unique rule identifier
    pub id: String,
    /// Human-readable name for the rule
    pub name: String,
    /// Which feed(s) this rule applies to (one or more feed names)
    pub feed_names: Vec<String>,
    /// Category to assign to downloaded torrents
    pub category: Option<String>,
    /// Download priority (0=low, 1=normal, 2=high, 3=force)
    pub priority: i32,
    /// Regex to match against feed item titles
    pub match_regex: String,
    /// Whether this rule is active
    pub enabled: bool,
}
