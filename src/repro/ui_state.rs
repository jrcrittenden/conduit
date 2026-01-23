use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::web::handlers::ui_state::WebUiState;

/// Minimal UI state snapshot to help force the UI into a known "status".
///
/// Note: Most persistent UI state already lives in SQLite (app_state + session_tabs).
/// This is for *ephemeral* view tweaks that are helpful during repro/replay, and it
/// is intentionally small for v1.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiStateSnapshot {
    pub tui: Option<TuiUiStateSnapshot>,
    pub web: Option<WebUiState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiUiStateSnapshot {
    pub active_session_id: Option<Uuid>,
    pub active_tab_index: usize,
    pub sidebar_visible: bool,
}
