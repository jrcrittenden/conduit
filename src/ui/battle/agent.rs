//! Battle agent - tracks state for one side of the battle

use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::{AgentHandle, AgentType, SessionId, TokenUsage};
use crate::ui::components::{ChatView, ThinkingIndicator, TurnSummary};

/// Represents one agent in a battle (left or right side)
pub struct BattleAgent {
    /// Unique ID for this battle agent
    pub id: Uuid,

    /// Agent type (Claude or Codex)
    pub agent_type: AgentType,

    /// Selected model
    pub model: Option<String>,

    /// Chat view showing this agent's output
    pub chat_view: ChatView,

    /// Thinking indicator for processing state
    pub thinking_indicator: ThinkingIndicator,

    /// Current turn summary
    pub turn_summary: TurnSummary,

    /// Handle to the running agent process
    pub handle: Option<AgentHandle>,

    /// Agent session ID from the CLI
    pub session_id: Option<SessionId>,

    /// Process ID for interruption
    pub pid: Option<u32>,

    /// Input channel for streaming stdin
    pub input_tx: Option<mpsc::Sender<String>>,

    /// Whether currently processing
    pub is_processing: bool,

    /// Time when processing started
    pub started_at: Option<Instant>,

    /// Time when processing completed
    pub completed_at: Option<Instant>,

    /// Total token usage
    pub usage: TokenUsage,

    /// Files modified during this battle
    pub files_modified: Vec<String>,

    /// Number of tool calls made
    pub tool_calls: usize,

    /// Error message if agent failed
    pub error: Option<String>,

    /// Number of tools currently in flight
    pub tools_in_flight: usize,
}

impl BattleAgent {
    /// Create a new battle agent
    pub fn new(agent_type: AgentType) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_type,
            model: None,
            chat_view: ChatView::new(),
            thinking_indicator: ThinkingIndicator::new(),
            turn_summary: TurnSummary::new(),
            handle: None,
            session_id: None,
            pid: None,
            input_tx: None,
            is_processing: false,
            started_at: None,
            completed_at: None,
            usage: TokenUsage::default(),
            files_modified: Vec::new(),
            tool_calls: 0,
            error: None,
            tools_in_flight: 0,
        }
    }

    /// Start processing
    pub fn start_processing(&mut self) {
        self.is_processing = true;
        self.started_at = Some(Instant::now());
        self.completed_at = None;
        self.thinking_indicator.reset();
        self.turn_summary = TurnSummary::new();
        self.error = None;
        self.tools_in_flight = 0;
    }

    /// Stop processing (successful completion)
    pub fn complete(&mut self) {
        self.is_processing = false;
        self.completed_at = Some(Instant::now());

        // Finalize turn summary with duration
        if let Some(started) = self.started_at {
            self.turn_summary.duration_secs = started.elapsed().as_secs();
        }
    }

    /// Mark as failed
    pub fn fail(&mut self, error: String) {
        self.is_processing = false;
        self.completed_at = Some(Instant::now());
        self.error = Some(error);
    }

    /// Get elapsed time since start
    pub fn elapsed(&self) -> Duration {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => end.duration_since(start),
            (Some(start), None) => start.elapsed(),
            _ => Duration::ZERO,
        }
    }

    /// Get completion time (only if completed)
    pub fn completion_time(&self) -> Option<Duration> {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => Some(end.duration_since(start)),
            _ => None,
        }
    }

    /// Check if completed successfully
    pub fn is_complete(&self) -> bool {
        !self.is_processing && self.completed_at.is_some() && self.error.is_none()
    }

    /// Check if failed
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Add token usage
    pub fn add_usage(&mut self, usage: TokenUsage) {
        self.turn_summary.input_tokens = usage.input_tokens.max(0) as u64;
        self.turn_summary.output_tokens = usage.output_tokens.max(0) as u64;

        self.usage.input_tokens += usage.input_tokens;
        self.usage.output_tokens += usage.output_tokens;
        self.usage.cached_tokens += usage.cached_tokens;
        self.usage.total_tokens += usage.total_tokens;
    }

    /// Record a file modification
    pub fn record_file_change(&mut self, filename: String) {
        if !self.files_modified.contains(&filename) {
            self.files_modified.push(filename);
        }
    }

    /// Increment tool call count
    pub fn record_tool_call(&mut self) {
        self.tool_calls += 1;
    }

    /// Add streaming tokens
    pub fn add_streaming_tokens(&mut self, count: usize) {
        self.thinking_indicator.add_tokens(count);
    }

    /// Advance animation frame
    pub fn tick(&mut self) {
        if self.is_processing {
            self.thinking_indicator.tick();
        }
    }

    /// Calculate estimated cost based on model pricing
    pub fn estimated_cost(&self) -> f64 {
        // Rough pricing (per 1M tokens)
        let (input_price, output_price) = match self.agent_type {
            AgentType::Claude => (3.0, 15.0), // Sonnet pricing
            AgentType::Codex => (2.5, 10.0),  // GPT-4 pricing estimate
        };

        let input_cost = (self.usage.input_tokens as f64 / 1_000_000.0) * input_price;
        let output_cost = (self.usage.output_tokens as f64 / 1_000_000.0) * output_price;

        input_cost + output_cost
    }

    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self.agent_type {
            AgentType::Claude => "CLAUDE CODE",
            AgentType::Codex => "CODEX CLI",
        }
    }

    /// Get status emoji
    pub fn status_emoji(&self) -> &'static str {
        if self.has_error() {
            "❌"
        } else if self.is_complete() {
            "✅"
        } else if self.is_processing {
            "🔄"
        } else {
            "⏳"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_agent() {
        let agent = BattleAgent::new(AgentType::Claude);
        assert_eq!(agent.agent_type, AgentType::Claude);
        assert!(!agent.is_processing);
        assert!(!agent.is_complete());
    }

    #[test]
    fn test_processing_lifecycle() {
        let mut agent = BattleAgent::new(AgentType::Codex);

        agent.start_processing();
        assert!(agent.is_processing);
        assert!(agent.started_at.is_some());

        agent.complete();
        assert!(!agent.is_processing);
        assert!(agent.is_complete());
        assert!(agent.completion_time().is_some());
    }

    #[test]
    fn test_failure() {
        let mut agent = BattleAgent::new(AgentType::Claude);

        agent.start_processing();
        agent.fail("Something went wrong".into());

        assert!(agent.has_error());
        assert!(!agent.is_complete());
        assert_eq!(agent.status_emoji(), "❌");
    }
}
