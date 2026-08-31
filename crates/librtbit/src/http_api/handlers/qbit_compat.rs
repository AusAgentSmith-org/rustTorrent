//! qBittorrent WebUI API v2 compatibility layer.
//!
//! This module implements a subset of the qBittorrent WebUI API v2 so that
//! *arr apps (Sonarr, Radarr, etc.) can use rtbit as a download client by
//! pretending to be qBittorrent.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
    path::Path,
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
    limits::LimitsConfig,
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

/// In-memory tag store for the compat layer. rtbit has no native tag concept,
/// so tags live here (a global set plus a per-torrent, info-hash-keyed set) and
/// are not persisted across restarts.
#[derive(Default)]
struct QbitTags {
    all: RwLock<BTreeSet<String>>,
    per_torrent: RwLock<HashMap<String, BTreeSet<String>>>,
}

impl QbitTags {
    fn all_tags(&self) -> Vec<String> {
        self.all.read().iter().cloned().collect()
    }

    fn create(&self, tags: &[String]) {
        let mut all = self.all.write();
        all.extend(tags.iter().cloned());
    }

    fn delete(&self, tags: &[String]) {
        let mut all = self.all.write();
        let mut per = self.per_torrent.write();
        for tag in tags {
            all.remove(tag);
        }
        for set in per.values_mut() {
            for tag in tags {
                set.remove(tag);
            }
        }
    }

    fn add_to(&self, hashes: &[String], tags: &[String]) {
        if tags.is_empty() {
            return;
        }
        self.create(tags);
        let mut per = self.per_torrent.write();
        for hash in hashes {
            per.entry(hash.clone())
                .or_default()
                .extend(tags.iter().cloned());
        }
    }

    fn remove_from(&self, hashes: &[String], tags: &[String]) {
        let mut per = self.per_torrent.write();
        for hash in hashes {
            if let Some(set) = per.get_mut(hash) {
                // An empty tag list clears every tag, matching qBittorrent.
                if tags.is_empty() {
                    set.clear();
                } else {
                    for tag in tags {
                        set.remove(tag);
                    }
                }
            }
        }
    }

    fn set(&self, hashes: &[String], tags: &[String]) {
        self.create(tags);
        let mut per = self.per_torrent.write();
        for hash in hashes {
            per.insert(hash.clone(), tags.iter().cloned().collect());
        }
    }

    fn tags_for(&self, hash: &str) -> String {
        self.per_torrent
            .read()
            .get(hash)
            .map(|set| set.iter().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default()
    }

    fn has_tag(&self, hash: &str, tag: &str) -> bool {
        self.per_torrent
            .read()
            .get(hash)
            .is_some_and(|set| set.contains(tag))
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
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
    tags: QbitTags,
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

fn qbit_save_path(handle: &crate::torrent_state::ManagedTorrentHandle) -> String {
    handle
        .shared()
        .options
        .output_folder_root
        .as_ref()
        .unwrap_or(&handle.shared().options.output_folder)
        .to_string_lossy()
        .into_owned()
}

fn qbit_content_path(handle: &crate::torrent_state::ManagedTorrentHandle, name: &str) -> String {
    Path::new(&qbit_save_path(handle))
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn qbit_file_name(
    torrent_name: Option<&str>,
    file_name: &str,
    is_multi_file: bool,
    has_save_path_root: bool,
) -> String {
    if is_multi_file && has_save_path_root {
        torrent_name
            .map(|root| {
                Path::new(root)
                    .join(file_name)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| file_name.to_owned())
    } else {
        file_name.to_owned()
    }
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
    listen_port: u16,
    announce_port: u16,
}

#[derive(Deserialize)]
struct SetPreferencesForm {
    json: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPreferences {
    announce_port: u16,
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

async fn h_app_default_save_path(State(state): State<Arc<QbitState>>) -> String {
    state.api_state.api.api_output_folder()
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

async fn h_app_preferences(State(state): State<Arc<QbitState>>) -> axum::Json<QbitPreferences> {
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
        listen_port: state.api_state.api.api_listen_port(),
        announce_port: state.api_state.api.api_announce_port(),
    })
}

async fn h_app_set_preferences(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form = match serde_urlencoded::from_bytes::<SetPreferencesForm>(&body) {
        Ok(form) => form,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid preferences form: {error}"),
            );
        }
    };
    let preferences = match serde_json::from_str::<SetPreferences>(&form.json) {
        Ok(preferences) => preferences,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid preferences JSON: {error}"),
            );
        }
    };
    let Some(port) = NonZeroU16::new(preferences.announce_port) else {
        return (
            StatusCode::BAD_REQUEST,
            "announce_port must be between 1 and 65535".to_string(),
        );
    };

    state.api_state.api.api_set_announce_port(port);
    (StatusCode::OK, String::new())
}

// ---------------------------------------------------------------------------
// Transfer info
// ---------------------------------------------------------------------------

async fn h_transfer_info(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let session_stats = state.api_state.api.api_session_stats();
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(QbitTransferInfo {
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_info_data: session_stats.counters.fetched_bytes,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_info_data: session_stats.counters.uploaded_bytes,
        dl_rate_limit: bps_to_u64(config.download_bps),
        up_rate_limit: bps_to_u64(config.upload_bps),
        dht_nodes: 0,
        connection_status: "connected",
    })
}

fn bps_to_u64(bps: Option<NonZeroU32>) -> u64 {
    bps.map_or(0, |v| u64::from(v.get()))
}

/// Parse a qBittorrent byte-rate limit (0 means unlimited).
fn limit_to_bps(limit: u64) -> Option<NonZeroU32> {
    NonZeroU32::new(u32::try_from(limit).unwrap_or(u32::MAX))
}

#[derive(Deserialize, Default)]
struct LimitForm {
    #[serde(default)]
    limit: u64,
}

#[derive(Deserialize, Default)]
struct ModeForm {
    #[serde(default)]
    mode: u8,
}

async fn h_transfer_download_limit(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.download_bps))
}

async fn h_transfer_upload_limit(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let config = state.api_state.api.session().ratelimits.get_config();
    axum::Json(bps_to_u64(config.upload_bps))
}

async fn h_transfer_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: LimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let session = state.api_state.api.session();
    let upload = session.ratelimits.get_config().upload_bps;
    session.set_normal_rate_limits(limit_to_bps(form.limit), upload);
    "Ok."
}

async fn h_transfer_set_upload_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: LimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let session = state.api_state.api.session();
    let download = session.ratelimits.get_config().download_bps;
    session.set_normal_rate_limits(download, limit_to_bps(form.limit));
    "Ok."
}

/// qBittorrent alternative-speed mode maps onto our alt-speed toggle: `1` when
/// alternative limits are active, `0` otherwise.
async fn h_transfer_speed_limits_mode(State(state): State<Arc<QbitState>>) -> &'static str {
    if state.api_state.api.session().alt_speed_enabled() {
        "1"
    } else {
        "0"
    }
}

async fn h_transfer_toggle_speed_limits_mode(State(state): State<Arc<QbitState>>) -> &'static str {
    let session = state.api_state.api.session();
    session.set_alt_speed_enabled(!session.alt_speed_enabled());
    "Ok."
}

async fn h_transfer_set_speed_limits_mode(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: ModeForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    state
        .api_state
        .api
        .session()
        .set_alt_speed_enabled(form.mode != 0);
    "Ok."
}

/// `transfer/pauseSession` — qBittorrent pauses the whole session; we have no
/// global pause, so we pause every torrent.
async fn h_transfer_pause_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_pause(idx).await {
            warn!(%error, "qbit compat: error pausing session torrent");
        }
    }
    "Ok."
}

/// `transfer/resumeSession` — resume every torrent (see pauseSession).
async fn h_transfer_resume_session(State(state): State<Arc<QbitState>>) -> &'static str {
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, "all") {
        if let Err(error) = api.api_torrent_action_start(idx).await {
            warn!(%error, "qbit compat: error resuming session torrent");
        }
    }
    "Ok."
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
///
/// We advertise WebAPI 2.11.3, so we emit the post-2.11 `stoppedDL`/`stoppedUP`
/// names (renamed from `pausedDL`/`pausedUP` in 2.11); clients that switch on
/// the advertised version expect these.
fn map_state(state: TorrentStatsState, finished: bool) -> &'static str {
    match (state, finished) {
        (TorrentStatsState::Initializing, _) => "metaDL",
        (TorrentStatsState::Live, false) => "downloading",
        (TorrentStatsState::Live, true) => "uploading",
        (TorrentStatsState::Paused, false) => "stoppedDL",
        (TorrentStatsState::Paused, true) => "stoppedUP",
        (TorrentStatsState::Error, _) => "error",
    }
}

#[derive(Deserialize, Default)]
struct TorrentsInfoQuery {
    filter: Option<String>,
    category: Option<String>,
    tag: Option<String>,
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
        // `paused` was renamed to `stopped` in 2.11; accept both spellings.
        "paused" | "stopped" => qbit_state == "stoppedDL" || qbit_state == "stoppedUP",
        "active" => qbit_state == "downloading" || qbit_state == "uploading",
        "inactive" => qbit_state != "downloading" && qbit_state != "uploading",
        // `resumed` was renamed to `running` in 2.11; accept both spellings.
        "resumed" | "running" => qbit_state != "stoppedDL" && qbit_state != "stoppedUP",
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

/// Build the qBittorrent `torrents/info` view of a single torrent. Shared by
/// `torrents/info` and `sync/maindata`.
fn build_torrent_info(
    handle: &crate::torrent_state::ManagedTorrentHandle,
    stats: &crate::torrent_state::stats::TorrentStats,
    now: u64,
) -> QbitTorrentInfo {
    let info_hash = handle.shared().info_hash.as_string();
    let name = handle
        .name()
        .unwrap_or_else(|| format!("torrent_{}", handle.id()));
    let output_folder = qbit_save_path(handle);
    let content_path = qbit_content_path(handle, &name);
    let qbit_state = map_state(stats.state, stats.finished);
    let category = handle.shared().category.read().clone().unwrap_or_default();

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
        .read()
        .iter()
        .next()
        .map(|u| u.to_string())
        .unwrap_or_default();

    let trackers_count = handle.shared().trackers.read().len();

    QbitTorrentInfo {
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
    }
}

/// Apply qBittorrent `offset`/`limit` pagination. An offset at or past the end
/// yields an empty list (not the whole list); a `limit` of 0 means "no limit".
fn apply_offset_limit<T>(mut items: Vec<T>, offset: usize, limit: Option<usize>) -> Vec<T> {
    if offset >= items.len() {
        return Vec::new();
    }
    if offset > 0 {
        items = items.split_off(offset);
    }
    if let Some(limit) = limit
        && limit > 0
    {
        items.truncate(limit);
    }
    items
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
        iter.filter_map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.tags = state.tags.tags_for(&info.hash);

            // Filter by hash if specified.
            if let Some(ref hashes) = hash_filter
                && !hashes.contains(&info.hash)
            {
                return None;
            }

            // qBittorrent treats an empty category and "uncategorized" as the
            // uncategorized bucket; otherwise category matching is exact.
            if let Some(ref requested) = query.category
                && !matches_category(requested, &info.category)
            {
                return None;
            }

            // A non-empty tag query filters to torrents carrying that tag.
            if let Some(ref tag) = query.tag
                && !tag.is_empty()
                && !state.tags.has_tag(&info.hash, tag)
            {
                return None;
            }

            // Apply the state filter.
            if let Some(ref filter) = query.filter
                && !matches_filter(filter, &info.state, &stats)
            {
                return None;
            }

            Some(info)
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

    let torrents = apply_offset_limit(torrents, query.offset.unwrap_or(0), query.limit);
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
    let output_folder = qbit_save_path(&handle);
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

    let details_name = details.name.clone();
    let details_files = details.files.unwrap_or_default();
    let qbit_root = handle.shared().options.output_folder_root.is_some();
    let is_multi_file = handle
        .with_metadata(|metadata| metadata.info.info().files.is_some())
        .unwrap_or(false);
    // Source per-file names from file_infos (which reflects renames) when its
    // count lines up with the details file list; otherwise fall back to the
    // metadata names to avoid any index skew (e.g. padding files).
    let rel_names: Vec<String> = handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let use_rel_names = rel_names.len() == details_files.len();
    let files: Vec<QbitFileInfo> = details_files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let file_progress = stats.file_progress.get(i).copied().unwrap_or(0);
            let progress = if f.length > 0 {
                file_progress as f64 / f.length as f64
            } else {
                1.0
            };
            let base_name = if use_rel_names {
                rel_names[i].as_str()
            } else {
                f.name.as_str()
            };
            let name = qbit_file_name(details_name.as_deref(), base_name, is_multi_file, qbit_root);
            QbitFileInfo {
                index: i,
                name,
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

#[derive(Serialize)]
struct QbitTrackerInfo {
    url: String,
    /// qBittorrent tracker status: 0 disabled, 1 not contacted, 2 working,
    /// 3 updating, 4 not working.
    status: u8,
    tier: i32,
    num_peers: i64,
    num_seeds: i64,
    num_leeches: i64,
    num_downloaded: i64,
    msg: String,
}

/// Map our tracker announce state to qBittorrent's integer status code.
fn qbit_tracker_status(state: tracker_comms::TrackerAnnounceState) -> u8 {
    use tracker_comms::TrackerAnnounceState::*;
    match state {
        Disabled => 0,
        NotContacted => 1,
        Working => 2,
        Updating => 3,
        Error => 4,
    }
}

async fn h_torrents_trackers(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let response = match api.api_tracker_status(idx) {
        Ok(r) => r,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    let trackers: Vec<QbitTrackerInfo> = response
        .trackers
        .into_iter()
        .map(|t| QbitTrackerInfo {
            status: qbit_tracker_status(t.state),
            tier: 0,
            num_peers: t.peers_returned.map_or(-1, i64::from),
            num_seeds: t.seeders.map_or(-1, i64::from),
            num_leeches: t.leechers.map_or(-1, i64::from),
            num_downloaded: -1,
            msg: t.last_error.unwrap_or_default(),
            url: t.url,
        })
        .collect();
    axum::Json(trackers).into_response()
}

async fn h_torrents_piece_states(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let idx = match TorrentIdOrHash::parse(&query.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let (bf, len) = match api.api_dump_haves(idx) {
        Ok(v) => v,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    // qBittorrent: 0 not downloaded, 1 downloading (requested), 2 downloaded.
    // We only distinguish have/not-have, so emit 0 or 2.
    let states: Vec<u8> = bf
        .iter()
        .take(len as usize)
        .map(|b| if *b { 2 } else { 0 })
        .collect();
    axum::Json(states).into_response()
}

async fn h_torrents_piece_hashes(
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
    let hashes: Option<Vec<String>> = handle
        .with_metadata(|m| {
            let info = m.info.info();
            let total = m.info.lengths().total_pieces();
            (0..total)
                .map(|p| {
                    info.get_hash(p).map(|h| {
                        h.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
                    })
                })
                .collect::<Option<Vec<String>>>()
        })
        .ok()
        .flatten();
    match hashes {
        Some(hashes) => axum::Json(hashes).into_response(),
        // Metadata not yet available (magnet still resolving) or v2-only torrent.
        None => axum::Json(Vec::<String>::new()).into_response(),
    }
}

async fn h_torrents_count(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    let count = state
        .api_state
        .api
        .session()
        .with_torrents(|iter| iter.count());
    axum::Json(count)
}

/// `torrents/export` — return the raw `.torrent` file bytes for one torrent.
async fn h_torrents_export(
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
    match handle.with_metadata(|meta| meta.torrent_bytes.clone()) {
        Ok(bytes) => (
            [("content-type", "application/x-bittorrent")],
            bytes,
        )
            .into_response(),
        // Metadata not resolved yet (magnet) — no file to export.
        Err(_) => (StatusCode::CONFLICT, "Metadata not available").into_response(),
    }
}

#[derive(Serialize)]
struct QbitWebSeed {
    url: String,
}

async fn h_torrents_webseeds(
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
    let seeds: Vec<QbitWebSeed> = handle
        .shared()
        .web_seed_urls
        .iter()
        .map(|url| QbitWebSeed { url: url.clone() })
        .collect();
    axum::Json(seeds).into_response()
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
            // `paused` was renamed to `stopped` in WebAPI 2.11; accept both.
            "paused" | "stopped" => {
                if let Ok(text) = field.text().await
                    && text.eq_ignore_ascii_case("true")
                {
                    paused = true;
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
            output_folder_root: savepath.clone(),
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
            output_folder_root: savepath.clone(),
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

/// `torrents/stop` — the WebAPI 2.11+ name for `torrents/pause`. We advertise
/// 2.11.3, so modern clients (qbittorrent-api, newer *arr) call this.
async fn h_torrents_stop(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_pause(state, body).await
}

/// `torrents/start` — the WebAPI 2.11+ name for `torrents/resume`.
async fn h_torrents_start(state: State<Arc<QbitState>>, body: Bytes) -> &'static str {
    h_torrents_resume(state, body).await
}

async fn h_torrents_recheck(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Err(error) = api.api_torrent_action_recheck(idx).await {
            warn!(%error, "qbit compat: error rechecking torrent");
        }
    }
    "Ok."
}

async fn h_torrents_reannounce(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: HashesForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.reannounce();
        }
    }
    "Ok."
}

/// Relative paths of every file in the torrent, indexed by file id.
fn torrent_file_paths(handle: &crate::torrent_state::ManagedTorrentHandle) -> Vec<PathBuf> {
    handle
        .with_metadata(|metadata| {
            metadata
                .file_infos
                .iter()
                .map(|fi| fi.relative_filename.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Map a rename engine error onto qBittorrent's 409 Conflict (used for
/// "torrent must be stopped", path collisions, invalid paths, etc.).
fn rename_conflict(error: anyhow::Error) -> axum::response::Response {
    (StatusCode::CONFLICT, format!("{error:#}")).into_response()
}

#[derive(Deserialize, Default)]
struct RenamePathForm {
    #[serde(default)]
    hash: String,
    #[serde(default, alias = "oldPath")]
    old_path: String,
    #[serde(default, alias = "newPath")]
    new_path: String,
}

async fn h_torrents_rename_file(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required").into_response();
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return (StatusCode::BAD_REQUEST, "oldPath and newPath are required").into_response();
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let paths = torrent_file_paths(&handle);
    let old = PathBuf::from(&form.old_path);
    let file_id = match paths.iter().position(|p| *p == old) {
        Some(id) => id,
        None => return (StatusCode::CONFLICT, "oldPath does not exist").into_response(),
    };
    match handle.rename_files(&[(file_id, PathBuf::from(&form.new_path))]) {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

async fn h_torrents_rename_folder(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> axum::response::Response {
    let form: RenamePathForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required").into_response();
    }
    if form.old_path.is_empty() || form.new_path.is_empty() {
        return (StatusCode::BAD_REQUEST, "oldPath and newPath are required").into_response();
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    // Rename every file whose path is under the old folder prefix.
    let old_prefix = PathBuf::from(&form.old_path);
    let new_prefix = PathBuf::from(&form.new_path);
    let mut renames: Vec<(usize, PathBuf)> = Vec::new();
    for (id, path) in torrent_file_paths(&handle).into_iter().enumerate() {
        if let Ok(rest) = path.strip_prefix(&old_prefix) {
            renames.push((id, new_prefix.join(rest)));
        }
    }
    if renames.is_empty() {
        return (StatusCode::CONFLICT, "no files under oldPath").into_response();
    }
    match handle.rename_files(&renames) {
        Ok(()) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => rename_conflict(error),
    }
}

#[derive(Deserialize, Default)]
struct RenameForm {
    #[serde(default)]
    hash: String,
    #[serde(default)]
    name: String,
}

async fn h_torrents_rename(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: RenameForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    if form.name.trim().is_empty() {
        return (StatusCode::CONFLICT, "name must not be empty");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    handle.set_display_name(Some(form.name));
    (StatusCode::OK, "Ok.")
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

#[derive(Deserialize, Default)]
struct AddTrackersForm {
    #[serde(default)]
    hash: String,
    #[serde(default)]
    urls: String,
}

async fn h_torrents_add_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: AddTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let trackers: Vec<String> = form
        .urls
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if trackers.is_empty() {
        return (StatusCode::OK, "Ok.");
    }
    match api.api_torrent_action_add_trackers(idx, trackers).await {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error adding trackers");
            (StatusCode::NOT_FOUND, "Not found")
        }
    }
}

#[derive(Deserialize, Default)]
struct RemoveTrackersForm {
    #[serde(default)]
    hash: String,
    /// Pipe-separated tracker URLs.
    #[serde(default)]
    urls: String,
}

async fn h_torrents_remove_trackers(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: RemoveTrackersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required").into_response();
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let urls: Vec<String> = form
        .urls
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if let Err(error) = api.session().remove_trackers(&handle, &urls).await {
        warn!(%error, "qbit compat: error removing trackers");
    }
    (StatusCode::OK, "Ok.").into_response()
}

#[derive(Deserialize, Default)]
struct EditTrackerForm {
    #[serde(default)]
    hash: String,
    #[serde(default, alias = "origUrl")]
    orig_url: String,
    #[serde(default, alias = "newUrl")]
    new_url: String,
}

async fn h_torrents_edit_tracker(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> impl IntoResponse {
    let form: EditTrackerForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required").into_response();
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    // Validate newUrl *before* removing origUrl so a bad newUrl can't leave the
    // torrent with neither tracker.
    let valid_new = url::Url::parse(&form.new_url)
        .ok()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https" | "udp"));
    if !valid_new {
        return (StatusCode::BAD_REQUEST, "invalid newUrl").into_response();
    }
    let removed = api
        .session()
        .remove_trackers(&handle, std::slice::from_ref(&form.orig_url))
        .await
        .unwrap_or(0);
    if removed == 0 {
        return (StatusCode::CONFLICT, "origUrl not found").into_response();
    }
    match api
        .api_torrent_action_add_trackers(idx, vec![form.new_url])
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok.").into_response(),
        Err(error) => {
            warn!(%error, "qbit compat: error editing tracker");
            (StatusCode::CONFLICT, "Failed").into_response()
        }
    }
}

#[derive(Deserialize, Default)]
struct AddPeersForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    peers: String,
}

async fn h_torrents_add_peers(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: AddPeersForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let peers: Vec<std::net::SocketAddr> = form
        .peers
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    for idx in resolve_hashes(api, &form.hashes) {
        let Ok(handle) = api.mgr_handle(idx) else {
            continue;
        };
        let Some(live) = handle.live() else {
            continue;
        };
        for addr in &peers {
            let _ = live.add_peer_if_not_seen(*addr);
        }
    }
    "Ok."
}

#[derive(Deserialize, Default)]
struct FilePrioForm {
    #[serde(default)]
    hash: String,
    /// Pipe-separated file indices.
    #[serde(default)]
    id: String,
    #[serde(default)]
    priority: u8,
}

async fn h_torrents_file_prio(State(state): State<Arc<QbitState>>, body: Bytes) -> impl IntoResponse {
    let form: FilePrioForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    if form.hash.is_empty() {
        return (StatusCode::BAD_REQUEST, "hash is required");
    }
    let idx = match TorrentIdOrHash::parse(&form.hash) {
        Ok(idx) => idx,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let handle = match api.mgr_handle(idx) {
        Ok(h) => h,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };
    let num_files = match api.api_torrent_details(idx) {
        Ok(details) => details.files.map(|f| f.len()).unwrap_or(0),
        Err(_) => return (StatusCode::NOT_FOUND, "Not found"),
    };

    // qBittorrent priority 0 means "do not download"; anything else downloads.
    // We only model an include/exclude selection, so map onto only_files.
    let mut included: HashSet<usize> = match handle.only_files() {
        Some(files) => files.into_iter().collect(),
        None => (0..num_files).collect(),
    };
    let download = form.priority != 0;
    for id in form.id.split('|').filter_map(|s| s.trim().parse::<usize>().ok()) {
        if id >= num_files {
            return (StatusCode::CONFLICT, "Invalid file id");
        }
        if download {
            included.insert(id);
        } else {
            included.remove(&id);
        }
    }
    match api
        .api_torrent_action_update_only_files(idx, &included)
        .await
    {
        Ok(_) => (StatusCode::OK, "Ok."),
        Err(error) => {
            warn!(%error, "qbit compat: error setting file priority");
            (StatusCode::CONFLICT, "Failed")
        }
    }
}

// ---------------------------------------------------------------------------
// Per-torrent speed limits
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct HashesQuery {
    #[serde(default)]
    hashes: String,
}

#[derive(Deserialize, Default)]
struct SetTorrentLimitForm {
    #[serde(default)]
    hashes: String,
    /// Bytes/s; <= 0 means unlimited.
    #[serde(default)]
    limit: i64,
}

fn torrent_limit_map(
    api: &Api,
    hashes: &str,
    pick: impl Fn(LimitsConfig) -> Option<NonZeroU32>,
) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    for idx in resolve_hashes(api, hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            let hash = handle.shared().info_hash.as_string();
            map.insert(hash, bps_to_u64(pick(handle.rate_limits())));
        }
    }
    map
}

fn parse_torrent_limit(limit: i64) -> Option<NonZeroU32> {
    if limit <= 0 {
        None
    } else {
        limit_to_bps(limit as u64)
    }
}

async fn h_torrents_download_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.download_bps);
    axum::Json(map)
}

async fn h_torrents_upload_limit(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<HashesQuery>,
) -> impl IntoResponse {
    let map = torrent_limit_map(&state.api_state.api, &query.hashes, |c| c.upload_bps);
    axum::Json(map)
}

async fn h_torrents_set_download_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.set_download_limit(bps);
        }
    }
    "Ok."
}

async fn h_torrents_set_upload_limit(
    State(state): State<Arc<QbitState>>,
    body: Bytes,
) -> &'static str {
    let form: SetTorrentLimitForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let api = &state.api_state.api;
    let bps = parse_torrent_limit(form.limit);
    for idx in resolve_hashes(api, &form.hashes) {
        if let Ok(handle) = api.mgr_handle(idx) {
            handle.set_upload_limit(bps);
        }
    }
    "Ok."
}

// ---------------------------------------------------------------------------
// Tag endpoints (backed by the in-memory QbitTags store)
// ---------------------------------------------------------------------------

/// Resolve hash(es) to canonical (lowercase hex) info-hash strings, used as the
/// key space for the tag store. Skips torrents that cannot be resolved.
fn resolve_info_hashes(api: &Api, hashes_str: &str) -> Vec<String> {
    resolve_hashes(api, hashes_str)
        .into_iter()
        .filter_map(|idx| {
            api.mgr_handle(idx)
                .ok()
                .map(|handle| handle.shared().info_hash.as_string())
        })
        .collect()
}

#[derive(Deserialize, Default)]
struct TagsForm {
    #[serde(default)]
    hashes: String,
    #[serde(default)]
    tags: String,
}

async fn h_torrents_tags(State(state): State<Arc<QbitState>>) -> impl IntoResponse {
    axum::Json(state.tags.all_tags())
}

async fn h_torrents_create_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    state.tags.create(&parse_tags(&form.tags));
    "Ok."
}

async fn h_torrents_delete_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    state.tags.delete(&parse_tags(&form.tags));
    "Ok."
}

async fn h_torrents_add_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let hashes = resolve_info_hashes(&state.api_state.api, &form.hashes);
    state.tags.add_to(&hashes, &parse_tags(&form.tags));
    "Ok."
}

async fn h_torrents_remove_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let hashes = resolve_info_hashes(&state.api_state.api, &form.hashes);
    state.tags.remove_from(&hashes, &parse_tags(&form.tags));
    "Ok."
}

async fn h_torrents_set_tags(State(state): State<Arc<QbitState>>, body: Bytes) -> &'static str {
    let form: TagsForm = serde_urlencoded::from_bytes(&body).unwrap_or_default();
    let hashes = resolve_info_hashes(&state.api_state.api, &form.hashes);
    state.tags.set(&hashes, &parse_tags(&form.tags));
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
// Sync (main polling endpoint)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct QbitServerState {
    connection_status: &'static str,
    dht_nodes: u64,
    dl_info_data: u64,
    dl_info_speed: u64,
    dl_rate_limit: u64,
    up_info_data: u64,
    up_info_speed: u64,
    up_rate_limit: u64,
    queueing: bool,
    use_alt_speed_limits: bool,
    refresh_interval: u64,
    free_space_on_disk: u64,
    global_ratio: String,
}

#[derive(Serialize)]
struct QbitMainData {
    rid: u64,
    full_update: bool,
    torrents: HashMap<String, QbitTorrentInfo>,
    categories: HashMap<String, QbitCategory>,
    tags: Vec<String>,
    server_state: QbitServerState,
}

#[derive(Deserialize, Default)]
struct SyncQuery {
    #[serde(default)]
    rid: u64,
}

/// `sync/maindata` — the primary polling endpoint for WebUI frontends and many
/// integrations. We do not track per-client deltas, so every response is a
/// `full_update` snapshot (a valid, if chattier, mode of the protocol); the
/// `rid` is echoed back incremented so clients keep polling.
async fn h_sync_maindata(
    State(state): State<Arc<QbitState>>,
    Query(query): Query<SyncQuery>,
) -> impl IntoResponse {
    let api = &state.api_state.api;
    let now = now_unix();

    let torrents: HashMap<String, QbitTorrentInfo> = api.session().with_torrents(|iter| {
        iter.map(|(_id, handle)| {
            let stats = handle.stats();
            let mut info = build_torrent_info(handle, &stats, now);
            info.tags = state.tags.tags_for(&info.hash);
            (info.hash.clone(), info)
        })
        .collect()
    });

    let categories: HashMap<String, QbitCategory> = api
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

    let session_stats = api.api_session_stats();
    let config = api.session().ratelimits.get_config();
    let downloaded = session_stats.counters.fetched_bytes;
    let uploaded = session_stats.counters.uploaded_bytes;
    let global_ratio = if downloaded == 0 {
        "0.00".to_string()
    } else {
        format!("{:.2}", uploaded as f64 / downloaded as f64)
    };

    let server_state = QbitServerState {
        connection_status: "connected",
        dht_nodes: 0,
        dl_info_data: downloaded,
        dl_info_speed: session_stats.download_speed.as_bytes(),
        dl_rate_limit: bps_to_u64(config.download_bps),
        up_info_data: uploaded,
        up_info_speed: session_stats.upload_speed.as_bytes(),
        up_rate_limit: bps_to_u64(config.upload_bps),
        queueing: false,
        use_alt_speed_limits: api.session().alt_speed_enabled(),
        refresh_interval: 1500,
        free_space_on_disk: 0,
        global_ratio,
    };

    axum::Json(QbitMainData {
        rid: query.rid.saturating_add(1),
        full_update: true,
        torrents,
        categories,
        tags: state.tags.all_tags(),
        server_state,
    })
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
        tags: QbitTags::default(),
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
        .route("/defaultSavePath", get(h_app_default_save_path))
        .route("/preferences", get(h_app_preferences))
        .route("/setPreferences", post(h_app_set_preferences));

    // Torrent endpoints
    let torrents_router = Router::new()
        .route("/info", get(h_torrents_info))
        .route("/count", get(h_torrents_count))
        .route("/properties", get(h_torrents_properties))
        .route("/files", get(h_torrents_files))
        .route("/trackers", get(h_torrents_trackers))
        .route("/webseeds", get(h_torrents_webseeds))
        .route("/export", get(h_torrents_export))
        .route("/pieceStates", get(h_torrents_piece_states))
        .route("/pieceHashes", get(h_torrents_piece_hashes))
        .route("/add", post(h_torrents_add))
        .route("/pause", post(h_torrents_pause))
        .route("/resume", post(h_torrents_resume))
        .route("/stop", post(h_torrents_stop))
        .route("/start", post(h_torrents_start))
        .route("/recheck", post(h_torrents_recheck))
        .route("/reannounce", post(h_torrents_reannounce))
        .route("/rename", post(h_torrents_rename))
        .route("/renameFile", post(h_torrents_rename_file))
        .route("/renameFolder", post(h_torrents_rename_folder))
        .route("/delete", post(h_torrents_delete))
        .route("/addTrackers", post(h_torrents_add_trackers))
        .route("/removeTrackers", post(h_torrents_remove_trackers))
        .route("/editTracker", post(h_torrents_edit_tracker))
        .route("/addPeers", post(h_torrents_add_peers))
        .route("/filePrio", post(h_torrents_file_prio))
        .route("/downloadLimit", get(h_torrents_download_limit))
        .route("/uploadLimit", get(h_torrents_upload_limit))
        .route("/setDownloadLimit", post(h_torrents_set_download_limit))
        .route("/setUploadLimit", post(h_torrents_set_upload_limit))
        .route("/setCategory", post(h_torrents_set_category))
        .route("/tags", get(h_torrents_tags))
        .route("/createTags", post(h_torrents_create_tags))
        .route("/deleteTags", post(h_torrents_delete_tags))
        .route("/addTags", post(h_torrents_add_tags))
        .route("/removeTags", post(h_torrents_remove_tags))
        .route("/setTags", post(h_torrents_set_tags))
        .route("/categories", get(h_categories))
        .route("/createCategory", post(h_create_category))
        .route("/editCategory", post(h_edit_category))
        .route("/removeCategories", post(h_remove_categories));

    // Transfer info + session speed limits
    let transfer_router = Router::new()
        .route("/info", get(h_transfer_info))
        .route("/downloadLimit", get(h_transfer_download_limit))
        .route("/uploadLimit", get(h_transfer_upload_limit))
        .route("/setDownloadLimit", post(h_transfer_set_download_limit))
        .route("/setUploadLimit", post(h_transfer_set_upload_limit))
        .route("/speedLimitsMode", get(h_transfer_speed_limits_mode))
        .route(
            "/toggleSpeedLimitsMode",
            post(h_transfer_toggle_speed_limits_mode),
        )
        .route(
            "/setSpeedLimitsMode",
            post(h_transfer_set_speed_limits_mode),
        )
        .route("/pauseSession", post(h_transfer_pause_session))
        .route("/resumeSession", post(h_transfer_resume_session));

    // Sync (main polling endpoint). Must stay nested inside protected_router so
    // it runs behind the SID-cookie auth layer.
    let sync_router = Router::new().route("/maindata", get(h_sync_maindata));

    let protected_router = Router::new()
        .nest("/app", app_router)
        .nest("/torrents", torrents_router)
        .nest("/transfer", transfer_router)
        .nest("/sync", sync_router)
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
    use std::{net::Ipv4Addr, path::Path, sync::Arc};

    use axum::response::IntoResponse;
    use bytes::Bytes;
    use http::StatusCode;

    use crate::{
        AddTorrent, AddTorrentOptions, Api, CreateTorrentOptions, ListenerMode, Session,
        SessionOptions, create_torrent,
        http_api::{HttpApi, HttpApiOptions},
        listen::ListenerOptions,
        spawn_utils::BlockingSpawner,
        torrent_state::TorrentStatsState,
    };

    use super::{
        QbitSessions, QbitState, QbitTags, h_app_preferences, h_app_set_preferences,
        h_torrents_recheck, matches_category, qbit_file_name,
    };

    async fn qbit_state() -> (Arc<QbitState>, Arc<Session>, tempfile::TempDir) {
        let output = tempfile::TempDir::with_prefix("qbit_preferences").unwrap();
        let session = Session::new_with_opts(
            output.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_local_service_discovery: true,
                listen: Some(ListenerOptions {
                    mode: ListenerMode::TcpOnly,
                    listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                    announce_port: Some(4241),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::new(
            session.clone(),
            None,
            #[cfg(feature = "tracing-subscriber-utils")]
            None,
        );
        let api_state = Arc::new(HttpApi::new(
            api,
            Some(HttpApiOptions {
                web_ui_port: Some(3031),
                ..Default::default()
            }),
        ));
        (
            Arc::new(QbitState {
                api_state,
                sessions: QbitSessions::new(),
                tags: QbitTags::default(),
            }),
            session,
            output,
        )
    }

    #[tokio::test]
    async fn preferences_reports_fixed_listener_and_runtime_announce_port() {
        let (state, session, _output) = qbit_state().await;

        let preferences = h_app_preferences(axum::extract::State(state)).await.0;

        assert_eq!(
            preferences.listen_port,
            session.listen_addr().unwrap().port()
        );
        assert_eq!(preferences.announce_port, 4241);
        assert_eq!(preferences.web_ui_port, 3031);
    }

    #[tokio::test]
    async fn set_preferences_updates_port_idempotently_without_rebinding_listener() {
        let (state, session, _output) = qbit_state().await;
        let listen_addr = session.listen_addr().unwrap();
        let body = Bytes::from_static(b"json=%7B%22announce_port%22%3A51234%7D");

        for _ in 0..2 {
            let response = h_app_set_preferences(axum::extract::State(state.clone()), body.clone())
                .await
                .into_response();
            assert_eq!(response.status(), StatusCode::OK);
        }

        assert_eq!(session.announce_port(), Some(51_234));
        assert_eq!(session.listen_addr(), Some(listen_addr));
    }

    #[tokio::test]
    async fn set_preferences_rejects_invalid_and_unsupported_fields() {
        let (state, session, _output) = qbit_state().await;
        let invalid_bodies = [
            "json=%7B%22announce_port%22%3A0%7D",
            "json=%7B%22announce_port%22%3A65536%7D",
            "json=%7B%22save_path%22%3A%22%2Ftmp%22%7D",
            "json=not-json",
            "announce_port=4241",
        ];

        for body in invalid_bodies {
            let response = h_app_set_preferences(
                axum::extract::State(state.clone()),
                Bytes::copy_from_slice(body.as_bytes()),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body={body}");
        }
        assert_eq!(session.announce_port(), Some(4241));
    }

    #[tokio::test]
    async fn recheck_endpoint_accepts_qbit_hash_form() {
        let (state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        let body = Bytes::from(format!("hashes={}", handle.info_hash().as_string()));
        assert_eq!(
            h_torrents_recheck(axum::extract::State(state), body).await,
            "Ok."
        );
        assert!(matches!(
            handle.stats().state,
            TorrentStatsState::Initializing
        ));
        handle.wait_until_initialized().await.unwrap();
        assert!(matches!(handle.stats().state, TorrentStatsState::Paused));
    }

    #[tokio::test]
    async fn per_torrent_rate_limits_round_trip_on_handle() {
        use std::num::NonZeroU32;

        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        // Defaults to unlimited.
        assert_eq!(handle.rate_limits().download_bps, None);
        assert_eq!(handle.rate_limits().upload_bps, None);

        // Each direction is set independently and persists on the override
        // (this torrent is paused, so there is no live limiter to update).
        handle.set_download_limit(NonZeroU32::new(4096));
        handle.set_upload_limit(NonZeroU32::new(8192));
        assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(4096));
        assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

        // Setting one leaves the other untouched.
        handle.set_download_limit(NonZeroU32::new(1024));
        assert_eq!(handle.rate_limits().download_bps, NonZeroU32::new(1024));
        assert_eq!(handle.rate_limits().upload_bps, NonZeroU32::new(8192));

        // None clears the limit.
        handle.set_download_limit(None);
        assert_eq!(handle.rate_limits().download_bps, None);
    }

    #[tokio::test]
    async fn rename_file_moves_on_disk_and_updates_metadata_when_paused() {
        let (_state, session, output) = qbit_state().await;
        std::fs::write(output.path().join("payload.bin"), vec![0x71; 32 * 1024]).unwrap();
        let torrent = create_torrent(
            output.path(),
            CreateTorrentOptions {
                piece_length: Some(16_384),
                ..Default::default()
            },
            &BlockingSpawner::new(1),
        )
        .await
        .unwrap()
        .as_bytes()
        .unwrap()
        .to_vec();
        let handle = session
            .add_torrent(
                AddTorrent::from_bytes(torrent),
                Some(AddTorrentOptions {
                    paused: true,
                    overwrite: true,
                    output_folder: Some(output.path().to_string_lossy().into_owned()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        handle.wait_until_initialized().await.unwrap();

        // Discover file 0's real on-disk location from the metadata.
        let old_rel = handle
            .with_metadata(|m| m.file_infos[0].relative_filename.clone())
            .unwrap();
        let old_abs = output.path().join(&old_rel);
        assert!(old_abs.exists(), "expected file at {old_abs:?}");

        // A path escaping the root is rejected before anything moves.
        assert!(
            handle
                .rename_files(&[(0, std::path::PathBuf::from("../escape.bin"))])
                .is_err()
        );
        assert!(old_abs.exists(), "rejected rename must not move anything");

        // A valid rename moves the file on disk and updates file_infos.
        let new_rel = std::path::PathBuf::from("renamed_dir/renamed.bin");
        handle.rename_files(&[(0, new_rel.clone())]).unwrap();
        assert_eq!(
            handle
                .with_metadata(|m| m.file_infos[0].relative_filename.clone())
                .unwrap(),
            new_rel
        );
        assert!(!old_abs.exists(), "old path should be gone");
        assert!(output.path().join(&new_rel).exists(), "new path should exist");

        // The display-name override is independent of file renames.
        assert!(handle.name().is_some());
        handle.set_display_name(Some("Custom Name".to_string()));
        assert_eq!(handle.name().as_deref(), Some("Custom Name"));
        handle.set_display_name(Some("   ".to_string()));
        assert_ne!(handle.name().as_deref(), Some("   "), "blank name clears override");
    }

    #[test]
    fn category_filter_supports_qbit_special_values() {
        assert!(matches_category("all", "Linux ISOs"));
        assert!(matches_category("all", ""));
        assert!(matches_category("uncategorized", ""));
        assert!(matches_category("", ""));
        assert!(!matches_category("uncategorized", "Linux ISOs"));
        assert!(matches_category("Linux ISOs", "Linux ISOs"));
        assert!(!matches_category("linux isos", "Linux ISOs"));
    }

    #[test]
    fn qbit_multi_file_names_include_torrent_root_for_save_path_imports() {
        assert_eq!(
            qbit_file_name(Some("release"), "disc/file.bin", true, true),
            Path::new("release").join("disc/file.bin").to_string_lossy()
        );
        assert_eq!(
            qbit_file_name(Some("release"), "single.bin", false, true),
            "single.bin"
        );
        assert_eq!(
            qbit_file_name(Some("release"), "only.bin", true, true),
            Path::new("release").join("only.bin").to_string_lossy()
        );
        assert_eq!(
            qbit_file_name(Some("release"), "disc/file.bin", true, false),
            "disc/file.bin"
        );
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

    #[test]
    fn parse_tags_trims_splits_and_drops_empties() {
        assert_eq!(
            super::parse_tags("tv, , radarr ,,sonarr"),
            vec!["tv".to_string(), "radarr".to_string(), "sonarr".to_string()]
        );
        assert!(super::parse_tags("").is_empty());
        assert!(super::parse_tags("  , ,").is_empty());
    }

    #[test]
    fn tag_store_add_remove_delete_and_set() {
        let tags = QbitTags::default();
        let a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();

        // createTags adds to the global set without touching any torrent.
        tags.create(&["tv".to_string(), "hd".to_string()]);
        assert_eq!(tags.all_tags(), vec!["hd".to_string(), "tv".to_string()]);
        assert_eq!(tags.tags_for(&a), "");

        // addTags associates tags with torrents (and registers new ones).
        tags.add_to(&[a.clone(), b.clone()], &["tv".to_string(), "new".to_string()]);
        assert!(tags.has_tag(&a, "tv"));
        assert!(tags.has_tag(&b, "new"));
        assert!(tags.all_tags().contains(&"new".to_string()));

        // removeTags with a specific tag only drops it from the given torrent.
        tags.remove_from(&[a.clone()], &["tv".to_string()]);
        assert!(!tags.has_tag(&a, "tv"));
        assert!(tags.has_tag(&a, "new"));
        assert!(tags.has_tag(&b, "tv"));

        // setTags replaces the whole tag set of a torrent.
        tags.set(&[b.clone()], &["only".to_string()]);
        assert_eq!(tags.tags_for(&b), "only");

        // deleteTags removes a tag globally and from every torrent.
        tags.delete(&["new".to_string()]);
        assert!(!tags.has_tag(&a, "new"));
        assert!(!tags.all_tags().contains(&"new".to_string()));

        // removeTags with an empty list clears all tags on the torrent.
        tags.add_to(&[a.clone()], &["x".to_string(), "y".to_string()]);
        tags.remove_from(&[a.clone()], &[]);
        assert_eq!(tags.tags_for(&a), "");
    }

    #[test]
    fn offset_limit_pagination_matches_qbittorrent_semantics() {
        use super::apply_offset_limit;
        let v = || vec![0, 1, 2, 3, 4];
        // Normal window.
        assert_eq!(apply_offset_limit(v(), 1, Some(2)), vec![1, 2]);
        // Offset past the end -> empty (not the whole list).
        assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
        assert_eq!(apply_offset_limit(v(), 99, Some(3)), Vec::<i32>::new());
        // Offset exactly at the end -> empty.
        assert_eq!(apply_offset_limit(v(), 5, None), Vec::<i32>::new());
        // limit == 0 means no limit.
        assert_eq!(apply_offset_limit(v(), 0, Some(0)), vec![0, 1, 2, 3, 4]);
        // No offset, no limit.
        assert_eq!(apply_offset_limit(v(), 0, None), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn tracker_status_maps_to_qbit_codes() {
        use super::qbit_tracker_status;
        use tracker_comms::TrackerAnnounceState::*;
        assert_eq!(qbit_tracker_status(Disabled), 0);
        assert_eq!(qbit_tracker_status(NotContacted), 1);
        assert_eq!(qbit_tracker_status(Working), 2);
        assert_eq!(qbit_tracker_status(Updating), 3);
        assert_eq!(qbit_tracker_status(Error), 4);
    }

    #[test]
    fn rate_limit_conversions_round_trip_and_treat_zero_as_unlimited() {
        use std::num::NonZeroU32;
        assert_eq!(super::limit_to_bps(0), None);
        assert_eq!(super::bps_to_u64(None), 0);
        assert_eq!(super::limit_to_bps(1024), NonZeroU32::new(1024));
        assert_eq!(super::bps_to_u64(NonZeroU32::new(1024)), 1024);
        // Values beyond u32 saturate rather than overflow.
        assert_eq!(super::limit_to_bps(u64::MAX), NonZeroU32::new(u32::MAX));
    }

    #[test]
    fn per_torrent_limit_treats_non_positive_as_unlimited() {
        use std::num::NonZeroU32;
        assert_eq!(super::parse_torrent_limit(0), None);
        assert_eq!(super::parse_torrent_limit(-1), None);
        assert_eq!(super::parse_torrent_limit(-999), None);
        assert_eq!(super::parse_torrent_limit(2048), NonZeroU32::new(2048));
        assert_eq!(super::parse_torrent_limit(i64::MAX), NonZeroU32::new(u32::MAX));
    }
}
