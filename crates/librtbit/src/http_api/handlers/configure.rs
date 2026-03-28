use axum::{Json, extract::Query, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};

use super::ApiState;
use crate::{
    api::{EmptyJsonResponse, Result},
    limits::LimitsConfig,
};

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/torrents/limits",
    request_body(content = LimitsConfig, description = "Rate limits configuration"),
    responses(
        (status = 200, description = "Rate limits updated", body = EmptyJsonResponse)
    )
))]
pub async fn h_update_session_ratelimits(
    State(state): State<ApiState>,
    Json(limits): Json<LimitsConfig>,
) -> Result<impl IntoResponse> {
    state
        .api
        .session()
        .ratelimits
        .set_upload_bps(limits.upload_bps);
    state
        .api
        .session()
        .ratelimits
        .set_download_bps(limits.download_bps);
    if let Some(peer_limit) = limits.peer_limit {
        state.api.session().set_peer_limit(peer_limit);
    }
    if let Some(init_limit) = limits.concurrent_init_limit {
        state.api.session().set_concurrent_init_limit(init_limit);
    }
    Ok(Json(EmptyJsonResponse {}))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/torrents/limits",
    responses(
        (status = 200, description = "Current rate limits", body = LimitsConfig)
    )
))]
pub async fn h_get_session_ratelimits(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let mut config = state.api.session().ratelimits.get_config();
    config.peer_limit = Some(state.api.session().get_peer_limit());
    config.concurrent_init_limit = Some(state.api.session().get_concurrent_init_limit());
    Ok(Json(config))
}

// --- Folder management ---

#[derive(Serialize, Deserialize)]
pub struct FoldersConfig {
    pub download_folder: String,
    pub completed_folder: Option<String>,
}

pub async fn h_get_folders(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(FoldersConfig {
        download_folder: state.api.api_output_folder(),
        completed_folder: state.api.api_completed_folder(),
    }))
}

pub async fn h_set_folders(
    State(state): State<ApiState>,
    Json(config): Json<FoldersConfig>,
) -> Result<impl IntoResponse> {
    state.api.api_set_output_folder(config.download_folder);
    state.api.api_set_completed_folder(config.completed_folder);
    Ok(Json(EmptyJsonResponse {}))
}

// --- Directory browsing ---

#[derive(Deserialize)]
pub struct BrowseQuery {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct BrowseResponse {
    pub current: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntry>,
}

pub async fn h_browse_directory(Query(query): Query<BrowseQuery>) -> Result<impl IntoResponse> {
    let path = query.path.map(std::path::PathBuf::from).unwrap_or_else(|| {
        // Default to root on Unix
        std::path::PathBuf::from("/")
    });

    // Canonicalize to prevent directory traversal
    let canonical = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("invalid path: {e}"))?;

    let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());

    let mut entries = Vec::new();
    let read_dir =
        std::fs::read_dir(&canonical).map_err(|e| anyhow::anyhow!("cannot read directory: {e}"))?;

    for entry in read_dir.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip hidden files/dirs
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        entries.push(DirEntry {
            name,
            path: entry_path.to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }

    // Sort: directories first, then alphabetically
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(Json(BrowseResponse {
        current: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    }))
}
