use axum::{extract::State, response::IntoResponse};

use super::ApiState;
use crate::api::Result;

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/dht/stats",
    responses(
        (status = 200, description = "DHT statistics")
    )
))]
pub async fn h_dht_stats(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    state.api.api_dht_stats().map(axum::Json)
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/dht/table",
    responses(
        (status = 200, description = "DHT routing table")
    )
))]
pub async fn h_dht_table(State(state): State<ApiState>) -> Result<impl IntoResponse + 'static> {
    state.api.api_dht_table().map(axum::Json)
}
