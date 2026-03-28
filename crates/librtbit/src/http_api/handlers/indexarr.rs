//! Indexarr integration — proxy handlers that forward requests to an Indexarr instance.
//!
//! All endpoints return 404 when Indexarr integration is not enabled
//! (i.e. `state.opts.indexarr_url` is `None`).

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use http::StatusCode;
use serde_json::json;

use super::ApiState;

/// Extract Indexarr config from state, returning owned values.
/// Returns Err(Response) if Indexarr is not enabled.
fn indexarr_config(state: &ApiState) -> Result<(String, Option<String>), Box<Response>> {
    match &state.opts.indexarr_url {
        Some(url) => Ok((
            url.trim_end_matches('/').to_string(),
            state.opts.indexarr_api_key.clone(),
        )),
        None => Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "Indexarr integration is not enabled"})),
            )
                .into_response(),
        )),
    }
}

/// Forward a GET request to Indexarr, injecting the API key.
async fn proxy_get(
    base_url: String,
    api_key: Option<String>,
    path: &str,
    params: &HashMap<String, String>,
) -> Response {
    let client = reqwest::Client::new();
    let url = format!("{}{}", base_url, path);
    let mut req = client.get(&url).query(params);
    if let Some(key) = &api_key {
        req = req.header("X-Api-Key", key);
    }

    match req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (status, [("Content-Type", "application/json")], body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Failed to reach Indexarr: {e}")})),
        )
            .into_response(),
    }
}

/// Forward a POST request (JSON body) to Indexarr, injecting the API key.
async fn proxy_post_json(
    base_url: String,
    api_key: Option<String>,
    path: &str,
    body: serde_json::Value,
) -> Response {
    let client = reqwest::Client::new();
    let url = format!("{}{}", base_url, path);
    let mut req = client.post(&url).json(&body);
    if let Some(key) = &api_key {
        req = req.header("X-Api-Key", key);
    }

    match req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.text().await.unwrap_or_default();
            (status, [("Content-Type", "application/json")], body).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": format!("Failed to reach Indexarr: {e}")})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Public handler functions
// ---------------------------------------------------------------------------

/// GET /indexarr/status — check if Indexarr is enabled and reachable.
pub async fn h_indexarr_status(State(state): State<ApiState>) -> Response {
    let Some(url) = &state.opts.indexarr_url else {
        return Json(json!({
            "enabled": false,
        }))
        .into_response();
    };

    let client = reqwest::Client::new();
    let base = url.trim_end_matches('/');
    let mut req = client.get(format!("{base}/health"));
    if let Some(key) = &state.opts.indexarr_api_key {
        req = req.header("X-Api-Key", key);
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            let body: serde_json::Value = resp.json().await.unwrap_or(json!({}));
            Json(json!({
                "enabled": true,
                "reachable": true,
                "indexarr": body,
            }))
            .into_response()
        }
        Ok(resp) => Json(json!({
            "enabled": true,
            "reachable": false,
            "error": format!("Indexarr returned status {}", resp.status()),
        }))
        .into_response(),
        Err(e) => Json(json!({
            "enabled": true,
            "reachable": false,
            "error": format!("{e}"),
        }))
        .into_response(),
    }
}

/// GET /indexarr/search — proxy to Indexarr search API.
pub async fn h_indexarr_search(
    State(state): State<ApiState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(base_url, api_key, "/api/v1/search", &params).await
}

/// GET /indexarr/recent — proxy to Indexarr recent torrents.
pub async fn h_indexarr_recent(
    State(state): State<ApiState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(base_url, api_key, "/api/v1/recent", &params).await
}

/// GET /indexarr/trending — proxy to Indexarr trending torrents.
pub async fn h_indexarr_trending(
    State(state): State<ApiState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(base_url, api_key, "/api/v1/trending", &params).await
}

/// GET /indexarr/torrent/{info_hash} — proxy to Indexarr torrent detail.
pub async fn h_indexarr_torrent_detail(
    State(state): State<ApiState>,
    Path(info_hash): Path<String>,
) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(
        base_url,
        api_key,
        &format!("/api/v1/torrent/{info_hash}"),
        &HashMap::new(),
    )
    .await
}

/// GET /indexarr/identity/status — proxy to Indexarr identity status.
pub async fn h_indexarr_identity_status(State(state): State<ApiState>) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(
        base_url,
        api_key,
        "/api/v1/identity/status",
        &HashMap::new(),
    )
    .await
}

/// POST /indexarr/identity/acknowledge — proxy to Indexarr identity acknowledge.
pub async fn h_indexarr_identity_acknowledge(State(state): State<ApiState>) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_post_json(base_url, api_key, "/api/v1/identity/acknowledge", json!({})).await
}

/// GET /indexarr/sync/preferences — proxy to Indexarr sync preferences.
pub async fn h_indexarr_sync_preferences_get(State(state): State<ApiState>) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_get(
        base_url,
        api_key,
        "/api/v1/system/sync/preferences",
        &HashMap::new(),
    )
    .await
}

/// POST /indexarr/sync/preferences — proxy to Indexarr sync preferences.
pub async fn h_indexarr_sync_preferences_set(
    State(state): State<ApiState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let (base_url, api_key) = match indexarr_config(&state) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    proxy_post_json(base_url, api_key, "/api/v1/system/sync/preferences", body).await
}
