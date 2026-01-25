use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::repro::runtime::{self, ReplayStateSnapshot};
use crate::web::error::WebError;
use crate::web::state::WebAppState;

#[derive(Debug, Serialize)]
pub struct ReproStateResponse {
    pub mode: String,
    pub paused: bool,
    pub current_seq: u64,
    pub max_seq: Option<u64>,
    pub total_events: u64,
}

#[derive(Debug, Deserialize)]
pub struct ReproControlRequest {
    pub action: String,
    pub seq: Option<u64>,
}

fn snapshot_to_response(mode: &str, snapshot: ReplayStateSnapshot) -> ReproStateResponse {
    ReproStateResponse {
        mode: mode.to_string(),
        paused: snapshot.paused,
        current_seq: snapshot.current_seq,
        max_seq: snapshot.max_seq,
        total_events: snapshot.total_events,
    }
}

pub async fn get_repro_state(
    State(_state): State<WebAppState>,
) -> Result<Json<ReproStateResponse>, WebError> {
    let mode = runtime::mode();
    let mode_label = match mode {
        runtime::ReproMode::Off => "off",
        runtime::ReproMode::Record => "record",
        runtime::ReproMode::Replay { .. } => "replay",
    };

    if let Some(snapshot) = runtime::replay_state_snapshot() {
        return Ok(Json(snapshot_to_response(mode_label, snapshot)));
    }

    Ok(Json(ReproStateResponse {
        mode: mode_label.to_string(),
        paused: false,
        current_seq: 0,
        max_seq: None,
        total_events: 0,
    }))
}

pub async fn post_repro_control(
    State(_state): State<WebAppState>,
    Json(payload): Json<ReproControlRequest>,
) -> Result<Json<ReproStateResponse>, WebError> {
    let controller = runtime::replay_controller()
        .ok_or_else(|| WebError::Conflict("replay controller not available".into()))?;

    match payload.action.to_ascii_lowercase().as_str() {
        "pause" => controller.pause(),
        "resume" => controller.resume(),
        "step" => controller.step(),
        "seek" => {
            let seq = payload
                .seq
                .ok_or_else(|| WebError::BadRequest("missing seq for seek".into()))?;
            controller.seek_forward(seq);
        }
        other => {
            return Err(WebError::BadRequest(format!(
                "unsupported replay action: {}",
                other
            )))
        }
    }

    let mode_label = match runtime::mode() {
        runtime::ReproMode::Off => "off",
        runtime::ReproMode::Record => "record",
        runtime::ReproMode::Replay { .. } => "replay",
    };

    let snapshot = runtime::replay_state_snapshot().unwrap_or_default();
    Ok(Json(snapshot_to_response(mode_label, snapshot)))
}
