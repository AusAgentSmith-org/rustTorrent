use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, params};
use tracing::info;

use super::models::{RssItem, RssRule};

/// Database handle for RSS feed item and rule persistence.
pub struct RssDatabase {
    conn: Connection,
}

impl RssDatabase {
    /// Open (or create) the RSS database at the given path.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );",
        )?;

        let version: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < 1 {
            info!("Applying RSS database migration v1");
            self.conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_items (
                    id TEXT PRIMARY KEY,
                    feed_name TEXT NOT NULL,
                    title TEXT NOT NULL,
                    url TEXT,
                    published_at TEXT,
                    first_seen_at TEXT NOT NULL,
                    downloaded INTEGER NOT NULL DEFAULT 0,
                    downloaded_at TEXT,
                    category TEXT,
                    size_bytes INTEGER DEFAULT 0
                );

                CREATE INDEX IF NOT EXISTS idx_rss_items_feed ON rss_items(feed_name);
                CREATE INDEX IF NOT EXISTS idx_rss_items_seen ON rss_items(first_seen_at DESC);

                CREATE TABLE IF NOT EXISTS rss_rules (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    feed_name TEXT NOT NULL,
                    category TEXT,
                    priority INTEGER NOT NULL DEFAULT 1,
                    match_regex TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1
                );

                INSERT INTO schema_version (version) VALUES (1);
                ",
            )?;
        }

        // Store RSS feed configs in the database so they persist across restarts
        // without needing a config file.
        if version < 2 {
            info!("Applying RSS database migration v2");
            self.conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS rss_feeds (
                    name TEXT PRIMARY KEY,
                    url TEXT NOT NULL,
                    poll_interval_secs INTEGER NOT NULL DEFAULT 900,
                    category TEXT,
                    filter_regex TEXT,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    auto_download INTEGER NOT NULL DEFAULT 0
                );

                UPDATE schema_version SET version = 2;
                ",
            )?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // RSS feed config operations (stored in DB for API-managed feeds)
    // -----------------------------------------------------------------------

    /// List all RSS feed configs from the database.
    pub fn feed_list(&self) -> anyhow::Result<Vec<super::config::RssFeedConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, url, poll_interval_secs, category, filter_regex, enabled, auto_download
             FROM rss_feeds ORDER BY name ASC",
        )?;
        let feeds = stmt
            .query_map([], |row| {
                Ok(super::config::RssFeedConfig {
                    name: row.get(0)?,
                    url: row.get(1)?,
                    poll_interval_secs: row.get::<_, i64>(2)? as u64,
                    category: row.get(3)?,
                    filter_regex: row.get(4)?,
                    enabled: row.get::<_, i32>(5)? != 0,
                    auto_download: row.get::<_, i32>(6)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(feeds)
    }

    /// Insert or replace an RSS feed config.
    pub fn feed_upsert(&self, feed: &super::config::RssFeedConfig) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO rss_feeds (name, url, poll_interval_secs, category, filter_regex, enabled, auto_download)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                feed.name,
                feed.url,
                feed.poll_interval_secs as i64,
                feed.category,
                feed.filter_regex,
                feed.enabled as i32,
                feed.auto_download as i32,
            ],
        )?;
        Ok(())
    }

    /// Delete an RSS feed config by name.
    pub fn feed_delete(&self, name: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM rss_feeds WHERE name = ?1", params![name])?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // RSS item operations
    // -----------------------------------------------------------------------

    /// Upsert an RSS feed item (insert or ignore if already exists).
    pub fn rss_item_upsert(&self, item: &RssItem) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO rss_items (id, feed_name, title, url, published_at,
             first_seen_at, downloaded, downloaded_at, category, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                item.id,
                item.feed_name,
                item.title,
                item.url,
                item.published_at.map(|d| d.to_rfc3339()),
                item.first_seen_at.to_rfc3339(),
                item.downloaded as i32,
                item.downloaded_at.map(|d| d.to_rfc3339()),
                item.category,
                item.size_bytes as i64,
            ],
        )?;
        Ok(())
    }

    /// Check if an RSS item ID already exists.
    pub fn rss_item_exists(&self, id: &str) -> anyhow::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM rss_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// List RSS items, optionally filtered by feed name.
    pub fn rss_items_list(
        &self,
        feed_name: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<RssItem>> {
        if let Some(name) = feed_name {
            let mut stmt = self.conn.prepare(
                "SELECT id, feed_name, title, url, published_at, first_seen_at,
                 downloaded, downloaded_at, category, size_bytes
                 FROM rss_items WHERE feed_name = ?1
                 ORDER BY first_seen_at DESC LIMIT ?2",
            )?;
            let items = stmt
                .query_map(params![name, limit as i64], |row| self.map_rss_item(row))?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(items);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, feed_name, title, url, published_at, first_seen_at,
             downloaded, downloaded_at, category, size_bytes
             FROM rss_items ORDER BY first_seen_at DESC LIMIT ?1",
        )?;
        let items = stmt
            .query_map(params![limit as i64], |row| self.map_rss_item(row))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    /// Get a single RSS item by ID.
    pub fn rss_item_get(&self, id: &str) -> anyhow::Result<Option<RssItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, feed_name, title, url, published_at, first_seen_at,
             downloaded, downloaded_at, category, size_bytes
             FROM rss_items WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], |row| self.map_rss_item(row));
        match result {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Mark an RSS item as downloaded.
    pub fn rss_item_mark_downloaded(&self, id: &str, category: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE rss_items SET downloaded = 1, downloaded_at = ?2, category = ?3 WHERE id = ?1",
            params![id, Utc::now().to_rfc3339(), category],
        )?;
        Ok(())
    }

    /// Count total RSS items.
    pub fn rss_item_count(&self) -> anyhow::Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM rss_items", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Prune RSS items to keep only the N most recent.
    pub fn rss_items_prune(&self, keep: usize) -> anyhow::Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM rss_items WHERE id NOT IN (
                SELECT id FROM rss_items ORDER BY first_seen_at DESC LIMIT ?1
            )",
            params![keep as i64],
        )?;
        Ok(deleted)
    }

    fn map_rss_item(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<RssItem> {
        Ok(RssItem {
            id: row.get(0)?,
            feed_name: row.get(1)?,
            title: row.get(2)?,
            url: row.get(3)?,
            published_at: row.get::<_, Option<String>>(4)?.map(|s| parse_datetime(&s)),
            first_seen_at: parse_datetime(&row.get::<_, String>(5)?),
            downloaded: row.get::<_, i32>(6)? != 0,
            downloaded_at: row.get::<_, Option<String>>(7)?.map(|s| parse_datetime(&s)),
            category: row.get(8)?,
            size_bytes: row.get::<_, i64>(9)? as u64,
        })
    }

    // -----------------------------------------------------------------------
    // RSS rule operations
    // -----------------------------------------------------------------------

    /// Insert a new RSS download rule.
    pub fn rss_rule_insert(&self, rule: &RssRule) -> anyhow::Result<()> {
        let feed_names_str = rule.feed_names.join(",");
        self.conn.execute(
            "INSERT INTO rss_rules (id, name, feed_name, category, priority, match_regex, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rule.id,
                rule.name,
                feed_names_str,
                rule.category,
                rule.priority,
                rule.match_regex,
                rule.enabled as i32,
            ],
        )?;
        Ok(())
    }

    /// List all RSS download rules.
    pub fn rss_rule_list(&self) -> anyhow::Result<Vec<RssRule>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, feed_name, category, priority, match_regex, enabled
             FROM rss_rules ORDER BY name ASC",
        )?;
        let rules = stmt
            .query_map([], |row| {
                let feed_names_str: String = row.get(2)?;
                let feed_names: Vec<String> = feed_names_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                Ok(RssRule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    feed_names,
                    category: row.get(3)?,
                    priority: row.get(4)?,
                    match_regex: row.get(5)?,
                    enabled: row.get::<_, i32>(6)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rules)
    }

    /// Update an RSS download rule.
    pub fn rss_rule_update(&self, rule: &RssRule) -> anyhow::Result<()> {
        let feed_names_str = rule.feed_names.join(",");
        self.conn.execute(
            "UPDATE rss_rules SET name=?2, feed_name=?3, category=?4, priority=?5,
             match_regex=?6, enabled=?7 WHERE id=?1",
            params![
                rule.id,
                rule.name,
                feed_names_str,
                rule.category,
                rule.priority,
                rule.match_regex,
                rule.enabled as i32,
            ],
        )?;
        Ok(())
    }

    /// Delete an RSS download rule.
    pub fn rss_rule_delete(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM rss_rules WHERE id=?1", params![id])?;
        Ok(())
    }
}

fn parse_datetime(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
