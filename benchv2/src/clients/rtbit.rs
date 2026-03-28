use anyhow::Result;
use std::path::Path;

pub struct RtbitClient {
    url: String,
    http: reqwest::Client,
}

impl RtbitClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn healthy(&self) -> bool {
        self.http
            .get(format!("{}/", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub async fn add_torrent(&self, torrent_path: &Path) -> Result<u64> {
        let data = tokio::fs::read(torrent_path).await?;
        let resp = self
            .http
            .post(format!("{}/torrents?overwrite=true", self.url))
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await?;
        resp.error_for_status_ref()?;
        let body: serde_json::Value = resp.json().await?;
        // Response: {"id": N, "details": {...}, ...}
        if let Some(id) = body.get("id").and_then(|v| v.as_u64()) {
            return Ok(id);
        }
        if let Some(id) = body
            .get("details")
            .and_then(|d| d.get("id"))
            .and_then(|v| v.as_u64())
        {
            return Ok(id);
        }
        anyhow::bail!("could not extract torrent id from response: {body}")
    }

    pub async fn list_torrents(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(format!("{}/torrents", self.url))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        let body: serde_json::Value = resp.json().await?;
        if let Some(arr) = body.get("torrents").and_then(|v| v.as_array()) {
            return Ok(arr.clone());
        }
        if let Some(arr) = body.as_array() {
            return Ok(arr.clone());
        }
        Ok(vec![])
    }

    pub async fn stats(&self, id: u64) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{}/torrents/{}/stats/v1", self.url, id))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete_torrent(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/torrents/{}/delete", self.url, id))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;
        if resp.status().is_client_error() || resp.status().is_server_error() {
            tracing::warn!("rtbit delete {id}: HTTP {}", resp.status());
        }
        Ok(())
    }

    pub async fn delete_all(&self) -> Result<()> {
        let torrents = self.list_torrents().await.unwrap_or_default();
        for t in &torrents {
            if let Some(id) = t.get("id").and_then(|v| v.as_u64()) {
                let _ = self.delete_torrent(&id.to_string()).await;
            }
        }
        // Retry by info_hash
        let torrents = self.list_torrents().await.unwrap_or_default();
        for t in &torrents {
            if let Some(ih) = t.get("info_hash").and_then(|v| v.as_str()) {
                let _ = self.delete_torrent(ih).await;
            }
        }
        Ok(())
    }

    pub async fn all_finished(&self, ids: &[u64]) -> Result<bool> {
        for &id in ids {
            let s = self.stats(id).await?;
            if !s.get("finished").and_then(|v| v.as_bool()).unwrap_or(false) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub async fn aggregate_speed(&self, ids: &[u64]) -> f64 {
        let mut total = 0.0f64;
        for &id in ids {
            if let Ok(s) = self.stats(id).await {
                if let Some(live) = s.get("live") {
                    if let Some(ds) = live.get("download_speed") {
                        // Speed serializes as {"mbps": <float>, ...} where mbps = MiB/s
                        if let Some(mibps) = ds.get("mbps").and_then(|v| v.as_f64()) {
                            total += mibps * 1024.0 * 1024.0; // MiB/s -> bytes/s
                        }
                    }
                }
            }
        }
        total
    }

    pub async fn progress_fraction(&self, ids: &[u64]) -> f64 {
        if ids.is_empty() {
            return 0.0;
        }
        let mut sum = 0.0f64;
        for &id in ids {
            if let Ok(s) = self.stats(id).await {
                let total = s
                    .get("total_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as f64;
                let progress = s
                    .get("progress_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as f64;
                sum += progress / total;
            }
        }
        sum / ids.len() as f64
    }
}
