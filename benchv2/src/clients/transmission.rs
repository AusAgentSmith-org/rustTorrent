use anyhow::Result;
use std::sync::Mutex;

pub struct TransmissionClient {
    url: String,
    http: reqwest::Client,
    session_id: Mutex<String>,
}

impl TransmissionClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
            http: reqwest::Client::new(),
            session_id: Mutex::new(String::new()),
        }
    }

    async fn rpc(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value> {
        let payload = serde_json::json!({"method": method, "arguments": args});
        let sid = self.session_id.lock().unwrap().clone();

        let resp = self
            .http
            .post(&self.url)
            .header("X-Transmission-Session-Id", &sid)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await?;

        if resp.status().as_u16() == 409 {
            if let Some(new_sid) = resp.headers().get("X-Transmission-Session-Id") {
                let new_sid = new_sid.to_str().unwrap_or("").to_string();
                *self.session_id.lock().unwrap() = new_sid.clone();
                let resp = self
                    .http
                    .post(&self.url)
                    .header("X-Transmission-Session-Id", &new_sid)
                    .json(&payload)
                    .timeout(std::time::Duration::from_secs(30))
                    .send()
                    .await?;
                let body: serde_json::Value = resp.json().await?;
                if body.get("result").and_then(|v| v.as_str()) != Some("success") {
                    anyhow::bail!("Transmission RPC error: {body}");
                }
                return Ok(body.get("arguments").cloned().unwrap_or_default());
            }
        }

        let body: serde_json::Value = resp.json().await?;
        if body.get("result").and_then(|v| v.as_str()) != Some("success") {
            anyhow::bail!("Transmission RPC error: {body}");
        }
        Ok(body.get("arguments").cloned().unwrap_or_default())
    }

    pub async fn add_torrent(&self, torrent_path: &str, download_dir: &str) -> Result<()> {
        self.rpc(
            "torrent-add",
            serde_json::json!({
                "filename": torrent_path,
                "download-dir": download_dir,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn get_torrents(&self) -> Result<Vec<serde_json::Value>> {
        let args = self
            .rpc(
                "torrent-get",
                serde_json::json!({"fields": ["id", "name", "status", "percentDone", "rateUpload"]}),
            )
            .await?;
        Ok(args
            .get("torrents")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn is_seeding(&self) -> bool {
        match self.get_torrents().await {
            Ok(torrents) => {
                !torrents.is_empty()
                    && torrents.iter().all(|t| {
                        t.get("status").and_then(|v| v.as_u64()).unwrap_or(0) == 6
                    })
            }
            Err(_) => false,
        }
    }

    pub async fn remove_all(&self) -> Result<()> {
        let torrents = self.get_torrents().await?;
        let ids: Vec<u64> = torrents
            .iter()
            .filter_map(|t| t.get("id").and_then(|v| v.as_u64()))
            .collect();
        if !ids.is_empty() {
            self.rpc(
                "torrent-remove",
                serde_json::json!({"ids": ids, "delete-local-data": false}),
            )
            .await?;
        }
        Ok(())
    }
}
