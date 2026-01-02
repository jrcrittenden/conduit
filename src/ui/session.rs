use serde_json::Value;
use uuid::Uuid;

use crate::agent::{AgentHandle, AgentType, SessionId, TokenUsage};
use crate::ui::components::{
    ChatView, EventDirection, InputBox, ProcessingState, RawEventsView, StatusBar,
    ThinkingIndicator, TurnSummary,
};

/// Represents a single agent session (one tab)
pub struct AgentSession {
    /// Unique identifier for this session
    pub id: Uuid,
    /// Type of agent (Claude or Codex)
    pub agent_type: AgentType,
    /// Chat view component
    pub chat_view: ChatView,
    /// Raw events view (debug)
    pub raw_events_view: RawEventsView,
    /// Input box component
    pub input_box: InputBox,
    /// Status bar component
    pub status_bar: StatusBar,
    /// Thinking indicator (shown while processing)
    pub thinking_indicator: ThinkingIndicator,
    /// Current turn summary (built during processing)
    pub current_turn_summary: TurnSummary,
    /// Handle to the running agent process (if any)
    pub agent_handle: Option<AgentHandle>,
    /// Agent session ID (from the agent itself)
    pub agent_session_id: Option<SessionId>,
    /// Whether the agent is currently processing
    pub is_processing: bool,
    /// Accumulated token usage
    pub total_usage: TokenUsage,
    /// Turn count
    pub turn_count: u32,
}

impl AgentSession {
    pub fn new(agent_type: AgentType) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_type,
            chat_view: ChatView::new(),
            raw_events_view: RawEventsView::new(),
            input_box: InputBox::new(),
            status_bar: StatusBar::new(agent_type),
            thinking_indicator: ThinkingIndicator::new(),
            current_turn_summary: TurnSummary::new(),
            agent_handle: None,
            agent_session_id: None,
            is_processing: false,
            total_usage: TokenUsage::default(),
            turn_count: 0,
        }
    }

    /// Get display name for the tab
    pub fn tab_name(&self) -> String {
        if let Some(ref session_id) = self.agent_session_id {
            let id_short = &session_id.as_str()[..8.min(session_id.as_str().len())];
            format!("{} ({})", self.agent_type.as_str(), id_short)
        } else {
            format!("{} (new)", self.agent_type.as_str())
        }
    }

    /// Update status bar with current state
    pub fn update_status(&mut self) {
        self.status_bar.set_session_id(self.agent_session_id.clone());
        self.status_bar.set_token_usage(self.total_usage.clone());
        self.status_bar.set_processing(self.is_processing);
    }

    /// Add token usage from a turn
    pub fn add_usage(&mut self, usage: TokenUsage) {
        // Update turn summary with this turn's tokens
        self.current_turn_summary.input_tokens = usage.input_tokens.max(0) as u64;
        self.current_turn_summary.output_tokens = usage.output_tokens.max(0) as u64;

        // Accumulate total usage
        self.total_usage.input_tokens += usage.input_tokens;
        self.total_usage.output_tokens += usage.output_tokens;
        self.total_usage.cached_tokens += usage.cached_tokens;
        self.total_usage.total_tokens += usage.total_tokens;
        self.turn_count += 1;
        self.update_status();
    }

    /// Start processing (resets thinking indicator and turn summary)
    pub fn start_processing(&mut self) {
        self.is_processing = true;
        self.thinking_indicator.reset();
        self.current_turn_summary = TurnSummary::new();
        self.update_status();
    }

    /// Stop processing and finalize turn summary
    pub fn stop_processing(&mut self) {
        self.is_processing = false;
        // Finalize the turn summary with duration and tokens
        let duration = self.thinking_indicator.elapsed();
        self.current_turn_summary.duration_secs = duration.as_secs();
        self.update_status();
    }

    /// Record a file change for the current turn
    pub fn record_file_change(&mut self, filename: impl Into<String>, additions: usize, deletions: usize) {
        self.current_turn_summary.add_file(filename, additions, deletions);
    }

    /// Add tokens to the thinking indicator
    pub fn add_streaming_tokens(&mut self, count: usize) {
        self.thinking_indicator.add_tokens(count);
    }

    /// Set the current processing state
    pub fn set_processing_state(&mut self, state: ProcessingState) {
        self.thinking_indicator.set_state(state);
    }

    /// Advance spinner animation (called on tick)
    pub fn tick(&mut self) {
        self.status_bar.tick();
        if self.is_processing {
            self.thinking_indicator.tick();
        }
    }

    /// Record a raw event for the debug view
    pub fn record_raw_event(
        &mut self,
        direction: EventDirection,
        event_type: impl Into<String>,
        raw_json: Value,
    ) {
        self.raw_events_view
            .push_event(direction, event_type, raw_json);
    }
}
