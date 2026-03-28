use axum::{extract::State, response::IntoResponse};
use futures::TryStreamExt;
use tracing::debug;

use super::ApiState;
use crate::api::Result;

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/rust_log",
    request_body(content = String, description = "New RUST_LOG value"),
    responses(
        (status = 200, description = "RUST_LOG updated")
    )
))]
pub async fn h_set_rust_log(
    State(state): State<ApiState>,
    new_value: String,
) -> Result<impl IntoResponse> {
    state.api.api_set_rust_log(new_value).map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/stream_logs",
    responses(
        (status = 200, description = "Continuous log stream (Server-Sent Events)")
    )
))]
pub async fn h_stream_logs(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    let s = state.api.api_log_lines_stream()?.map_err(|e| {
        debug!(error=%e, "stream_logs");
        e
    });
    Ok(axum::body::Body::from_stream(s))
}
