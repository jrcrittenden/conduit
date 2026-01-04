use std::path::PathBuf;

use crate::agent::{AgentEvent, AgentType};
use crate::git::PrPreflightResult;
use uuid::Uuid;

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

    /// Agent event stream ended (process exited)
    AgentStreamEnded { tab_index: usize },

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

    /// Toggle sidebar visibility
    ToggleSidebar,

    /// Open a workspace (creates/switches to tab)
    OpenWorkspace(Uuid),

    /// Refresh sidebar data from database
    RefreshSidebar,

    /// Tick event for animations/updates
    Tick,

    /// Request to quit the application
    Quit,

    /// Error occurred
    Error(String),

    /// PR preflight check completed
    PrPreflightCompleted {
        tab_index: usize,
        working_dir: PathBuf,
        result: PrPreflightResult,
    },
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
    /// Navigating sidebar
    SidebarNavigation,
    /// Adding a repository (custom path)
    AddingRepository,
    /// Selecting model for current session
    SelectingModel,
    /// Setting base projects directory
    SettingBaseDir,
    /// Picking a project from the list
    PickingProject,
    /// Showing a confirmation dialog
    Confirming,
    /// Removing a project (showing spinner)
    RemovingProject,
    /// Showing an error dialog
    ShowingError,
    /// Command mode (typing :command)
    Command,
    /// Showing help dialog
    ShowingHelp,
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
