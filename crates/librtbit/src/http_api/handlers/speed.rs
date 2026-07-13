use axum::{Json, extract::State, response::IntoResponse};
use serde::Deserialize;

use super::ApiState;
use crate::{
    alt_speed::{AltSpeedConfig, AltSpeedSchedule},
    api::{EmptyJsonResponse, Result},
};

#[derive(Deserialize)]
pub struct ToggleAltSpeedRequest {
    pub enabled: bool,
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/speed/alt",
    responses((status = 200, description = "Alternative speed limits status"))
))]
pub async fn h_get_alt_speed(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(state.api.session().alt_speed_status()))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/speed/alt",
    responses((status = 200, description = "Toggle alternative speed limits"))
))]
pub async fn h_toggle_alt_speed(
    State(state): State<ApiState>,
    Json(req): Json<ToggleAltSpeedRequest>,
) -> Result<impl IntoResponse> {
    state.api.session().set_alt_speed_enabled(req.enabled);
    Ok(Json(EmptyJsonResponse {}))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/speed/alt/config",
    responses((status = 200, description = "Set alternative speed limit rates"))
))]
pub async fn h_set_alt_speed_config(
    State(state): State<ApiState>,
    Json(config): Json<AltSpeedConfig>,
) -> Result<impl IntoResponse> {
    state.api.session().set_alt_speed_config(config);
    Ok(Json(EmptyJsonResponse {}))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    get,
    path = "/speed/schedule",
    responses((status = 200, description = "Alternative speed limits schedule"))
))]
pub async fn h_get_speed_schedule(State(state): State<ApiState>) -> Result<impl IntoResponse> {
    Ok(Json(state.api.session().alt_speed_schedule()))
}

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    path = "/speed/schedule",
    responses((status = 200, description = "Set alternative speed limits schedule"))
))]
pub async fn h_set_speed_schedule(
    State(state): State<ApiState>,
    Json(schedule): Json<AltSpeedSchedule>,
) -> Result<impl IntoResponse> {
    state.api.session().set_alt_speed_schedule(schedule);
    Ok(Json(EmptyJsonResponse {}))
}
