//! qBittorrent WebUI API v2 compatibility layer.
//!
//! This module implements a subset of the qBittorrent WebUI API v2 so that
//! *arr apps (Sonarr, Radarr, etc.) can use rtbit as a download client by
//! pretending to be qBittorrent.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Multipart, Query, State},
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Bytes;
use http::{HeaderMap, StatusCode, header::SET_COOKIE};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    AddTorrent, AddTorrentOptions,
    api::{Api, TorrentIdOrHash},
    torrent_state::stats::TorrentStatsState,
};

use super::ApiState;

// ---------------------------------------------------------------------------
// Shared state for qBit compat layer
// ---------------------------------------------------------------------------

/// In-memory session store for qBittorrent auth cookies.
struct QbitSessions {
    sessions: RwLock<HashMap<String, Instant>>,
}

impl QbitSessions {
    fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    fn create_session(&self) -> String {
        let sid: String = (0..32)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        self.sessions.write().insert(sid.clone(), Instant::now());
        sid
    }

    fn validate_session(&self, sid: &str) -> bool {
        let sessions = self.sessions.read();
        if let Some(created) = sessions.get(sid) {
            // Sessions expire after 1 hour
            created.elapsed().as_secs() < 3600
        } else {
            false
        }
    }

    fn remove_session(&self, sid: &str) {
        self.sessions.write().remove(sid);
    }
}

#[derive(Clone, Serialize)]
struct QbitCategory {
    name: String,
    #[serde(rename = "savePath")]
    save_path: String,
}

/// Combined qBit compat state.
pub(crate) struct QbitState {
    api_state: ApiState,
    sessions: QbitSessions,
}

// ---------------------------------------------------------------------------
// Serializable response types (avoids json! macro recursion limit issues)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct QbitTorrentInfo {
    added_on: u64,
    amount_left: u64,
    auto_tmm: bool,
    availability: i32,
    category: String,
    completed: u64,
    completion_on: i64,
    content_path: String,
    dl_limit: i32,
    dlspeed: u64,
    download_path: String,
    downloaded: u64,
    downloaded_session: u64,
    eta: i64,
    f_l_piece_prio: bool,
    force_start: bool,
    hash: String,
    infohash_v1: String,
    infohash_v2: String,
    last_activity: u64,
    magnet_uri: String,
    max_ratio: i32,
    max_seeding_time: i32,
    name: String,
    num_complete: u32,
    num_incomplete: u32,
    num_leechs: u32,
    num_seeds: u32,
    priority: u32,
    progress: f64,
    ratio: f64,
    ratio_limit: i32,
    save_path: String,
    seeding_time: u64,
    seeding_time_limit: i32,
    seen_complete: i64,
    seq_dl: bool,
    size: u64,
    state: String,
    super_seeding: bool,
    tags: String,
    time_active: u64,
    total_size: u64,
    tracker: String,
    trackers_count: usize,
    up_limit: i32,
    uploaded: u64,
    uploaded_session: u64,
    upspeed: u64,
}

#[derive(Serialize)]
struct QbitTorrentProperties {
    save_path: String,
    creation_date: u64,
    piece_size: u64,
    comment: String,
    total_wasted: u64,
    total_uploaded: u64,
    total_uploaded_session: u64,
    total_downloaded: u64,
    total_downloaded_session: u64,
    up_limit: i32,
    dl_limit: i32,
    time_elapsed: u64,
    seeding_time: u64,
    nb_connections: u32,
    nb_connections_limit: i32,
    share_ratio: f64,
    addition_date: u64,
    completion_date: i64,
    created_by: String,
    dl_speed_avg: u64,
    dl_speed: u64,
    eta: i64,
    last_seen: u64,
    peers: u32,
    peers_total: u32,
    pieces_have: u32,
    pieces_num: u32,
    reannounce: u32,
    seeds: u32,
    seeds_total: u32,
    total_size: u64,
    up_speed_avg: u64,
    up_speed: u64,
}

#[derive(Serialize)]
struct QbitFileInfo {
    index: usize,
    name: String,
    size: u64,
    progress: f64,
    priority: u8,
    is_seed: bool,
    piece_range: [u32; 2],
    availability: f64,
}

#[derive(Serialize)]
struct QbitBuildInfo {
    qt: &'static str,
    libtorrent: &'static str,
    boost: &'static str,
    openssl: &'static str,
    bitness: u32,
}

#[derive(Serialize)]
struct QbitTransferInfo {
    dl_info_speed: u64,
    dl_info_data: u64,
    up_info_speed: u64,
    up_info_data: u64,
    dl_rate_limit: u64,
    up_rate_limit: u64,
    dht_nodes: u64,
    connection_status: &'static str,
}

#[derive(Serialize)]
struct QbitPreferences {
    save_path: String,
    temp_path_enabled: bool,
    temp_path: String,
    max_connec: i32,
    max_connec_per_torrent: i32,
    max_uploads: i32,
    max_uploads_per_torrent: i32,
    dl_limit: i32,
    up_limit: i32,
    dht: bool,
    pex: bool,
    lsd: bool,
    encryption: u32,
    queueing_enabled: bool,
    locale: &'static str,
    web_ui_port: u16,
}

// ---------------------------------------------------------------------------
// Auth endpoints
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct LoginForm {
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

async fn h_auth_login(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: LoginForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();

    let stored_credentials = state
        .api_state
        .opts
        .credential_store
        .as_ref()
        .filter(|store| store.has_credentials());
    let auth_ok = match (stored_credentials, &state.api_state.opts.basic_auth) {
        (Some(store), _) => store.validate(&form.username, &form.password),
        (None, Some((expected_user, expected_pass))) => {
            super::super::constant_time_eq(form.username.as_bytes(), expected_user.as_bytes())
                && super::super::constant_time_eq(
                    form.password.as_bytes(),
                    expected_pass.as_bytes(),
                )
        }
        (None, None) => true,
    };

    if auth_ok {
        let sid = state.sessions.create_session();
        let cookie = format!("SID={sid}; Path=/; HttpOnly");
        (StatusCode::OK, [(SET_COOKIE, cookie)], "Ok.".to_string()).into_response()
    } else {
        (StatusCode::FORBIDDEN, "Fails.".to_string()).into_response()
    }
}

async fn h_auth_logout(State(state): State<Arc<QbitState>>, headers: HeaderMap) -> &'static str {
    if let Some(sid) = extract_sid(&headers) {
        state.sessions.remove_session(&sid);
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// App info endpoints
// ---------------------------------------------------------------------------

async fn h_app_version() -> &'static str {
    "v0.0.1"
}

async fn h_app_webapi_version() -> &'static str {
    "2.11.3"
}

async fn h_app_build_info() -> impl IntoResponse {
    axum::Json(QbitBuildInfo {
        qt: "N/A",
        libtorrent: "N/A",
        boost: "N/A",
        openssl: "N/A",
        bitness: 64,
    })
}

async fn h_app_preferences(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let save_path = state.api_state.api.api_output_folder();

    axum::Json(QbitPreferences {
        save_path,
        temp_path_enabled: false,
        temp_path: String::new(),
        max_connec: -1,
        max_connec_per_torrent: -1,
        max_uploads: -1,
        max_uploads_per_torrent: -1,
        dl_limit: 0,
        up_limit: 0,
        dht: true,
        pex: true,
        lsd: true,
        encryption: 0,
        queueing_enabled: false,
        locale: "en",
        web_ui_port: state.api_state.opts.web_ui_port.unwrap_or(3030),
    })
}

// ---------------------------------------------------------------------------
// Transfer info
// ---------------------------------------------------------------------------

async fn h_transfer_info(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let session_stats = state.api_state.api.api_session_stats();
    axum::Json(QbitTransferInfo {
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_info_data: session_stats.counters.fetched_bytes,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_info_data: session_stats.counters.uploaded_bytes,
        dl_rate_limit: 0,
        up_rate_limit: 0,
        dht_nodes: 0,
        connection_status: "connected",
    })
}

// ---------------------------------------------------------------------------
// Torrent management endpoints
// ---------------------------------------------------------------------------

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Map rtbit torrent state to qBittorrent state string.
fn map_state(state: TorrentStatsState, finished: bool) -> &'static str {
    match (state, finished) {
        (TorrentStatsState::Initializing, _) => "metaDL",
        (TorrentStatsState::Live, false) => "downloading",
        (TorrentStatsState::Live, true) => "uploading",
        (TorrentStatsState::Paused, false) => "pausedDL",
        (TorrentStatsState::Paused, true) => "pausedUP",
        (TorrentStatsState::Error, _) => "error",
    }
}

#[derive(Deserialize, Default)]
struct TorrentsInfoQuery {
    filter: Option<String>,
    category: Option<String>,
    hashes: Option<String>,
    sort: Option<String>,
    reverse: Option<bool>,
    limit: Option<usize>,
    offset: Option<usize>,
}

/// Check if a torrent matches the qBit filter.
fn matches_filter(
    filter: &str,
    qbit_state: &str,
    stats: &crate::torrent_state::stats::TorrentStats,
) -> bool {
    match filter {
        "all" => true,
        "downloading" => qbit_state == "downloading" || qbit_state == "metaDL",
        "seeding" => qbit_state == "uploading",
        "completed" => stats.finished,
        "paused" => qbit_state == "pausedDL" || qbit_state == "pausedUP",
        "active" => qbit_state == "downloading" || qbit_state == "uploading",
        "inactive" => qbit_state != "downloading" && qbit_state != "uploading",
        "resumed" => qbit_state != "pausedDL" && qbit_state != "pausedUP",
        "stalled" | "stalled_uploading" | "stalled_downloading" => {
            matches!(stats.state, TorrentStatsState::Live)
                && stats
                    .live
                    .as_ref()
                    .map(|l| l.download_speed.mbps < 0.001 && l.upload_speed.mbps < 0.001)
                    .unwrap_or(true)
        }
        "errored" => qbit_state == "error",
        _ => true,
    }
}

async fn h_torrents_info(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<TorrentsInfoQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let hash_filter: Option<Vec<String>> = query
        .hashes
        .as_ref()
        .map(|h| h.split('|').map(|s| s.to_lowercase()).collect());

    let mut torrents: Vec<QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.filter_map(|(id, handle)| {
            let info_hash = handle.shared().info_hash.as_string();

            // Filter by hash if specified
            if let Some(ref hashes) = hash_filter
                && !hashes.contains(&info_hash)
            {
                return None;
            }

            let stats = handle.stats();
            let name = handle.name().unwrap_or_else(|| format!("torrent_{id}"));
            let output_folder = handle
                .shared()
                .options
                .output_folder
                .to_string_lossy()
                .into_owned();
            let content_path = format!("{}/{}", output_folder, name);

            let qbit_state = map_state(stats.state, stats.finished);
            let category = handle.shared().category.read().clone().unwrap_or_default();

            // qBittorrent treats an empty category and "uncategorized" as the
            // uncategorized bucket; otherwise category matching is exact.
            if let Some(ref requested) = query.category
                && !matches_category(requested, &category)
            {
                return None;
            }

            // Apply filter
            if let Some(ref filter) = query.filter
                && !matches_filter(filter, qbit_state, &stats)
            {
                return None;
            }

            let dl_speed = stats
                .live
                .as_ref()
                .map(|l| l.download_speed.as_bytes())
                .unwrap_or(0);
            let up_speed = stats
                .live
                .as_ref()
                .map(|l| l.upload_speed.as_bytes())
                .unwrap_or(0);

            let progress = if stats.total_bytes > 0 {
                stats.progress_bytes as f64 / stats.total_bytes as f64
            } else {
                0.0
            };

            let eta = stats
                .total_bytes
                .saturating_sub(stats.progress_bytes)
                .checked_div(dl_speed)
                .map(|seconds| i64::try_from(seconds.min(8_640_000)).unwrap_or(8_640_000))
                .unwrap_or(8_640_000);

            let num_seeds = stats
                .live
                .as_ref()
                .map(|l| l.snapshot.connected_seeders)
                .unwrap_or(0);
            let num_leechs = stats
                .live
                .as_ref()
                .map(|l| l.snapshot.connected_leechers)
                .unwrap_or(0);
            let added_on = handle.shared().added_on;
            let completion_on = handle
                .shared()
                .completion_on
                .load(std::sync::atomic::Ordering::Relaxed);
            let ratio = if stats.progress_bytes == 0 {
                0.0
            } else {
                stats.uploaded_bytes as f64 / stats.progress_bytes as f64
            };
            let time_active = now.saturating_sub(added_on);
            let seeding_time = completion_on
                .checked_sub(added_on)
                .map_or(0, |_| now.saturating_sub(completion_on));

            let tracker = handle
                .shared()
                .trackers
                .iter()
                .next()
                .map(|u| u.to_string())
                .unwrap_or_default();

            let trackers_count = handle.shared().trackers.len();

            Some(QbitTorrentInfo {
                added_on,
                amount_left: stats.total_bytes.saturating_sub(stats.progress_bytes),
                auto_tmm: false,
                availability: -1,
                category,
                completed: stats.progress_bytes,
                completion_on: if completion_on > 0 {
                    i64::try_from(completion_on).unwrap_or(i64::MAX)
                } else {
                    -1
                },
                content_path,
                dl_limit: -1,
                dlspeed: dl_speed,
                download_path: String::new(),
                downloaded: stats.progress_bytes,
                downloaded_session: 0,
                eta,
                f_l_piece_prio: false,
                force_start: false,
                hash: info_hash.clone(),
                infohash_v1: info_hash,
                infohash_v2: String::new(),
                last_activity: if completion_on > 0 {
                    completion_on
                } else {
                    added_on
                },
                magnet_uri: String::new(),
                max_ratio: -1,
                max_seeding_time: -1,
                name,
                num_complete: num_seeds,
                num_incomplete: num_leechs,
                num_leechs,
                num_seeds,
                priority: 0,
                progress,
                ratio,
                ratio_limit: -1,
                save_path: output_folder,
                seeding_time,
                seeding_time_limit: -1,
                seen_complete: if completion_on > 0 {
                    i64::try_from(completion_on).unwrap_or(i64::MAX)
                } else {
                    -1
                },
                seq_dl: false,
                size: stats.total_bytes,
                state: qbit_state.to_string(),
                super_seeding: false,
                tags: String::new(),
                time_active,
                total_size: stats.total_bytes,
                tracker,
                trackers_count,
                up_limit: -1,
                uploaded: stats.uploaded_bytes,
                uploaded_session: stats.uploaded_bytes,
                upspeed: up_speed,
            })
        })
        .collect()
    });

    // Sort
    if let Some(ref sort_field) = query.sort {
        let reverse = query.reverse.unwrap_or(false);
        torrents.sort_by(|a, b| {
            let cmp = match sort_field.as_str() {
                "name" => a.name.cmp(&b.name),
                "size" | "total_size" => a.total_size.cmp(&b.total_size),
                "progress" => a
                    .progress
                    .partial_cmp(&b.progress)
                    .unwrap_or(std::cmp::Ordering::Equal),
                "dlspeed" => a.dlspeed.cmp(&b.dlspeed),
                "upspeed" => a.upspeed.cmp(&b.upspeed),
                "eta" => a.eta.cmp(&b.eta),
                "state" => a.state.cmp(&b.state),
                "added_on" => a.added_on.cmp(&b.added_on),
                "hash" => a.hash.cmp(&b.hash),
                "downloaded" => a.downloaded.cmp(&b.downloaded),
                "uploaded" => a.uploaded.cmp(&b.uploaded),
                "ratio" => a
                    .ratio
                    .partial_cmp(&b.ratio)
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            };
            if reverse { cmp.reverse() } else { cmp }
        });
    }

    // Offset and limit
    let offset = query.offset.unwrap_or(0);
    if offset > 0 && offset < torrents.len() {
        torrents = torrents.split_off(offset);
    }
    if let Some(limit) = query.limit {
        torrents.truncate(limit);
    }

    axum::Json(torrents)
}

fn matches_category(requested: &str, category: &str) -> bool {
    match requested {
        "all" => true,
        "uncategorized" | "" => category.is_empty(),
        value => category == value,
    }
}

#[derive(Deserialize)]
struct HashQuery {
    hash: String,
}

async fn h_torrents_properties(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let stats = handle.stats();
    let output_folder = handle
        .shared()
        .options
        .output_folder
        .to_string_lossy()
        .into_owned();
    let now = now_unix();
    let added_on = handle.shared().added_on;
    let completion_on = handle
        .shared()
        .completion_on
        .load(std::sync::atomic::Ordering::Relaxed);
    let ratio = if stats.progress_bytes == 0 {
        0.0
    } else {
        stats.uploaded_bytes as f64 / stats.progress_bytes as f64
    };

    let dl_speed = stats
        .live
        .as_ref()
        .map(|l| l.download_speed.as_bytes())
        .unwrap_or(0);
    let up_speed = stats
        .live
        .as_ref()
        .map(|l| l.upload_speed.as_bytes())
        .unwrap_or(0);

    let eta = stats
        .total_bytes
        .saturating_sub(stats.progress_bytes)
        .checked_div(dl_speed)
        .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
        .unwrap_or(8_640_000);

    let piece_size = handle
        .with_metadata(|m| m.info.lengths().default_piece_length() as u64)
        .unwrap_or(0);
    let pieces_num = handle
        .with_metadata(|m| m.info.lengths().total_pieces())
        .unwrap_or(0);

    axum::Json(QbitTorrentProperties {
        save_path: output_folder,
        creation_date: now,
        piece_size,
        comment: String::new(),
        total_wasted: 0,
        total_uploaded: stats.uploaded_bytes,
        total_uploaded_session: stats.uploaded_bytes,
        total_downloaded: stats.progress_bytes,
        total_downloaded_session: stats.progress_bytes,
        up_limit: -1,
        dl_limit: -1,
        time_elapsed: now.saturating_sub(added_on),
        seeding_time: if completion_on > 0 {
            now.saturating_sub(completion_on)
        } else {
            0
        },
        nb_connections: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.peer_stats.live)
            .unwrap_or(0),
        nb_connections_limit: -1,
        share_ratio: ratio,
        addition_date: added_on,
        completion_date: if completion_on > 0 {
            i64::try_from(completion_on).unwrap_or(i64::MAX)
        } else {
            -1
        },
        created_by: String::new(),
        dl_speed_avg: dl_speed,
        dl_speed,
        eta,
        last_seen: if completion_on > 0 {
            completion_on
        } else {
            added_on
        },
        peers: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        peers_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_leechers)
            .unwrap_or(0),
        pieces_have: stats
            .progress_bytes
            .checked_div(piece_size)
            .map(|pieces| u32::try_from(pieces).unwrap_or(u32::MAX))
            .unwrap_or(0),
        pieces_num,
        reannounce: 0,
        seeds: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        seeds_total: stats
            .live
            .as_ref()
            .map(|live| live.snapshot.connected_seeders)
            .unwrap_or(0),
        total_size: stats.total_bytes,
        up_speed_avg: up_speed,
        up_speed,
    })
    .into_response()
}

async fn h_torrents_files(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let details = match api.api_torrent_details(idx) {
        Ok(d) => d,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let stats = handle.stats();
    let is_seed = stats.finished;

    let files: Vec<QbitFileInfo> = details
        .files
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let file_progress = stats.file_progress.get(i).copied().unwrap_or(0);
            let progress = if f.length > 0 {
                file_progress as f64 / f.length as f64
            } else {
                1.0
            };
            QbitFileInfo {
                index: i,
                name: f.name.clone(),
                size: f.length,
                progress,
                priority: if f.included { 1 } else { 0 },
                is_seed,
                piece_range: [0, 0],
                availability: if progress >= 1.0 { 1.0 } else { progress },
            }
        })
        .collect();

    axum::Json(files).into_response()
}

// ---------------------------------------------------------------------------
// Torrent actions (add, pause, resume, delete)
// ---------------------------------------------------------------------------

async fn h_torrents_add(
    State(state): State<Arc<QbitState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut urls: Vec<String> = Vec::new();
    let mut torrent_bytes: Vec<Bytes> = Vec::new();
    let mut savepath: Option<String> = None;
    let mut category: Option<String> = None;
    let mut paused = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "urls" => {
                if let Ok(text) = field.text().await {
                    for line in text.lines() {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && urls.len() < 100 {
                            urls.push(trimmed.to_string());
                        }
                    }
                }
            }
            "torrents" => {
                if torrent_bytes.len() >= 100 {
                    continue;
                }
                if let Ok(data) = field.bytes().await
                    && !data.is_empty()
                {
                    torrent_bytes.push(data);
                }
            }
            "category" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    category = Some(text);
                }
            }
            "savepath" => {
                if let Ok(text) = field.text().await
                    && !text.is_empty()
                {
                    savepath = Some(text);
                }
            }
            "paused" => {
                if let Ok(text) = field.text().await {
                    paused = text.eq_ignore_ascii_case("true");
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let api = &state.api_state.api;
    let mut had_error = false;

    for url in urls {
        let opts = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: savepath.clone(),
            category: category.clone(),
            ..Default::default()
        };
        if let Err(e) = api
            .api_add_torrent(AddTorrent::Url(url.into()), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent URL: {e:#}");
            had_error = true;
        }
    }

    for data in torrent_bytes {
        let opts = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: savepath.clone(),
            category: category.clone(),
            ..Default::default()
        };
        if let Err(e) = api
            .api_add_torrent(AddTorrent::TorrentFileBytes(data), Some(opts))
            .await
        {
            warn!("qbit compat: error adding torrent file: {e:#}");
            had_error = true;
        }
    }

    if had_error {
        (StatusCode::INTERNAL_SERVER_ERROR, "Error adding torrent(s)")
    } else {
        (StatusCode::OK, "Ok.")
    }
}

#[derive(Deserialize, Default)]
struct HashesForm {
    #[serde(default)]
    hashes: String,
}

/// Resolve hash(es) from form body. "all" means all torrents.
fn resolve_hashes(api: &Api, hashes_str: &str) -> Vec<TorrentIdOrHash> {
    if hashes_str == "all" {
        api.session().with_torrents(|iter| {
            iter.map(|(_, handle)| TorrentIdOrHash::Hash(handle.shared().info_hash))
                .collect()
        })
    } else {
        hashes_str
            .split('|')
            .filter_map(|h| {
                let h = h.trim();
                if h.is_empty() {
                    return None;
                }
                TorrentIdOrHash::parse(h).ok()
            })
            .collect()
    }
}

async fn h_torrents_pause(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_pause(idx).await {
            warn!("qbit compat: error pausing torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

async fn h_torrents_resume(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);

    for idx in hashes {
        if let Err(e) = api.api_torrent_action_start(idx).await {
            warn!("qbit compat: error resuming torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

#[derive(Deserialize, Default)]
struct DeleteForm {
    #[serde(default)]
    hashes: String,
    #[serde(default, alias = "deleteFiles")]
    delete_files: Option<String>,
}

async fn h_torrents_delete(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: DeleteForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let hashes = resolve_hashes(api, &form.hashes);
    let delete_files = form
        .delete_files
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("true"));

    for idx in hashes {
        let result = if delete_files {
            api.api_torrent_action_delete(idx).await
        } else {
            api.api_torrent_action_forget(idx).await
        };
        if let Err(e) = result {
            warn!("qbit compat: error deleting torrent {idx}: {e:#}");
        }
    }

    "Ok."
}

#[derive(Deserialize, Default)]
struct SetCategoryForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    category: String,
}

async fn h_torrents_set_category(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: SetCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let category = (!form.category.is_empty()).then_some(form.category);
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_set_torrent_category(idx, category.clone()).await {
            warn!(%error, "qbit compat: error setting torrent category");
        }
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Category endpoints
// ---------------------------------------------------------------------------

async fn h_categories(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let map: HashMap<String, QbitCategory> = state
        .api_state
        .api
        .api_list_categories()
        .into_iter()
        .map(|(name, category)| {
            let save_path = category
                .save_path
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            (name.clone(), QbitCategory { name, save_path })
        })
        .collect();
    axum::Json(serde_json::to_value(&map).unwrap_or_default())
}

#[derive(Deserialize, Default)]
struct CreateCategoryForm {
    #[serde(default)]
    category: String,
    #[serde(default, alias = "savePath")]
    save_path: String,
}

async fn h_create_category(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return (StatusCode::BAD_REQUEST, "Category name required").into_response();
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

async fn h_edit_category(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: CreateCategoryForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    if form.category.is_empty() {
        return (StatusCode::BAD_REQUEST, "Category name required").into_response();
    }
    let save_path = (!form.save_path.is_empty()).then(|| form.save_path.into());
    match state
        .api_state
        .api
        .api_create_or_edit_category(form.category, save_path)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct RemoveCategoriesForm {
    #[serde(default)]
    categories: String,
}

async fn h_remove_categories(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: RemoveCategoriesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    for name in form.categories.split('\n') {
        let name = name.trim();
        if !name.is_empty()
            && let Err(error) = state.api_state.api.api_remove_category(name).await
        {
            warn!(%error, %name, "qbit compat: error removing category");
        }
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Auth middleware helper
// ---------------------------------------------------------------------------

fn extract_sid(headers: &HeaderMap) -> Option<String> {
    headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let c = c.trim();
                c.strip_prefix("SID=").map(|s| s.to_string())
            })
        })
}

// ---------------------------------------------------------------------------
// Router construction
// ---------------------------------------------------------------------------

/// Build the qBittorrent v2 API router. Should be nested at `/api/v2`.
pub(crate) fn make_qbit_router(api_state: ApiState) -> Router {
    let qbit_state = Arc::new(QbitState {
        api_state: api_state.clone(),
        sessions: QbitSessions::new(),
    });

    // Auth endpoints (no auth required to reach these)
    let auth_router = Router::new()
        .route("/login", post(h_auth_login))
        .route("/logout", post(h_auth_logout));

    // App info endpoints
    let app_router = Router::new()
        .route("/version", get(h_app_version))
        .route("/webapiVersion", get(h_app_webapi_version))
        .route("/buildInfo", get(h_app_build_info))
        .route("/preferences", get(h_app_preferences));

    // Torrent endpoints
    let torrents_router = Router::new()
        .route("/info", get(h_torrents_info))
        .route("/properties", get(h_torrents_properties))
        .route("/files", get(h_torrents_files))
        .route("/add", post(h_torrents_add))
        .route("/pause", post(h_torrents_pause))
        .route("/resume", post(h_torrents_resume))
        .route("/delete", post(h_torrents_delete))
        .route("/setCategory", post(h_torrents_set_category))
        .route("/categories", get(h_categories))
        .route("/createCategory", post(h_create_category))
        .route("/editCategory", post(h_edit_category))
        .route("/removeCategories", post(h_remove_categories));

    // Transfer info
    let transfer_router = Router::new().route("/info", get(h_transfer_info));

    let protected_router = Router::new()
        .nest("/app", app_router)
        .nest("/torrents", torrents_router)
        .nest("/transfer", transfer_router)
        .route_layer({
            let qbit_state_for_layer = qbit_state.clone();
            axum::middleware::from_fn(
                move |headers: HeaderMap,
                      request: axum::extract::Request,
                      next: axum::middleware::Next| {
                    let qbit_state = qbit_state_for_layer.clone();
                    async move {
                        let auth_configured = qbit_state.api_state.opts.basic_auth.is_some()
                            || qbit_state
                                .api_state
                                .opts
                                .credential_store
                                .as_ref()
                                .is_some_and(|store| store.has_credentials());
                        if !auth_configured {
                            return Ok(next.run(request).await);
                        }
                        let sid = extract_sid(&headers);
                        let valid = sid
                            .as_deref()
                            .is_some_and(|s| qbit_state.sessions.validate_session(s));
                        if valid {
                            Ok(next.run(request).await)
                        } else {
                            Err((StatusCode::FORBIDDEN, "Not authenticated"))
                        }
                    }
                },
            )
        });

    Router::new()
        .nest("/auth", auth_router)
        .merge(protected_router)
        .with_state(qbit_state)
}

#[cfg(test)]
mod tests {
    use super::matches_category;

    #[test]
    fn category_filter_supports_qbittorrent_special_values() {
        assert!(matches_category("all", "Linux ISOs"));
        assert!(matches_category("all", ""));
        assert!(matches_category("uncategorized", ""));
        assert!(matches_category("", ""));
        assert!(!matches_category("uncategorized", "Linux ISOs"));
        assert!(matches_category("Linux ISOs", "Linux ISOs"));
        assert!(!matches_category("linux isos", "Linux ISOs"));
    }

    #[test]
    fn test_eta_overflow_safety() {
        // Simulate: very large remaining bytes / very small download speed
        // This should not panic, and should clamp to a safe fallback.
        let remaining: u64 = u64::MAX;
        let dl_speed: u64 = 1;
        let eta_secs = remaining / dl_speed;
        let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
        assert_eq!(eta, 8640000i64, "should clamp to fallback on overflow");

        // Large but within i64 range
        let remaining: u64 = 1_000_000_000_000;
        let dl_speed: u64 = 100;
        let eta_secs = remaining / dl_speed;
        let eta = i64::try_from(eta_secs).unwrap_or(8640000i64);
        assert_eq!(
            eta, 10_000_000_000i64,
            "should return exact ETA when it fits in i64"
        );

        // Zero speed: the calling code uses a guard, but verify the fallback path
        let dl_speed: u64 = 0;
        let eta = 1_000_000u64
            .checked_div(dl_speed)
            .map(|seconds| i64::try_from(seconds).unwrap_or(8_640_000))
            .unwrap_or(8_640_000);
        assert_eq!(eta, 8640000i64, "zero speed should return 8640000");
    }

    #[test]
    fn test_timestamp_cast_safety() {
        // Current unix timestamp fits in i64, but test the boundary
        let now: u64 = i64::MAX as u64 + 1;
        let result = i64::try_from(now).unwrap_or(i64::MAX);
        assert_eq!(result, i64::MAX, "should clamp to i64::MAX on overflow");

        // Normal timestamp should pass through
        let now: u64 = 1_700_000_000;
        let result = i64::try_from(now).unwrap_or(i64::MAX);
        assert_eq!(result, 1_700_000_000i64);
    }
}
