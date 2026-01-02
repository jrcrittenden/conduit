use crate::agent::{AgentEvent, AgentType};

/// Application-level events
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal input event
    Input(crossterm::event::Event),

    /// Agent event from a session
    Agent {
        tab_index: usize,
        event: AgentEvent,
    },

    /// User submitted a prompt
    PromptSubmit {
        tab_index: usize,
        prompt: String,
    },

    /// Request to create a new tab
    NewTab(AgentType),

    /// Request to close a tab
    CloseTab(usize),

    /// Request to switch to a tab
    SwitchTab(usize),

    /// Agent selection dialog requested
    ShowAgentSelector,

    /// Agent selected from dialog
    AgentSelected(AgentType),

    /// Request to interrupt current agent
    InterruptAgent(usize),

    /// Tick event for animations/updates
    Tick,

    /// Request to quit the application
    Quit,

    /// Error occurred
    Error(String),
}

/// Input mode for the application
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Normal mode - input focused
    #[default]
    Normal,
    /// Selecting agent for new tab
    SelectingAgent,
    /// Scrolling through chat history
    Scrolling,
}

/// View mode for the main content area
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Standard chat view
    #[default]
    Chat,
    /// Raw events debug view
    RawEvents,
}
