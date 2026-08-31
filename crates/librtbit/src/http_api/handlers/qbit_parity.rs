//! qBittorrent WebUI API parity checker.
//!
//! `qbit_parity_spec.json` is a machine-readable inventory of the upstream
//! qBittorrent WebUI API v2 surface (extracted from the qBittorrent sources),
//! annotated with the parity status of each endpoint in our compat layer
//! (`qbit_compat.rs`). These tests keep the spec and the actual router from
//! drifting apart:
//!
//! - every endpoint marked `full` or `partial` must be routed;
//! - every endpoint marked `missing` or `out_of_scope` must return 404;
//! - every route literal in `qbit_compat.rs` must be tracked in the spec.
//!
//! When implementing a new compat endpoint, flip its `status` in the spec
//! (and drop a note about any semantic gaps). To refresh the upstream
//! inventory against a newer qBittorrent checkout, run
//! `scripts/refresh-qbit-parity-spec.py <path-to-qbittorrent-checkout>`.

use std::{collections::HashSet, net::Ipv4Addr, sync::Arc};

use http::StatusCode;
use serde::Deserialize;
use tower::ServiceExt;

use crate::{
    Api, ListenerMode, Session, SessionOptions,
    http_api::{HttpApi, HttpApiOptions},
    listen::ListenerOptions,
};

const SPEC_JSON: &str = include_str!("qbit_parity_spec.json");
const COMPAT_SOURCE: &str = include_str!("qbit_compat.rs");

#[derive(Deserialize)]
struct Spec {
    upstream: UpstreamInfo,
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
struct UpstreamInfo {
    webapi_version: String,
}

#[derive(Deserialize)]
struct Endpoint {
    /// `controller/action`, e.g. `torrents/info`.
    endpoint: String,
    method: String,
    status: Status,
    /// False for legacy endpoints we serve that upstream has since removed.
    #[serde(default = "default_true")]
    upstream: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum Status {
    /// Routed, semantics match qBittorrent closely enough for real clients.
    Full,
    /// Routed, but with stubbed fields or semantic gaps (see notes).
    Partial,
    /// Not routed; candidate for future parity work.
    Missing,
    /// Not routed by explicit decision.
    OutOfScope,
}

impl Status {
    fn is_routed(self) -> bool {
        matches!(self, Status::Full | Status::Partial)
    }
}

fn parse_spec() -> Spec {
    serde_json::from_str(SPEC_JSON).expect("qbit_parity_spec.json must be valid JSON")
}

async fn make_router() -> (axum::Router, Arc<Session>, tempfile::TempDir) {
    let output = tempfile::TempDir::with_prefix("qbit_parity").unwrap();
    let session = Session::new_with_opts(
        output.path().to_owned(),
        SessionOptions {
            disable_dht: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
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
    let api_state = Arc::new(HttpApi::new(api, Some(HttpApiOptions::default())));
    (
        super::qbit_compat::make_qbit_router(api_state),
        session,
        output,
    )
}

#[test]
fn spec_is_well_formed() {
    let spec = parse_spec();
    assert!(!spec.upstream.webapi_version.is_empty());
    let mut seen = HashSet::new();
    for ep in &spec.endpoints {
        assert!(
            seen.insert(ep.endpoint.as_str()),
            "duplicate spec entry: {}",
            ep.endpoint
        );
        assert!(
            matches!(ep.method.as_str(), "GET" | "POST"),
            "{}: unexpected method {}",
            ep.endpoint,
            ep.method
        );
        assert!(
            ep.endpoint.split('/').count() == 2,
            "{}: endpoint must be controller/action",
            ep.endpoint
        );
        if !ep.upstream {
            assert!(
                ep.status.is_routed(),
                "{}: non-upstream (legacy) entries only make sense if we route them",
                ep.endpoint
            );
        }
    }
}

/// Probes every spec endpoint against the real compat router and fails on any
/// mismatch between the declared parity status and what is actually routed.
#[tokio::test]
async fn spec_matches_router() {
    let spec = parse_spec();
    let (router, _session, _output) = make_router().await;

    let mut violations = Vec::new();
    for ep in &spec.endpoints {
        let request = http::Request::builder()
            .method(ep.method.as_str())
            .uri(format!("/{}", ep.endpoint))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::empty())
            .unwrap();
        let status = router.clone().oneshot(request).await.unwrap().status();
        // Anything but 404/405 (including 400 for probes lacking required
        // params) proves the route exists with the expected method.
        let routed =
            status != StatusCode::NOT_FOUND && status != StatusCode::METHOD_NOT_ALLOWED;
        if routed != ep.status.is_routed() {
            violations.push(format!(
                "{} {} is marked {:?} in qbit_parity_spec.json but the router returned {status}",
                ep.method, ep.endpoint, ep.status
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "qbit compat router and qbit_parity_spec.json disagree; \
         update the spec status (or the router) for:\n{}",
        violations.join("\n")
    );
}

/// Every `.route("...")` literal in qbit_compat.rs must correspond to a spec
/// entry marked as routed, so new compat routes can't land untracked.
#[test]
fn every_compat_route_is_tracked_in_spec() {
    let spec = parse_spec();
    let routed_actions: HashSet<&str> = spec
        .endpoints
        .iter()
        .filter(|ep| ep.status.is_routed())
        .filter_map(|ep| ep.endpoint.split_once('/').map(|(_, action)| action))
        .collect();

    let mut untracked = Vec::new();
    for (idx, _) in COMPAT_SOURCE.match_indices(".route(\"/") {
        let start = idx + ".route(\"/".len();
        let action = COMPAT_SOURCE[start..]
            .split('"')
            .next()
            .expect("unterminated route literal");
        if !routed_actions.contains(action) {
            untracked.push(action);
        }
    }

    assert!(
        untracked.is_empty(),
        "routes in qbit_compat.rs with no full/partial entry in qbit_parity_spec.json: {untracked:?}"
    );
}

/// Not an assertion — prints the parity scoreboard (visible with
/// `cargo test -p swarmforge qbit_parity -- --nocapture`).
#[test]
fn parity_summary() {
    let spec = parse_spec();
    let upstream: Vec<&Endpoint> = spec.endpoints.iter().filter(|ep| ep.upstream).collect();
    let count = |status: Status| upstream.iter().filter(|ep| ep.status == status).count();
    let (full, partial, missing, oos) = (
        count(Status::Full),
        count(Status::Partial),
        count(Status::Missing),
        count(Status::OutOfScope),
    );
    let implemented = full + partial;
    eprintln!(
        "qBittorrent WebUI API v{} parity: {implemented}/{} endpoints routed \
         ({full} full, {partial} partial, {missing} missing, {oos} out of scope)",
        spec.upstream.webapi_version,
        upstream.len(),
    );
}
