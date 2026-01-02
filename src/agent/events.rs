use serde::{Deserialize, Serialize};

use crate::agent::session::SessionId;

/// Unified event type emitted by all agents
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    /// Session initialized with session ID
    SessionInit(SessionInitEvent),

    /// Turn/task started
    TurnStarted,

    /// Turn/task completed
    TurnCompleted(TurnCompletedEvent),

    /// Turn failed with error
    TurnFailed(TurnFailedEvent),

    /// Assistant text message
    AssistantMessage(AssistantMessageEvent),

    /// Assistant reasoning/thinking
    AssistantReasoning(ReasoningEvent),

    /// Tool use started
    ToolStarted(ToolStartedEvent),

    /// Tool use completed
    ToolCompleted(ToolCompletedEvent),

    /// File operation
    FileChanged(FileChangedEvent),

    /// Command execution output
    CommandOutput(CommandOutputEvent),

    /// Token usage update
    TokenUsage(TokenUsageEvent),

    /// Context compaction triggered
    ContextCompaction(ContextCompactionEvent),

    /// Error event
    Error(ErrorEvent),

    /// Raw/unknown event (for forward compatibility)
    Raw { data: serde_json::Value },
}

impl AgentEvent {
    /// Get a human-readable event type name for display
    pub fn event_type_name(&self) -> &'static str {
        match self {
            AgentEvent::SessionInit(_) => "SessionInit",
            AgentEvent::TurnStarted => "TurnStarted",
            AgentEvent::TurnCompleted(_) => "TurnCompleted",
            AgentEvent::TurnFailed(_) => "TurnFailed",
            AgentEvent::AssistantMessage(_) => "AssistantMessage",
            AgentEvent::AssistantReasoning(_) => "AssistantReasoning",
            AgentEvent::ToolStarted(_) => "ToolStarted",
            AgentEvent::ToolCompleted(_) => "ToolCompleted",
            AgentEvent::FileChanged(_) => "FileChanged",
            AgentEvent::CommandOutput(_) => "CommandOutput",
            AgentEvent::TokenUsage(_) => "TokenUsage",
            AgentEvent::ContextCompaction(_) => "ContextCompaction",
            AgentEvent::Error(_) => "Error",
            AgentEvent::Raw { .. } => "Raw",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInitEvent {
    pub session_id: SessionId,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCompletedEvent {
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFailedEvent {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessageEvent {
    pub text: String,
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningEvent {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStartedEvent {
    pub tool_name: String,
    pub tool_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCompletedEvent {
    pub tool_id: String,
    pub success: bool,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangedEvent {
    pub path: String,
    pub operation: FileOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileOperation {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutputEvent {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub is_streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageEvent {
    pub usage: TokenUsage,
    pub context_window: Option<i64>,
    pub usage_percent: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompactionEvent {
    pub reason: String,
    pub tokens_before: i64,
    pub tokens_after: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEvent {
    pub message: String,
    pub is_fatal: bool,
}
