use axum::{extract::Query, extract::State, Json};
use serde::{Deserialize, Serialize};

use crate::agent::events::AgentEvent;
use crate::repro::runtime::{self, ReplayStateSnapshot};
use crate::repro::tape::{RecordedInput, ReproTapeEntry};
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

#[derive(Debug, Deserialize)]
pub struct ReproEventsQuery {
    pub session_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ReproEventSummary {
    pub seq: u64,
    pub ts_ms: u64,
    pub kind: String,
    pub session_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReproEventsResponse {
    pub events: Vec<ReproEventSummary>,
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

fn agent_event_detail(event: &AgentEvent) -> Option<String> {
    match event {
        AgentEvent::AssistantMessage(msg) => Some(format!("final={}", msg.is_final)),
        AgentEvent::AssistantReasoning(msg) => Some(format!("chars={}", msg.text.len())),
        AgentEvent::ToolStarted(tool) => Some(format!("tool={}", tool.tool_name)),
        AgentEvent::ToolCompleted(tool) => {
            Some(format!("tool={} success={}", tool.tool_id, tool.success))
        }
        AgentEvent::TurnFailed(ev) => Some(ev.error.clone()),
        AgentEvent::Error(ev) => Some(ev.message.clone()),
        _ => None,
    }
}

fn input_detail(input: &RecordedInput) -> (String, Option<String>) {
    match input {
        RecordedInput::ClaudeJsonl { .. } => ("input_claude_jsonl".into(), None),
        RecordedInput::CodexPrompt { .. } => ("input_codex_prompt".into(), None),
        RecordedInput::OpencodeQuestion { request_id, .. } => (
            "input_opencode_question".into(),
            Some(format!("request_id={}", request_id)),
        ),
    }
}

pub async fn get_repro_events(
    State(_state): State<WebAppState>,
    Query(query): Query<ReproEventsQuery>,
) -> Result<Json<ReproEventsResponse>, WebError> {
    let tape = runtime::replay_tape()
        .map_err(|err| WebError::Internal(format!("failed to load repro tape: {err}")))?
        .ok_or_else(|| WebError::Conflict("replay tape not available".into()))?;
    let limit = query.limit.unwrap_or(200).min(2000);
    let session_id = query.session_id.as_deref();

    let mut events: Vec<ReproEventSummary> = tape
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ReproTapeEntry::AgentEvent {
                seq,
                ts_ms,
                session_id: entry_session,
                event,
            } => {
                if session_id.is_some_and(|id| id != entry_session) {
                    return None;
                }
                Some(ReproEventSummary {
                    seq: *seq,
                    ts_ms: *ts_ms,
                    kind: event.event_type_name().to_string(),
                    session_id: Some(entry_session.clone()),
                    detail: agent_event_detail(event),
                })
            }
            ReproTapeEntry::AgentInput {
                seq,
                ts_ms,
                session_id: entry_session,
                input,
            } => {
                if session_id.is_some_and(|id| id != entry_session) {
                    return None;
                }
                let (kind, detail) = input_detail(input);
                Some(ReproEventSummary {
                    seq: *seq,
                    ts_ms: *ts_ms,
                    kind,
                    session_id: Some(entry_session.clone()),
                    detail,
                })
            }
            ReproTapeEntry::Note {
                seq,
                ts_ms,
                message,
            } => {
                if session_id.is_some() {
                    return None;
                }
                Some(ReproEventSummary {
                    seq: *seq,
                    ts_ms: *ts_ms,
                    kind: "note".into(),
                    session_id: None,
                    detail: Some(message.clone()),
                })
            }
        })
        .collect();

    if events.len() > limit {
        events.sort_by_key(|ev| ev.seq);
        events = events.into_iter().rev().take(limit).collect();
        events.sort_by_key(|ev| ev.seq);
    }

    Ok(Json(ReproEventsResponse { events }))
}

pub async fn post_repro_control(
    State(_state): State<WebAppState>,
    Json(payload): Json<ReproControlRequest>,
) -> Result<Json<ReproStateResponse>, WebError> {
    let controller = runtime::replay_controller()
        .ok_or_else(|| WebError::Conflict("replay controller not available".into()))?;
    let snapshot = runtime::replay_state_snapshot().unwrap_or_default();

    match payload.action.to_ascii_lowercase().as_str() {
        "pause" => controller.pause(),
        "resume" => controller.resume(),
        "step" => controller.step(),
        "seek" => {
            let seq = payload
                .seq
                .ok_or_else(|| WebError::BadRequest("missing seq for seek".into()))?;
            if seq < snapshot.current_seq {
                return Err(WebError::BadRequest(
                    "seek can only move forward; restart replay to go back".into(),
                ));
            }
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
