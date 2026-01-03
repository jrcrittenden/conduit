use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::agent::{
    load_claude_history, load_codex_history, AgentEvent, AgentRunner, AgentStartConfig, AgentType,
    ClaudeCodeRunner, CodexCliRunner, SessionId,
};
use crate::config::Config;
use crate::data::{
    AppStateDao, Database, Repository, RepositoryDao, SessionTab, SessionTabDao, WorkspaceDao,
};
use crate::git::WorktreeManager;
use crate::ui::components::{
    AddRepoDialog, AddRepoDialogState, AgentSelector, AgentSelectorState, BaseDirDialog,
    BaseDirDialogState, ChatMessage, ConfirmationDialog, ConfirmationDialogState,
    ConfirmationType, EventDirection, GlobalFooter, ModelSelector, ModelSelectorState,
    ProcessingState, ProjectPicker, ProjectPickerState, Sidebar, SidebarData, SidebarState,
    SplashScreen, TabBar,
};
use crate::ui::events::{AppEvent, InputMode, ViewMode};
use crate::ui::session::AgentSession;
use crate::ui::tab_manager::TabManager;

/// Main application state
pub struct App {
    /// Application configuration
    config: Config,
    /// Whether the app should quit
    should_quit: bool,
    /// Tab manager for multiple sessions
    tab_manager: TabManager,
    /// Current input mode
    input_mode: InputMode,
    /// Current view mode (Chat or RawEvents)
    view_mode: ViewMode,
    /// Agent runners
    claude_runner: Arc<ClaudeCodeRunner>,
    codex_runner: Arc<CodexCliRunner>,
    /// Event channel sender
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Event channel receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    /// Tick counter for spinner animation
    tick_count: u32,
    /// Splash screen (shown when no tabs)
    splash_screen: SplashScreen,
    /// Repository DAO
    repo_dao: Option<RepositoryDao>,
    /// Workspace DAO
    workspace_dao: Option<WorkspaceDao>,
    /// App state DAO (for persisting app settings)
    app_state_dao: Option<AppStateDao>,
    /// Session tab DAO (for persisting open tabs)
    session_tab_dao: Option<SessionTabDao>,
    /// Worktree manager
    worktree_manager: WorktreeManager,
    /// Sidebar state
    sidebar_state: SidebarState,
    /// Sidebar data (repositories and workspaces)
    sidebar_data: SidebarData,
    /// Add repository dialog state (for custom paths)
    add_repo_dialog_state: AddRepoDialogState,
    /// Model selector dialog state
    model_selector_state: ModelSelectorState,
    /// Agent selector dialog state
    agent_selector_state: AgentSelectorState,
    /// Whether to show the first-time splash screen (repo count < 1)
    show_first_time_splash: bool,
    /// Base directory dialog state
    base_dir_dialog_state: BaseDirDialogState,
    /// Project picker state
    project_picker_state: ProjectPickerState,
    /// Confirmation dialog state (for archive, delete, etc.)
    confirmation_dialog_state: ConfirmationDialogState,
    // Layout areas for mouse hit-testing
    /// Sidebar area (if visible)
    sidebar_area: Option<Rect>,
    /// Tab bar area
    tab_bar_area: Option<Rect>,
    /// Chat/content area
    chat_area: Option<Rect>,
    /// Input box area
    input_area: Option<Rect>,
    /// Status bar area
    status_bar_area: Option<Rect>,
    /// Footer area
    footer_area: Option<Rect>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Initialize database and DAOs
        let (repo_dao, workspace_dao, app_state_dao, session_tab_dao) =
            match Database::open_default() {
                Ok(db) => {
                    let repo_dao = RepositoryDao::new(db.connection());
                    let workspace_dao = WorkspaceDao::new(db.connection());
                    let app_state_dao = AppStateDao::new(db.connection());
                    let session_tab_dao = SessionTabDao::new(db.connection());
                    (
                        Some(repo_dao),
                        Some(workspace_dao),
                        Some(app_state_dao),
                        Some(session_tab_dao),
                    )
                }
                Err(e) => {
                    eprintln!("Warning: Failed to open database: {}", e);
                    (None, None, None, None)
                }
            };

        // Initialize worktree manager with managed directory (~/.conduit/worktrees)
        let worktree_manager = WorktreeManager::with_managed_dir(crate::util::worktrees_dir());

        let mut app = Self {
            config: config.clone(),
            should_quit: false,
            tab_manager: TabManager::new(config.max_tabs),
            input_mode: InputMode::Normal,
            view_mode: ViewMode::Chat,
            claude_runner: Arc::new(ClaudeCodeRunner::new()),
            codex_runner: Arc::new(CodexCliRunner::new()),
            event_tx,
            event_rx,
            tick_count: 0,
            splash_screen: SplashScreen::new(),
            repo_dao,
            workspace_dao,
            app_state_dao,
            session_tab_dao,
            worktree_manager,
            sidebar_state: SidebarState::new(),
            sidebar_data: SidebarData::new(),
            add_repo_dialog_state: AddRepoDialogState::new(),
            model_selector_state: ModelSelectorState::default(),
            agent_selector_state: AgentSelectorState::new(),
            show_first_time_splash: true, // Will be set properly in restore_session_state
            base_dir_dialog_state: BaseDirDialogState::new(),
            project_picker_state: ProjectPickerState::new(),
            confirmation_dialog_state: ConfirmationDialogState::new(),
            // Layout areas initialized to None, will be set during draw()
            sidebar_area: None,
            tab_bar_area: None,
            chat_area: None,
            input_area: None,
            status_bar_area: None,
            footer_area: None,
        };

        // Load sidebar data
        app.refresh_sidebar_data();

        // Restore session state
        app.restore_session_state();

        app
    }

    /// Restore session state from database
    fn restore_session_state(&mut self) {
        // Check repository count first
        let repo_count = self
            .repo_dao
            .as_ref()
            .and_then(|dao| dao.get_all().ok())
            .map(|repos| repos.len())
            .unwrap_or(0);

        // If no repos, show first-time splash
        if repo_count == 0 {
            self.show_first_time_splash = true;
            return;
        }

        // Has repos, don't show first-time splash
        self.show_first_time_splash = false;

        // Try to restore saved tabs
        let Some(session_tab_dao) = &self.session_tab_dao else {
            return;
        };
        let Some(app_state_dao) = &self.app_state_dao else {
            return;
        };

        let saved_tabs = match session_tab_dao.get_all() {
            Ok(tabs) => tabs,
            Err(_) => return,
        };

        if saved_tabs.is_empty() {
            // Has repos but no saved tabs - show main UI without tabs
            return;
        }

        // Restore each tab
        for tab in saved_tabs {
            let mut session = AgentSession::new(tab.agent_type);
            session.workspace_id = tab.workspace_id;
            session.model = tab.model;

            // Look up workspace to get working_dir
            if let Some(workspace_id) = tab.workspace_id {
                if let Some(workspace_dao) = &self.workspace_dao {
                    if let Ok(Some(workspace)) = workspace_dao.get_by_id(workspace_id) {
                        session.working_dir = Some(workspace.path);
                    }
                }
            }

            // Set resume session ID if available
            if let Some(ref session_id_str) = tab.agent_session_id {
                let session_id = SessionId::from_string(session_id_str.clone());
                session.resume_session_id = Some(session_id.clone());
                session.agent_session_id = Some(session_id.clone());

                // Load chat history from agent files
                let messages = match tab.agent_type {
                    AgentType::Claude => load_claude_history(session_id_str),
                    AgentType::Codex => load_codex_history(session_id_str),
                };

                if let Ok(msgs) = messages {
                    for msg in msgs {
                        session.chat_view.push(msg);
                    }
                }
            }

            self.tab_manager.add_session(session);
        }

        // Restore active tab
        if let Ok(Some(index_str)) = app_state_dao.get("active_tab_index") {
            if let Ok(index) = index_str.parse::<usize>() {
                self.tab_manager.switch_to(index);
            }
        }

        // Restore sidebar visibility
        if let Ok(Some(visible_str)) = app_state_dao.get("sidebar_visible") {
            self.sidebar_state.visible = visible_str == "true";
        }

        // Restore expanded repos
        if let Ok(Some(expanded_str)) = app_state_dao.get("tree_expanded_repos") {
            if !expanded_str.is_empty() {
                for id_str in expanded_str.split(',') {
                    if let Ok(id) = uuid::Uuid::parse_str(id_str) {
                        self.sidebar_data.expand_repo(id);
                    }
                }
            }
        }

        // Restore tree selection index (after expanding repos so visible count is correct)
        if let Ok(Some(index_str)) = app_state_dao.get("tree_selected_index") {
            if let Ok(index) = index_str.parse::<usize>() {
                let visible_count = self.sidebar_data.visible_nodes().len();
                self.sidebar_state.tree_state.selected = index.min(visible_count.saturating_sub(1));
            }
        }
    }

    /// Refresh sidebar data from database
    fn refresh_sidebar_data(&mut self) {
        self.sidebar_data = SidebarData::new();

        let Some(repo_dao) = &self.repo_dao else {
            return;
        };
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        // Load all repositories
        if let Ok(repos) = repo_dao.get_all() {
            for repo in repos {
                // Load workspaces for this repository
                if let Ok(workspaces) = workspace_dao.get_by_repository(repo.id) {
                    let workspace_info: Vec<_> = workspaces
                        .into_iter()
                        .map(|ws| (ws.id, ws.name, ws.branch))
                        .collect();
                    self.sidebar_data
                        .add_repository(repo.id, &repo.name, workspace_info);
                }
            }
        }
    }

    /// Save session state to database for restoration on next startup
    fn save_session_state(&self) {
        let Some(session_tab_dao) = &self.session_tab_dao else {
            return;
        };
        let Some(app_state_dao) = &self.app_state_dao else {
            return;
        };

        // Clear existing session data
        if let Err(e) = session_tab_dao.clear_all() {
            eprintln!("Warning: Failed to clear session tabs: {}", e);
            return;
        }

        // Save each tab
        for (index, session) in self.tab_manager.sessions().iter().enumerate() {
            let tab = SessionTab::new(
                index as i32,
                session.agent_type,
                session.workspace_id,
                session.agent_session_id.as_ref().map(|s| s.as_str().to_string()),
                session.model.clone(),
            );
            if let Err(e) = session_tab_dao.create(&tab) {
                eprintln!("Warning: Failed to save session tab: {}", e);
            }
        }

        // Save app state
        if let Err(e) = app_state_dao.set(
            "active_tab_index",
            &self.tab_manager.active_index().to_string(),
        ) {
            eprintln!("Warning: Failed to save active tab index: {}", e);
        }

        if let Err(e) = app_state_dao.set(
            "sidebar_visible",
            if self.sidebar_state.visible {
                "true"
            } else {
                "false"
            },
        ) {
            eprintln!("Warning: Failed to save sidebar visibility: {}", e);
        }

        // Save tree selection index
        if let Err(e) = app_state_dao.set(
            "tree_selected_index",
            &self.sidebar_state.tree_state.selected.to_string(),
        ) {
            eprintln!("Warning: Failed to save tree selection: {}", e);
        }

        // Save expanded repo IDs as comma-separated string
        let expanded_ids: Vec<String> = self
            .sidebar_data
            .expanded_repo_ids()
            .iter()
            .map(|id| id.to_string())
            .collect();
        if let Err(e) = app_state_dao.set("tree_expanded_repos", &expanded_ids.join(",")) {
            eprintln!("Warning: Failed to save expanded repos: {}", e);
        }
    }

    /// Run the application main loop
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Clear screen
        terminal.clear()?;

        // Main event loop
        let result = self.event_loop(&mut terminal).await;

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        loop {
            // Draw UI
            terminal.draw(|f| self.draw(f))?;

            // Handle events
            tokio::select! {
                // Terminal input events + tick
                _ = tokio::time::sleep(Duration::from_millis(16)) => {
                    // Handle keyboard and mouse input
                    if event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Key(key) => {
                                self.handle_key_event(key).await?;
                            }
                            Event::Mouse(mouse) => {
                                self.handle_mouse_event(mouse);
                            }
                            _ => {}
                        }
                    }

                    // Tick animations (every 6 frames = ~100ms)
                    self.tick_count += 1;
                    if self.tick_count % 6 == 0 {
                        if self.show_first_time_splash {
                            // Animate splash screen
                            self.splash_screen.tick();
                        } else if let Some(session) = self.tab_manager.active_session_mut() {
                            session.tick();
                        }
                    }
                }

                // App events from channel
                Some(event) = self.event_rx.recv() => {
                    self.handle_app_event(event).await?;
                }
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_key_event(&mut self, key: event::KeyEvent) -> anyhow::Result<()> {
        // Handle dialog-specific keys FIRST before global shortcuts
        // This prevents global shortcuts from interfering with dialog navigation
        if self.input_mode == InputMode::PickingProject
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('j') => {
                    self.project_picker_state.select_next();
                    return Ok(());
                }
                KeyCode::Char('k') => {
                    self.project_picker_state.select_prev();
                    return Ok(());
                }
                KeyCode::Char('f') => {
                    self.project_picker_state.page_down();
                    return Ok(());
                }
                KeyCode::Char('b') => {
                    self.project_picker_state.page_up();
                    return Ok(());
                }
                KeyCode::Char('a') => {
                    // Open custom path dialog
                    self.project_picker_state.hide();
                    self.add_repo_dialog_state.show();
                    self.input_mode = InputMode::AddingRepository;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Ctrl+Shift shortcuts (check first, before plain Ctrl)
        if key.modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT) {
            match key.code {
                KeyCode::Char('D') | KeyCode::Char('d') => {
                    // Ctrl+Shift+D: Dump debug state to file
                    self.dump_debug_state()?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Global shortcuts (work in any mode, but skip if already handled by dialog)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.save_session_state();
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('n') => {
                    // Always trigger new project workflow
                    let base_dir = self
                        .app_state_dao
                        .as_ref()
                        .and_then(|dao| dao.get("projects_base_dir").ok().flatten());

                    if let Some(base_dir_str) = base_dir {
                        let base_path = if base_dir_str.starts_with('~') {
                            dirs::home_dir()
                                .map(|h| h.join(&base_dir_str[1..].trim_start_matches('/')))
                                .unwrap_or_else(|| PathBuf::from(&base_dir_str))
                        } else {
                            PathBuf::from(&base_dir_str)
                        };
                        self.project_picker_state.show(base_path);
                        self.input_mode = InputMode::PickingProject;
                    } else {
                        self.base_dir_dialog_state.show();
                        self.input_mode = InputMode::SettingBaseDir;
                    }
                    return Ok(());
                }
                KeyCode::Char('w') => {
                    // Ctrl+W: delete word if input has text, else close tab
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        if !session.input_box.is_empty() {
                            session.input_box.delete_word_back();
                            return Ok(());
                        }
                    }
                    let active = self.tab_manager.active_index();
                    self.tab_manager.close_tab(active);
                    // Don't quit when closing last tab - show splash screen instead
                    return Ok(());
                }
                KeyCode::Char('c') => {
                    // Interrupt current agent
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        if session.is_processing {
                            session.chat_view.push(ChatMessage::system("Interrupted"));
                            session.stop_processing();
                            // TODO: Actually kill the agent process
                        }
                    }
                    return Ok(());
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let tab_num = c.to_digit(10).unwrap_or(0) as usize;
                    if tab_num > 0 {
                        self.tab_manager.switch_to(tab_num - 1);
                    }
                    return Ok(());
                }
                // Readline shortcuts
                KeyCode::Char('a') => {
                    // Ctrl+A: Move to start of line
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_start();
                    }
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    // Ctrl+E: Move to end of line
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_end();
                    }
                    return Ok(());
                }
                KeyCode::Char('u') => {
                    // Ctrl+U: Delete to start of line
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.delete_to_start();
                    }
                    return Ok(());
                }
                KeyCode::Char('k') => {
                    // Ctrl+K: Delete to end of line
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.delete_to_end();
                    }
                    return Ok(());
                }
                KeyCode::Char('j') => {
                    // Ctrl+J: Insert newline
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.insert_newline();
                    }
                    return Ok(());
                }
                KeyCode::Char('b') => {
                    // Ctrl+B: Toggle sidebar
                    self.sidebar_state.toggle();
                    if self.sidebar_state.visible {
                        self.sidebar_state.set_focused(true);
                        self.input_mode = InputMode::SidebarNavigation;
                    } else {
                        self.sidebar_state.set_focused(false);
                        self.input_mode = InputMode::Normal;
                    }
                    return Ok(());
                }
                KeyCode::Char('m') => {
                    // Ctrl+M: Show model selector for current session
                    if let Some(session) = self.tab_manager.active_session() {
                        self.model_selector_state.show(session.agent_type);
                        self.input_mode = InputMode::SelectingModel;
                    }
                    return Ok(());
                }
                KeyCode::Char('f') => {
                    // Ctrl+F: Move cursor forward (same as Right)
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_right();
                    }
                    return Ok(());
                }
                KeyCode::Char('d') => {
                    // Ctrl+D: Delete character at cursor (same as Delete)
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.delete();
                    }
                    return Ok(());
                }
                KeyCode::Char('h') => {
                    // Ctrl+H: Backspace
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.backspace();
                    }
                    return Ok(());
                }
                KeyCode::Char('g') => {
                    // Ctrl+G: Toggle view mode (Chat <-> RawEvents)
                    self.view_mode = match self.view_mode {
                        ViewMode::Chat => ViewMode::RawEvents,
                        ViewMode::RawEvents => ViewMode::Chat,
                    };
                    return Ok(());
                }
                _ => {}
            }
        }

        // First-time splash screen key handling (only Enter and Esc)
        // Skip if a dialog is visible - let the dialog handle keys
        if self.show_first_time_splash
            && key.modifiers.is_empty()
            && !self.base_dir_dialog_state.is_visible()
            && !self.project_picker_state.is_visible()
            && !self.add_repo_dialog_state.is_visible()
            && self.input_mode != InputMode::SelectingAgent
        {
            match key.code {
                KeyCode::Enter => {
                    // Check if base projects directory is set
                    let base_dir = self
                        .app_state_dao
                        .as_ref()
                        .and_then(|dao| dao.get("projects_base_dir").ok().flatten());

                    if let Some(base_dir_str) = base_dir {
                        // Base dir exists - show project picker
                        let base_path = if base_dir_str.starts_with('~') {
                            dirs::home_dir()
                                .map(|h| h.join(&base_dir_str[1..].trim_start_matches('/')))
                                .unwrap_or_else(|| PathBuf::from(&base_dir_str))
                        } else {
                            PathBuf::from(&base_dir_str)
                        };
                        self.project_picker_state.show(base_path);
                        self.input_mode = InputMode::PickingProject;
                    } else {
                        // No base dir - show setup dialog
                        self.base_dir_dialog_state.show();
                        self.input_mode = InputMode::SettingBaseDir;
                    }
                    return Ok(());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.save_session_state();
                    self.should_quit = true;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Alt key shortcuts
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('b') => {
                    // Alt+B: Move cursor back one word
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_word_left();
                    }
                    return Ok(());
                }
                KeyCode::Char('f') => {
                    // Alt+F: Move cursor forward one word
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_word_right();
                    }
                    return Ok(());
                }
                KeyCode::Char('d') => {
                    // Alt+D: Delete word forward (TODO: implement delete_word_forward)
                    return Ok(());
                }
                KeyCode::Backspace => {
                    // Alt+Backspace: Delete word back (same as Ctrl+W)
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.delete_word_back();
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        match self.input_mode {
            InputMode::SelectingAgent => {
                match key.code {
                    KeyCode::Enter => {
                        let agent_type = self.agent_selector_state.selected_agent();
                        self.agent_selector_state.hide();
                        self.create_tab_with_agent(agent_type);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.agent_selector_state.select_previous();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.agent_selector_state.select_next();
                    }
                    KeyCode::Esc => {
                        self.agent_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    _ => {}
                }
            }
            InputMode::Normal => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    match key.code {
                        KeyCode::Enter => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Toggle expand in raw events view
                                session.raw_events_view.toggle_expand();
                            } else if key.modifiers.contains(KeyModifiers::SHIFT)
                                || key.modifiers.contains(KeyModifiers::SUPER)
                                || key.modifiers.contains(KeyModifiers::META)
                            {
                                // Shift+Enter, Cmd+Enter, or Meta+Enter: insert newline
                                session.input_box.insert_newline();
                            } else if !session.input_box.is_empty() {
                                let prompt = session.input_box.submit();
                                self.submit_prompt(prompt).await?;
                            }
                        }
                        KeyCode::Backspace => {
                            session.input_box.backspace();
                        }
                        KeyCode::Delete => {
                            session.input_box.delete();
                        }
                        KeyCode::Left => {
                            session.input_box.move_left();
                        }
                        KeyCode::Right => {
                            session.input_box.move_right();
                        }
                        KeyCode::Home => {
                            session.input_box.move_start();
                        }
                        KeyCode::End => {
                            session.input_box.move_end();
                        }
                        KeyCode::Up => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Navigate selection in raw events view
                                session.raw_events_view.select_prev();
                            } else {
                                // Try to move up in multi-line input
                                // If can't move (single line or at top), try history
                                if !session.input_box.move_up() {
                                    if session.input_box.is_cursor_on_first_line() {
                                        session.input_box.history_prev();
                                    }
                                }
                            }
                        }
                        KeyCode::Down => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Navigate selection in raw events view
                                session.raw_events_view.select_next();
                            } else {
                                // Try to move down in multi-line input
                                // If can't move (single line or at bottom), try history
                                if !session.input_box.move_down() {
                                    if session.input_box.is_cursor_on_last_line() {
                                        session.input_box.history_next();
                                    }
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            session.chat_view.scroll_up(10);
                        }
                        KeyCode::PageDown => {
                            session.chat_view.scroll_down(10);
                        }
                        KeyCode::Tab => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Toggle expand in raw events view
                                session.raw_events_view.toggle_expand();
                            } else if session.input_box.is_empty() {
                                self.tab_manager.next_tab();
                            }
                        }
                        KeyCode::BackTab => {
                            if session.input_box.is_empty() {
                                self.tab_manager.prev_tab();
                            }
                        }
                        KeyCode::Char(c) => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Vim-style navigation in raw events view
                                match c {
                                    'j' => session.raw_events_view.select_next(),
                                    'k' => session.raw_events_view.select_prev(),
                                    'l' => session.raw_events_view.toggle_expand(),
                                    'h' => session.raw_events_view.collapse(),
                                    _ => {}
                                }
                            } else {
                                session.input_box.insert_char(c);
                            }
                        }
                        KeyCode::Esc => {
                            if self.view_mode == ViewMode::RawEvents {
                                // Collapse expanded event in raw events view
                                session.raw_events_view.collapse();
                            } else {
                                session.chat_view.scroll_to_bottom();
                            }
                        }
                        _ => {}
                    }
                }
            }
            InputMode::Scrolling => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            session.chat_view.scroll_up(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            session.chat_view.scroll_down(1);
                        }
                        KeyCode::PageUp => {
                            session.chat_view.scroll_up(10);
                        }
                        KeyCode::PageDown => {
                            session.chat_view.scroll_down(10);
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            session.chat_view.scroll_to_top();
                        }
                        KeyCode::End | KeyCode::Char('G') => {
                            session.chat_view.scroll_to_bottom();
                        }
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('i') => {
                            self.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    }
                }
            }
            InputMode::SidebarNavigation => {
                let visible_count = self.sidebar_data.visible_nodes().len();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.sidebar_state.tree_state.select_previous(visible_count);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.sidebar_state.tree_state.select_next(visible_count);
                    }
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            use crate::ui::components::{ActionType, NodeType};
                            match node.node_type {
                                NodeType::Action(ActionType::NewWorkspace) => {
                                    // Create new workspace under parent repo
                                    if let Some(parent_id) = node.parent_id {
                                        self.start_workspace_creation(parent_id);
                                    }
                                }
                                NodeType::Workspace => {
                                    // Open workspace
                                    self.open_workspace(node.id);
                                    self.input_mode = InputMode::Normal;
                                    self.sidebar_state.set_focused(false);
                                }
                                NodeType::Repository => {
                                    // Toggle expand
                                    self.sidebar_data.toggle_at(selected);
                                }
                            }
                        }
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        // Collapse current node
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            if !node.is_leaf() && node.expanded {
                                self.sidebar_data.toggle_at(selected);
                            }
                        }
                    }
                    KeyCode::Esc | KeyCode::Tab => {
                        self.input_mode = InputMode::Normal;
                        self.sidebar_state.set_focused(false);
                    }
                    KeyCode::Char('r') => {
                        // Add repository from sidebar
                        self.add_repo_dialog_state.show();
                        self.input_mode = InputMode::AddingRepository;
                    }
                    KeyCode::Char('s') => {
                        // Open settings - change base projects directory
                        if let Some(dao) = &self.app_state_dao {
                            if let Ok(Some(current_dir)) = dao.get("projects_base_dir") {
                                self.base_dir_dialog_state.show_with_path(&current_dir);
                            } else {
                                self.base_dir_dialog_state.show();
                            }
                        } else {
                            self.base_dir_dialog_state.show();
                        }
                        self.input_mode = InputMode::SettingBaseDir;
                    }
                    KeyCode::Char('x') => {
                        // Archive workspace (if workspace is selected)
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            use crate::ui::components::NodeType;
                            if node.node_type == NodeType::Workspace {
                                self.initiate_archive_workspace(node.id);
                            }
                        }
                    }
                    _ => {}
                }
            }
            InputMode::Confirming => {
                match key.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        self.confirmation_dialog_state.toggle_selection();
                    }
                    KeyCode::Enter => {
                        if self.confirmation_dialog_state.is_confirm_selected() {
                            // Execute the confirmed action
                            if let Some(workspace_id) = self.confirmation_dialog_state.context {
                                self.execute_archive_workspace(workspace_id);
                            }
                        }
                        self.confirmation_dialog_state.hide();
                        self.input_mode = InputMode::SidebarNavigation;
                    }
                    KeyCode::Esc | KeyCode::Char('n') => {
                        self.confirmation_dialog_state.hide();
                        self.input_mode = InputMode::SidebarNavigation;
                    }
                    KeyCode::Char('y') => {
                        // Quick confirm
                        if let Some(workspace_id) = self.confirmation_dialog_state.context {
                            self.execute_archive_workspace(workspace_id);
                        }
                        self.confirmation_dialog_state.hide();
                        self.input_mode = InputMode::SidebarNavigation;
                    }
                    _ => {}
                }
            }
            InputMode::AddingRepository => {
                match key.code {
                    KeyCode::Enter => {
                        if self.add_repo_dialog_state.is_valid {
                            let repo_id = self.add_repository();
                            self.add_repo_dialog_state.hide();

                            // If repo was created, expand and select it
                            if let Some(id) = repo_id {
                                self.sidebar_data.expand_repo(id);
                                // Select the "+ New workspace" action (index 1 if repo is at 0)
                                if let Some(repo_index) = self.sidebar_data.find_repo_index(id) {
                                    // Action is first child of expanded repo, so repo_index + 1
                                    self.sidebar_state.tree_state.selected = repo_index + 1;
                                }
                                self.sidebar_state.show();
                                self.sidebar_state.set_focused(true);
                                // No longer first-time, we have a project now
                                self.show_first_time_splash = false;
                                self.input_mode = InputMode::SidebarNavigation;
                            } else {
                                self.input_mode = InputMode::Normal;
                            }
                        }
                    }
                    KeyCode::Esc => {
                        self.add_repo_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Backspace => {
                        self.add_repo_dialog_state.delete_char();
                    }
                    KeyCode::Delete => {
                        self.add_repo_dialog_state.delete_forward();
                    }
                    KeyCode::Left => {
                        self.add_repo_dialog_state.move_left();
                    }
                    KeyCode::Right => {
                        self.add_repo_dialog_state.move_right();
                    }
                    KeyCode::Home => {
                        self.add_repo_dialog_state.move_start();
                    }
                    KeyCode::End => {
                        self.add_repo_dialog_state.move_end();
                    }
                    KeyCode::Char(c) => {
                        self.add_repo_dialog_state.insert_char(c);
                    }
                    _ => {}
                }
            }
            InputMode::SelectingModel => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.model_selector_state.select_previous();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.model_selector_state.select_next();
                    }
                    KeyCode::Enter => {
                        if let Some(model) = self.model_selector_state.selected_model() {
                            let model_id = model.id.clone();
                            // Update session's model
                            if let Some(session) = self.tab_manager.active_session_mut() {
                                session.model = Some(model_id.clone());
                                session.chat_view.push(ChatMessage::system(format!(
                                    "Model changed to: {}",
                                    model_id
                                )));
                            }
                        }
                        self.model_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Esc => {
                        self.model_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    _ => {}
                }
            }
            InputMode::SettingBaseDir => {
                match key.code {
                    KeyCode::Enter => {
                        if self.base_dir_dialog_state.is_valid {
                            // Save base directory to app_state
                            if let Some(dao) = &self.app_state_dao {
                                let _ = dao.set("projects_base_dir", self.base_dir_dialog_state.input());
                            }
                            // Show project picker
                            let base_path = self.base_dir_dialog_state.expanded_path();
                            self.base_dir_dialog_state.hide();
                            self.project_picker_state.show(base_path);
                            self.input_mode = InputMode::PickingProject;
                        }
                    }
                    KeyCode::Esc => {
                        self.base_dir_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Backspace => {
                        self.base_dir_dialog_state.delete_char();
                    }
                    KeyCode::Delete => {
                        self.base_dir_dialog_state.delete_forward();
                    }
                    KeyCode::Left => {
                        self.base_dir_dialog_state.move_left();
                    }
                    KeyCode::Right => {
                        self.base_dir_dialog_state.move_right();
                    }
                    KeyCode::Home => {
                        self.base_dir_dialog_state.move_start();
                    }
                    KeyCode::End => {
                        self.base_dir_dialog_state.move_end();
                    }
                    KeyCode::Char(c) => {
                        self.base_dir_dialog_state.insert_char(c);
                    }
                    _ => {}
                }
            }
            InputMode::PickingProject => {
                match key.code {
                    KeyCode::Enter => {
                        // Select the current project and add it to sidebar
                        if let Some(project) = self.project_picker_state.selected_project() {
                            let repo_id = self.add_project_to_sidebar(project.path.clone());
                            self.project_picker_state.hide();

                            // If repo was created, expand and select it
                            if let Some(id) = repo_id {
                                self.sidebar_data.expand_repo(id);
                                // Select the "+ New workspace" action (index 1 if repo is at 0)
                                if let Some(repo_index) = self.sidebar_data.find_repo_index(id) {
                                    // Action is first child of expanded repo, so repo_index + 1
                                    self.sidebar_state.tree_state.selected = repo_index + 1;
                                }
                                self.sidebar_state.show();
                                self.sidebar_state.set_focused(true);
                                // No longer first-time, we have a project now
                                self.show_first_time_splash = false;
                                self.input_mode = InputMode::SidebarNavigation;
                            } else {
                                self.input_mode = InputMode::Normal;
                            }
                        }
                    }
                    KeyCode::Esc => {
                        self.project_picker_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Up => {
                        self.project_picker_state.select_prev();
                    }
                    KeyCode::Down => {
                        self.project_picker_state.select_next();
                    }
                    KeyCode::Backspace => {
                        self.project_picker_state.delete_char();
                    }
                    KeyCode::Delete => {
                        self.project_picker_state.delete_forward();
                    }
                    KeyCode::Left => {
                        self.project_picker_state.move_cursor_left();
                    }
                    KeyCode::Right => {
                        self.project_picker_state.move_cursor_right();
                    }
                    KeyCode::Home => {
                        self.project_picker_state.move_cursor_start();
                    }
                    KeyCode::End => {
                        self.project_picker_state.move_cursor_end();
                    }
                    KeyCode::Char(c) => {
                        // Note: Ctrl+A/J/K/F/B are handled at the top of handle_key_event
                        // before global shortcuts to prevent bubbling
                        self.project_picker_state.insert_char(c);
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    /// Open a workspace (create or switch to tab)
    fn open_workspace(&mut self, workspace_id: uuid::Uuid) {
        // Check if there's already a tab with this workspace - switch to it
        if let Some(existing_index) = self.find_tab_for_workspace(workspace_id) {
            self.tab_manager.switch_to(existing_index);
            return;
        }

        // Find the workspace
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        let Ok(Some(workspace)) = workspace_dao.get_by_id(workspace_id) else {
            return;
        };

        // Verify workspace path exists
        if !workspace.path.exists() {
            tracing::error!(
                workspace_id = %workspace_id,
                path = %workspace.path.display(),
                "Workspace path does not exist"
            );
            // TODO: Could offer to recreate the worktree or delete the workspace
            return;
        }

        // Update last accessed
        let _ = workspace_dao.update_last_accessed(workspace_id);

        // Create a new tab with the workspace's working directory
        self.tab_manager
            .new_tab_with_working_dir(AgentType::Claude, workspace.path.clone());

        // Store workspace_id in session
        if let Some(session) = self.tab_manager.active_session_mut() {
            session.workspace_id = Some(workspace_id);
        }
    }

    /// Find the tab index for a workspace if it's already open
    fn find_tab_for_workspace(&self, workspace_id: uuid::Uuid) -> Option<usize> {
        self.tab_manager
            .sessions()
            .iter()
            .position(|session| session.workspace_id == Some(workspace_id))
    }

    /// Start the workspace creation process for a repository
    fn start_workspace_creation(&mut self, repo_id: uuid::Uuid) {
        use crate::data::Workspace;
        use crate::util::{generate_branch_name, generate_workspace_name, get_git_username};

        // Get the repository to find its base path
        let repo = if let Some(repo_dao) = &self.repo_dao {
            match repo_dao.get_by_id(repo_id) {
                Ok(Some(repo)) => repo,
                Ok(None) => {
                    tracing::error!(repo_id = %repo_id, "Repository not found");
                    return;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to get repository");
                    return;
                }
            }
        } else {
            tracing::error!("No repository DAO available");
            return;
        };

        let Some(base_path) = repo.base_path.as_ref() else {
            tracing::error!(repo_id = %repo_id, "Repository has no base path");
            return;
        };

        // Get existing workspace names for this repo
        let existing_names: Vec<String> = if let Some(workspace_dao) = &self.workspace_dao {
            workspace_dao
                .get_by_repository(repo_id)
                .unwrap_or_default()
                .iter()
                .map(|w| w.name.clone())
                .collect()
        } else {
            Vec::new()
        };

        // Generate workspace and branch names
        let workspace_name = generate_workspace_name(&existing_names);
        let username = get_git_username();
        let branch_name = generate_branch_name(&username, &workspace_name);

        tracing::info!(
            repo_id = %repo_id,
            workspace_name = %workspace_name,
            branch_name = %branch_name,
            "Creating workspace"
        );

        // Create the git worktree
        let worktree_path = match self.worktree_manager.create_worktree(
            base_path,
            &branch_name,
            &workspace_name,
        ) {
            Ok(path) => path,
            Err(e) => {
                tracing::error!(error = %e, "Failed to create worktree");
                return;
            }
        };

        // Create workspace in database
        let workspace = Workspace::new(repo_id, &workspace_name, &branch_name, worktree_path);
        let workspace_id = workspace.id;

        if let Some(workspace_dao) = &self.workspace_dao {
            if let Err(e) = workspace_dao.create(&workspace) {
                tracing::error!(error = %e, "Failed to save workspace to database");
                // Worktree was created but DB save failed - try to clean up
                if let Err(cleanup_err) =
                    self.worktree_manager
                        .remove_worktree(base_path, &workspace.path)
                {
                    tracing::error!(error = %cleanup_err, "Failed to clean up worktree after DB error");
                }
                return;
            }
        }

        tracing::info!(
            workspace_id = %workspace_id,
            "Workspace created successfully"
        );

        // Refresh sidebar to show new workspace
        self.refresh_sidebar_data();

        // Expand the repository to show the new workspace
        self.sidebar_data.expand_repo(repo_id);

        // Find and select the new workspace in sidebar
        if let Some(index) = self.find_workspace_index(workspace_id) {
            self.sidebar_state.tree_state.selected = index;
        }

        // Open the workspace
        self.open_workspace(workspace_id);
        self.input_mode = InputMode::Normal;
        self.sidebar_state.set_focused(false);
    }

    /// Find the visible index of a workspace by its ID
    fn find_workspace_index(&self, workspace_id: uuid::Uuid) -> Option<usize> {
        use crate::ui::components::NodeType;
        self.sidebar_data
            .visible_nodes()
            .iter()
            .position(|node| node.id == workspace_id && node.node_type == NodeType::Workspace)
    }

    /// Initiate the archive workspace flow - check git status and show confirmation dialog
    fn initiate_archive_workspace(&mut self, workspace_id: uuid::Uuid) {
        // Get the workspace
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        let Ok(Some(workspace)) = workspace_dao.get_by_id(workspace_id) else {
            tracing::error!(workspace_id = %workspace_id, "Workspace not found");
            return;
        };

        // Get git branch status
        let branch_status = self.worktree_manager.get_branch_status(&workspace.path);

        // Build warnings and determine confirmation type
        let mut warnings = Vec::new();
        let mut has_dirty = false;
        let mut has_unmerged = false;

        if let Ok(status) = branch_status {
            if status.is_dirty {
                has_dirty = true;
                if let Some(desc) = &status.dirty_description {
                    warnings.push(desc.clone());
                } else {
                    warnings.push("Uncommitted changes".to_string());
                }
            }

            if !status.is_merged {
                has_unmerged = true;
                if status.commits_ahead > 0 {
                    warnings.push(format!(
                        "Branch not merged ({} commits ahead)",
                        status.commits_ahead
                    ));
                } else {
                    warnings.push("Branch not merged into main".to_string());
                }
            }

            if status.commits_behind > 0 {
                warnings.push(format!(
                    "Branch is {} commits behind main",
                    status.commits_behind
                ));
            }
        }

        // Determine confirmation type based on warnings
        let confirmation_type = match (has_dirty, has_unmerged) {
            (true, true) => ConfirmationType::Danger,
            (true, false) | (false, true) => ConfirmationType::Warning,
            (false, false) => {
                if warnings.is_empty() {
                    ConfirmationType::Info
                } else {
                    ConfirmationType::Warning
                }
            }
        };

        // Show confirmation dialog
        self.confirmation_dialog_state.show(
            format!("Archive \"{}\"?", workspace.name),
            "This will remove the worktree but keep the branch.",
            warnings,
            confirmation_type,
            "Archive",
            Some(workspace_id),
        );
        self.input_mode = InputMode::Confirming;
    }

    /// Execute the archive workspace action after confirmation
    fn execute_archive_workspace(&mut self, workspace_id: uuid::Uuid) {
        // Get the workspace and its repository
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        let Ok(Some(workspace)) = workspace_dao.get_by_id(workspace_id) else {
            tracing::error!(workspace_id = %workspace_id, "Workspace not found");
            return;
        };

        // Get the repository to find its base path
        let repo_base_path = if let Some(repo_dao) = &self.repo_dao {
            match repo_dao.get_by_id(workspace.repository_id) {
                Ok(Some(repo)) => repo.base_path,
                _ => None,
            }
        } else {
            None
        };

        // Remove the git worktree
        if let Some(base_path) = repo_base_path {
            if let Err(e) = self.worktree_manager.remove_worktree(&base_path, &workspace.path) {
                tracing::error!(error = %e, "Failed to remove worktree");
                // Continue anyway to mark as archived in DB
            }
        }

        // Mark workspace as archived in database
        if let Err(e) = workspace_dao.archive(workspace_id) {
            tracing::error!(error = %e, "Failed to archive workspace in database");
            return;
        }

        tracing::info!(workspace_id = %workspace_id, "Workspace archived successfully");

        // Close any tabs using this workspace
        self.close_tabs_for_workspace(workspace_id);

        // Remember current selection to move to item above after refresh
        let current_selection = self.sidebar_state.tree_state.selected;

        // Refresh sidebar to remove archived workspace
        self.refresh_sidebar_data();

        // Move selection to item above (if exists) or clamp to valid range
        let visible_count = self.sidebar_data.visible_nodes().len();
        if visible_count > 0 {
            let new_selection = if current_selection > 0 {
                current_selection - 1
            } else {
                0
            };
            self.sidebar_state.tree_state.selected = new_selection.min(visible_count - 1);
        } else {
            self.sidebar_state.tree_state.selected = 0;
        }
    }

    /// Close any tabs that are using the specified workspace
    fn close_tabs_for_workspace(&mut self, workspace_id: uuid::Uuid) {
        // Find tabs with this workspace and close them (in reverse order to maintain indices)
        let indices_to_close: Vec<usize> = self
            .tab_manager
            .sessions()
            .iter()
            .enumerate()
            .filter_map(|(idx, session)| {
                if session.workspace_id == Some(workspace_id) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();

        for idx in indices_to_close.into_iter().rev() {
            self.tab_manager.close_tab(idx);
        }
    }

    /// Add a project to the sidebar (repository only, no workspace)
    /// Returns the repository ID if created successfully
    fn add_project_to_sidebar(&mut self, path: std::path::PathBuf) -> Option<uuid::Uuid> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let Some(repo_dao) = &self.repo_dao else {
            return None;
        };

        // Create repository (without default workspace)
        let repo = Repository::from_local_path(&name, path);
        if repo_dao.create(&repo).is_err() {
            return None;
        }

        let repo_id = repo.id;

        // Refresh sidebar
        self.refresh_sidebar_data();

        Some(repo_id)
    }

    /// Add a repository from the custom path dialog
    /// Returns the repository ID if created successfully
    fn add_repository(&mut self) -> Option<uuid::Uuid> {
        let path = self.add_repo_dialog_state.expanded_path();
        let name = self
            .add_repo_dialog_state
            .repo_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        let Some(repo_dao) = &self.repo_dao else {
            return None;
        };

        // Create repository (without default workspace)
        let repo = Repository::from_local_path(&name, path);
        if repo_dao.create(&repo).is_err() {
            return None;
        }

        let repo_id = repo.id;

        // Refresh sidebar
        self.refresh_sidebar_data();

        Some(repo_id)
    }

    /// Create a new tab with the selected agent type
    fn create_tab_with_agent(&mut self, agent_type: AgentType) {
        self.tab_manager.new_tab(agent_type);
        self.input_mode = InputMode::Normal;
    }

    fn handle_mouse_event(&mut self, mouse: event::MouseEvent) {
        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_up(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_down(3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Handle left clicks based on position
                self.handle_mouse_click(x, y);
            }
            _ => {}
        }
    }

    /// Handle a mouse click at the given position
    fn handle_mouse_click(&mut self, x: u16, y: u16) {
        // Handle project picker clicks first (it's a modal dialog)
        if self.input_mode == InputMode::PickingProject
            && self.project_picker_state.is_visible()
        {
            self.handle_project_picker_click(x, y);
            return;
        }

        // Check sidebar first (if visible)
        if let Some(sidebar_area) = self.sidebar_area {
            if Self::point_in_rect(x, y, sidebar_area) {
                self.handle_sidebar_click(x, y, sidebar_area);
                return;
            }
        }

        // Check tab bar
        if let Some(tab_bar_area) = self.tab_bar_area {
            if Self::point_in_rect(x, y, tab_bar_area) {
                self.handle_tab_bar_click(x, y, tab_bar_area);
                return;
            }
        }

        // Check input area
        if let Some(input_area) = self.input_area {
            if Self::point_in_rect(x, y, input_area) {
                self.handle_input_click(x, y, input_area);
                return;
            }
        }

        // Check status bar
        if let Some(status_bar_area) = self.status_bar_area {
            if Self::point_in_rect(x, y, status_bar_area) {
                self.handle_status_bar_click(x, y, status_bar_area);
                return;
            }
        }

        // Check footer
        if let Some(footer_area) = self.footer_area {
            if Self::point_in_rect(x, y, footer_area) {
                self.handle_footer_click(x, y, footer_area);
                return;
            }
        }

        // Click in chat area - could be used for text selection in future
        // For now, clicking in chat area while in sidebar mode returns to normal
        if self.input_mode == InputMode::SidebarNavigation {
            self.input_mode = InputMode::Normal;
            self.sidebar_state.set_focused(false);
        }
    }

    /// Check if a point is within a rectangle
    fn point_in_rect(x: u16, y: u16, rect: Rect) -> bool {
        x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
    }

    /// Handle click in sidebar area
    fn handle_sidebar_click(&mut self, _x: u16, y: u16, sidebar_area: Rect) {
        // Account for border (1 row at top for title)
        let inner_y = sidebar_area.y + 1;
        if y < inner_y {
            return; // Clicked on title bar
        }

        let clicked_row = (y - inner_y) as usize;
        let clicked_index = clicked_row + self.sidebar_state.tree_state.offset;

        // Get the node at this index
        if let Some(node) = self.sidebar_data.get_at(clicked_index) {
            use crate::ui::components::{ActionType, NodeType};

            // Update selection
            self.sidebar_state.tree_state.selected = clicked_index;

            // Focus sidebar
            self.sidebar_state.set_focused(true);
            self.input_mode = InputMode::SidebarNavigation;

            // Handle based on node type
            match node.node_type {
                NodeType::Repository => {
                    // Toggle expand/collapse
                    self.sidebar_data.toggle_at(clicked_index);
                }
                NodeType::Workspace => {
                    // Open workspace
                    self.open_workspace(node.id);
                    self.input_mode = InputMode::Normal;
                    self.sidebar_state.set_focused(false);
                }
                NodeType::Action(ActionType::NewWorkspace) => {
                    // Create new workspace
                    if let Some(parent_id) = node.parent_id {
                        self.start_workspace_creation(parent_id);
                    }
                }
            }
        }
    }

    /// Handle click in tab bar area
    fn handle_tab_bar_click(&mut self, x: u16, _y: u16, tab_bar_area: Rect) {
        let relative_x = x.saturating_sub(tab_bar_area.x) as usize;

        // Calculate tab positions
        let mut current_x: usize = 0;
        let tab_names = self.tab_manager.tab_names();
        let active_index = self.tab_manager.active_index();

        for (i, name) in tab_names.iter().enumerate() {
            // Format: " ▶ [N] Name " for active, "  [N] Name " for inactive
            let tab_width = if i == active_index {
                4 + 3 + name.len() + 1 // " ▶ " + "[N]" + " Name" + " "
            } else {
                2 + 3 + name.len() + 1 // "  " + "[N]" + " Name" + " "
            };

            if relative_x < current_x + tab_width {
                // Clicked on this tab
                self.tab_manager.switch_to(i);
                return;
            }
            current_x += tab_width;
        }

        // Check for "+ New" button
        if self.tab_manager.can_add_tab() {
            // "+ New" button width is about 7 characters: "  [+]  "
            let new_button_width = 7;
            if relative_x >= current_x && relative_x < current_x + new_button_width {
                // Show agent selector for new tab
                self.agent_selector_state.show();
                self.input_mode = InputMode::SelectingAgent;
            }
        }
    }

    /// Handle click in input area
    fn handle_input_click(&mut self, x: u16, y: u16, input_area: Rect) {
        // Switch to normal mode if we were in sidebar navigation
        if self.input_mode == InputMode::SidebarNavigation {
            self.input_mode = InputMode::Normal;
            self.sidebar_state.set_focused(false);
        }

        // Position cursor based on click
        if let Some(session) = self.tab_manager.active_session_mut() {
            session.input_box.set_cursor_from_click(x, y, input_area);
        }
    }

    /// Handle click in status bar area
    fn handle_status_bar_click(&mut self, x: u16, _y: u16, status_bar_area: Rect) {
        // Status bar format: "agent │ model │ status"
        // Click on model portion to open model selector
        let relative_x = x.saturating_sub(status_bar_area.x) as usize;

        // Approximate positions - agent type is ~6 chars, separator is 3, model starts around 9
        // This is a rough estimate; could be made more precise by tracking actual positions
        if relative_x >= 9 && relative_x < 25 {
            // Likely clicked on model area
            if let Some(session) = self.tab_manager.active_session() {
                let agent_type = session.agent_type;
                self.model_selector_state.show(agent_type);
                self.input_mode = InputMode::SelectingModel;
            }
        }
    }

    /// Handle click in project picker dialog
    fn handle_project_picker_click(&mut self, x: u16, y: u16) {
        // Calculate dialog position based on terminal size
        // The dialog is 60 wide and centered, height is 7 + list_height
        let terminal_size = crossterm::terminal::size().unwrap_or((80, 24));
        let screen_width = terminal_size.0;
        let screen_height = terminal_size.1;

        let dialog_width: u16 = 60;
        let list_height = self
            .project_picker_state
            .max_visible
            .min(self.project_picker_state.filtered.len().max(1)) as u16;
        let dialog_height = 7 + list_height;

        // Calculate dialog position (centered)
        let dialog_x = screen_width.saturating_sub(dialog_width) / 2;
        let dialog_y = screen_height.saturating_sub(dialog_height) / 2;

        // Inner area is dialog minus border (1 on each side)
        let inner_x = dialog_x + 1;
        let inner_y = dialog_y + 1;
        let inner_width = dialog_width.saturating_sub(2);

        // List area starts at row 3 within inner area (after search, search input, separator)
        // Layout: [0] search label, [1] search input, [2] separator, [3..] list
        let list_y = inner_y + 3;
        let list_height_actual = dialog_height.saturating_sub(7); // Same as list_height

        // Check if click is in the list area
        if x >= inner_x
            && x < inner_x + inner_width
            && y >= list_y
            && y < list_y + list_height_actual
        {
            // Calculate which row was clicked
            let clicked_row = (y - list_y) as usize;

            // Select the item and trigger double-click detection
            if self.project_picker_state.select_at_row(clicked_row) {
                // Check for double-click (would need timing - for now just select)
                // Could add double-click to open in future
            }
        }
    }

    /// Handle click in footer area
    fn handle_footer_click(&mut self, x: u16, _y: u16, footer_area: Rect) {
        // Footer format: "[Key] Action │ [Key] Action │ ..."
        // Map click position to hint and trigger corresponding action
        let relative_x = x.saturating_sub(footer_area.x) as usize;

        // Calculate hint positions based on current view mode
        let hints: Vec<(&str, &str)> = match self.view_mode {
            ViewMode::Chat => vec![
                ("Tab", "Switch"),
                ("^N", "+ Project"),
                ("^W", "Close"),
                ("^C", "Interrupt"),
                ("^G", "Debug"),
                ("^Q", "Quit"),
            ],
            ViewMode::RawEvents => vec![
                ("j/k", "Navigate"),
                ("l/Enter", "Expand"),
                ("h/Esc", "Collapse"),
                ("^G", "Chat"),
                ("^Q", "Quit"),
            ],
        };

        let mut current_x: usize = 0;
        for (key, action) in hints {
            // Format: "[Key] Action │ " - approximately key.len() + 2 + action.len() + 3
            let hint_width = key.len() + 2 + 1 + action.len() + 3;

            if relative_x >= current_x && relative_x < current_x + hint_width {
                // Clicked on this hint - trigger the action
                self.trigger_footer_action(key);
                return;
            }
            current_x += hint_width;
        }
    }

    /// Trigger an action from footer hint click
    fn trigger_footer_action(&mut self, key: &str) {
        match key {
            "Tab" => {
                // Toggle sidebar
                self.sidebar_state.toggle();
                if self.sidebar_state.visible {
                    self.input_mode = InputMode::SidebarNavigation;
                } else {
                    self.input_mode = InputMode::Normal;
                }
            }
            "^N" => {
                // New project - trigger same as Ctrl+N
                let base_dir = self
                    .app_state_dao
                    .as_ref()
                    .and_then(|dao| dao.get("projects_base_dir").ok().flatten());

                if let Some(base_dir_str) = base_dir {
                    let base_path = if base_dir_str.starts_with('~') {
                        dirs::home_dir()
                            .map(|h| h.join(&base_dir_str[1..].trim_start_matches('/')))
                            .unwrap_or_else(|| PathBuf::from(&base_dir_str))
                    } else {
                        PathBuf::from(&base_dir_str)
                    };
                    self.project_picker_state.show(base_path);
                    self.input_mode = InputMode::PickingProject;
                } else {
                    self.base_dir_dialog_state.show();
                    self.input_mode = InputMode::SettingBaseDir;
                }
            }
            "^W" => {
                // Close tab
                let active = self.tab_manager.active_index();
                self.tab_manager.close_tab(active);
            }
            "^C" => {
                // Interrupt
                if let Some(session) = self.tab_manager.active_session_mut() {
                    if session.is_processing {
                        session.chat_view.push(ChatMessage::system("Interrupted"));
                        session.stop_processing();
                    }
                }
            }
            "^G" => {
                // Toggle debug view
                self.view_mode = match self.view_mode {
                    ViewMode::Chat => ViewMode::RawEvents,
                    ViewMode::RawEvents => ViewMode::Chat,
                };
            }
            "^Q" => {
                // Quit
                self.save_session_state();
                self.should_quit = true;
            }
            _ => {}
        }
    }

    async fn handle_app_event(&mut self, event: AppEvent) -> anyhow::Result<()> {
        match event {
            AppEvent::Agent { tab_index, event } => {
                self.handle_agent_event(tab_index, event).await?;
            }
            AppEvent::Quit => {
                self.save_session_state();
                self.should_quit = true;
            }
            AppEvent::Error(msg) => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.push(ChatMessage::error(msg));
                    session.stop_processing();
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn handle_agent_event(
        &mut self,
        tab_index: usize,
        event: AgentEvent,
    ) -> anyhow::Result<()> {
        let Some(session) = self.tab_manager.session_mut(tab_index) else {
            return Ok(());
        };

        // Record raw event for debug view
        let event_type = event.event_type_name();
        let raw_json = serde_json::to_value(&event).unwrap_or_default();
        session.record_raw_event(EventDirection::Received, event_type, raw_json);

        match event {
            AgentEvent::SessionInit(init) => {
                session.agent_session_id = Some(init.session_id);
                session.update_status();
            }
            AgentEvent::TurnStarted => {
                session.is_processing = true;
                session.update_status();
            }
            AgentEvent::TurnCompleted(completed) => {
                session.add_usage(completed.usage);
                session.stop_processing();
                session.chat_view.finalize_streaming();
                // Add turn summary to chat
                let summary = session.current_turn_summary.clone();
                session.chat_view.push(ChatMessage::turn_summary(summary));
            }
            AgentEvent::TurnFailed(failed) => {
                session.stop_processing();
                session.chat_view.push(ChatMessage::error(failed.error));
            }
            AgentEvent::AssistantMessage(msg) => {
                // Track streaming tokens (rough estimate: ~4 chars per token)
                let token_estimate = (msg.text.len() / 4).max(1);
                session.add_streaming_tokens(token_estimate);

                if msg.is_final {
                    session.chat_view.push(ChatMessage::assistant(msg.text));
                } else {
                    session.chat_view.stream_append(&msg.text);
                }
            }
            AgentEvent::ToolStarted(tool) => {
                // Update processing state to show tool name
                session.set_processing_state(ProcessingState::ToolUse(tool.tool_name.clone()));

                let args_str = if tool.arguments.is_null() {
                    String::new()
                } else {
                    // Compact single-line for display
                    serde_json::to_string(&tool.arguments).unwrap_or_default()
                };
                session.chat_view.push(ChatMessage::tool(
                    &tool.tool_name,
                    args_str,
                    "Running...",
                ));
            }
            AgentEvent::ToolCompleted(tool) => {
                // Return to thinking state
                session.set_processing_state(ProcessingState::Thinking);

                // Track file changes for write/edit tools
                if tool.success {
                    let tool_name = tool.tool_id.to_lowercase();
                    if tool_name.contains("edit") || tool_name.contains("write") || tool_name.contains("multiedit") {
                        // Try to extract filename from result or use generic name
                        if let Some(ref result) = tool.result {
                            // Simple heuristic: look for file paths in result
                            if let Some(filename) = Self::extract_filename(result) {
                                // Rough estimate of changes (can be refined)
                                session.record_file_change(filename, 5, 2);
                            }
                        }
                    }
                }

                let content = if tool.success {
                    tool.result.unwrap_or_else(|| "Completed".to_string())
                } else {
                    format!("Error: {}", tool.error.unwrap_or_default())
                };
                session
                    .chat_view
                    .push(ChatMessage::tool(&tool.tool_id, "", content));
            }
            AgentEvent::CommandOutput(cmd) => {
                let output = format!(
                    "{}{}",
                    cmd.output,
                    cmd.exit_code
                        .map(|c| format!("\n[exit: {}]", c))
                        .unwrap_or_default()
                );
                session.chat_view.push(ChatMessage::tool(
                    "Bash",
                    &cmd.command,
                    output,
                ));
            }
            AgentEvent::Error(err) => {
                session.chat_view.push(ChatMessage::error(err.message));
                if err.is_fatal {
                    session.stop_processing();
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn submit_prompt(&mut self, prompt: String) -> anyhow::Result<()> {
        let tab_index = self.tab_manager.active_index();
        let Some(session) = self.tab_manager.active_session_mut() else {
            return Ok(());
        };

        // Record user input for debug view
        session.record_raw_event(
            EventDirection::Sent,
            "UserPrompt",
            serde_json::json!({ "prompt": &prompt }),
        );

        // Add user message to chat
        session.chat_view.push(ChatMessage::user(&prompt));
        session.start_processing();

        // Capture session state before releasing borrow
        let agent_type = session.agent_type;
        let model = session.model.clone();
        // Take resume_session_id (clears it after first use)
        let resume_session_id = session.resume_session_id.take();
        // Use session's working_dir if set, otherwise fall back to config
        let working_dir = session
            .working_dir
            .clone()
            .unwrap_or_else(|| self.config.working_dir.clone());

        // Validate working directory exists
        if !working_dir.exists() {
            if let Some(session) = self.tab_manager.active_session_mut() {
                session.chat_view.push(ChatMessage::error(format!(
                    "Working directory does not exist: {}",
                    working_dir.display()
                )));
                session.stop_processing();
            }
            return Ok(());
        }

        // Start agent
        let mut config = AgentStartConfig::new(prompt, working_dir)
            .with_tools(self.config.claude_allowed_tools.clone());

        // Add model if specified
        if let Some(model_id) = model {
            config = config.with_model(model_id);
        }

        // Add resume session if restoring from saved state
        if let Some(session_id) = resume_session_id {
            config = config.with_resume(session_id);
        }

        let runner: Arc<dyn AgentRunner> = match agent_type {
            AgentType::Claude => self.claude_runner.clone(),
            AgentType::Codex => self.codex_runner.clone(),
        };

        let event_tx = self.event_tx.clone();

        // Spawn agent task
        tokio::spawn(async move {
            match runner.start(config).await {
                Ok(mut handle) => {
                    while let Some(event) = handle.events.recv().await {
                        if event_tx
                            .send(AppEvent::Agent {
                                tab_index,
                                event,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AppEvent::Error(format!("Agent error: {}", e)));
                }
            }
        });

        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        let size = f.area();

        // Show splash screen only for first-time users (no repos)
        if self.show_first_time_splash {
            self.splash_screen.first_time_mode = true;
            self.splash_screen.render(size, f.buffer_mut());

            // Draw dialogs over splash screen
            if self.base_dir_dialog_state.is_visible() {
                let dialog = BaseDirDialog::new();
                dialog.render(size, f.buffer_mut(), &self.base_dir_dialog_state);
            } else if self.project_picker_state.is_visible() {
                let picker = ProjectPicker::new();
                picker.render(size, f.buffer_mut(), &self.project_picker_state);
            } else if self.add_repo_dialog_state.is_visible() {
                let dialog = AddRepoDialog::new();
                dialog.render(size, f.buffer_mut(), &self.add_repo_dialog_state);
            }

            // Draw agent selector dialog if needed
            if self.agent_selector_state.is_visible() {
                let selector = AgentSelector::new();
                selector.render(size, f.buffer_mut(), &self.agent_selector_state);
            }
            return;
        }

        // Calculate sidebar width
        let sidebar_width = if self.sidebar_state.visible { 30u16 } else { 0 };

        // Split horizontally for sidebar
        let (sidebar_area, content_area) = if sidebar_width > 0 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(sidebar_width),
                    Constraint::Min(20),
                ])
                .split(size);
            (chunks[0], chunks[1])
        } else {
            // No sidebar - use full width
            (Rect::default(), size)
        };

        // Store sidebar area for mouse hit-testing
        self.sidebar_area = if self.sidebar_state.visible {
            Some(sidebar_area)
        } else {
            None
        };

        // Render sidebar if visible
        if self.sidebar_state.visible {
            let sidebar = Sidebar::new(&self.sidebar_data);
            ratatui::widgets::StatefulWidget::render(
                sidebar,
                sidebar_area,
                f.buffer_mut(),
                &mut self.sidebar_state,
            );
        }

        match self.view_mode {
            ViewMode::Chat => {
                // Calculate dynamic input height (max 30% of screen)
                let max_input_height = (content_area.height as f32 * 0.30).ceil() as u16;
                let input_height = if let Some(session) = self.tab_manager.active_session() {
                    session.input_box.desired_height(max_input_height)
                } else {
                    3 // Minimum height
                };

                // Chat layout with input box
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),            // Tab bar
                        Constraint::Min(5),               // Chat view
                        Constraint::Length(input_height), // Input box (dynamic)
                        Constraint::Length(1),            // Status bar
                        Constraint::Length(1),            // Footer
                    ])
                    .split(content_area);

                // Store layout areas for mouse hit-testing
                self.tab_bar_area = Some(chunks[0]);
                self.chat_area = Some(chunks[1]);
                self.input_area = Some(chunks[2]);
                self.status_bar_area = Some(chunks[3]);
                self.footer_area = Some(chunks[4]);

                // Draw tab bar
                let tab_bar = TabBar::new(
                    self.tab_manager.tab_names(),
                    self.tab_manager.active_index(),
                    self.tab_manager.can_add_tab(),
                );
                tab_bar.render(chunks[0], f.buffer_mut());

                // Draw active session components
                if let Some(session) = self.tab_manager.active_session_mut() {
                    // Render chat with thinking indicator if processing
                    let thinking_line = if session.is_processing {
                        Some(session.thinking_indicator.render())
                    } else {
                        None
                    };
                    session
                        .chat_view
                        .render_with_indicator(chunks[1], f.buffer_mut(), thinking_line);

                    session.input_box.render(chunks[2], f.buffer_mut());
                    session.status_bar.render(chunks[3], f.buffer_mut());

                    // Set cursor position (accounting for scroll)
                    if self.input_mode == InputMode::Normal {
                        let scroll_offset = session.input_box.scroll_offset();
                        let (cx, cy) = session.input_box.cursor_position(chunks[2], scroll_offset);
                        f.set_cursor_position((cx, cy));
                    }
                }

                // Draw footer
                let footer = GlobalFooter::new().with_view_mode(self.view_mode);
                footer.render(chunks[4], f.buffer_mut());
            }
            ViewMode::RawEvents => {
                // Raw events layout - no input box, full height for events
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1), // Tab bar
                        Constraint::Min(5),    // Raw events view (full height)
                        Constraint::Length(1), // Footer
                    ])
                    .split(content_area);

                // Store layout areas for mouse hit-testing (no input/status in this mode)
                self.tab_bar_area = Some(chunks[0]);
                self.chat_area = Some(chunks[1]); // Raw events view uses chat area slot
                self.input_area = None;
                self.status_bar_area = None;
                self.footer_area = Some(chunks[2]);

                // Draw tab bar
                let tab_bar = TabBar::new(
                    self.tab_manager.tab_names(),
                    self.tab_manager.active_index(),
                    self.tab_manager.can_add_tab(),
                );
                tab_bar.render(chunks[0], f.buffer_mut());

                // Draw raw events view
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.raw_events_view.render(chunks[1], f.buffer_mut());
                }

                // Draw footer
                let footer = GlobalFooter::new().with_view_mode(self.view_mode);
                footer.render(chunks[2], f.buffer_mut());
            }
        }

        // Draw agent selector dialog if needed
        if self.agent_selector_state.is_visible() {
            let selector = AgentSelector::new();
            selector.render(size, f.buffer_mut(), &self.agent_selector_state);
        }

        // Draw add repository dialog if open
        if self.add_repo_dialog_state.is_visible() {
            let dialog = AddRepoDialog::new();
            dialog.render(size, f.buffer_mut(), &self.add_repo_dialog_state);
        }

        // Draw model selector dialog if open
        if self.model_selector_state.is_visible() {
            let model_selector = ModelSelector::new();
            model_selector.render(size, f.buffer_mut(), &self.model_selector_state);
        }

        // Draw base directory dialog if open
        if self.base_dir_dialog_state.is_visible() {
            let dialog = BaseDirDialog::new();
            dialog.render(size, f.buffer_mut(), &self.base_dir_dialog_state);
        }

        // Draw project picker if open
        if self.project_picker_state.is_visible() {
            let picker = ProjectPicker::new();
            picker.render(size, f.buffer_mut(), &self.project_picker_state);
        }

        // Draw confirmation dialog if open
        if self.confirmation_dialog_state.visible {
            use ratatui::widgets::Widget;
            let dialog = ConfirmationDialog::new(&self.confirmation_dialog_state);
            dialog.render(size, f.buffer_mut());
        }
    }

    /// Extract a filename from tool result text
    fn extract_filename(text: &str) -> Option<String> {
        // Look for common file path patterns
        for line in text.lines() {
            let line = line.trim();
            // Look for paths like /path/to/file.rs or file.rs
            if line.contains('/') || line.contains('.') {
                // Try to find a file path
                for word in line.split_whitespace() {
                    let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-');
                    if word.contains('.') && !word.starts_with('.') {
                        // Looks like a filename
                        return Some(word.to_string());
                    }
                }
            }
        }
        None
    }

    /// Dump complete app state to a JSON file for debugging
    fn dump_debug_state(&mut self) -> anyhow::Result<()> {
        use chrono::Local;
        use serde_json::json;

        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let filename = format!("conduit_debug_{}.json", timestamp);

        let mut sessions_data = Vec::new();

        for (idx, session) in self.tab_manager.sessions().iter().enumerate() {
            // Collect chat messages
            let messages: Vec<_> = session.chat_view.messages().iter().map(|msg| {
                let summary_data = msg.summary.as_ref().map(|s| json!({
                    "duration_secs": s.duration_secs,
                    "input_tokens": s.input_tokens,
                    "output_tokens": s.output_tokens,
                    "files_changed": s.files_changed.iter().map(|f| json!({
                        "filename": f.filename,
                        "additions": f.additions,
                        "deletions": f.deletions,
                    })).collect::<Vec<_>>(),
                }));

                json!({
                    "role": format!("{:?}", msg.role),
                    "content": msg.content,
                    "content_length": msg.content.len(),
                    "tool_name": msg.tool_name,
                    "tool_args": msg.tool_args,
                    "is_streaming": msg.is_streaming,
                    "has_summary": msg.summary.is_some(),
                    "summary": summary_data,
                })
            }).collect();

            // Collect raw events
            let raw_events: Vec<_> = session.raw_events_view.events().iter().map(|evt| {
                let elapsed = evt.timestamp.duration_since(evt.session_start);
                json!({
                    "timestamp_ms": elapsed.as_millis(),
                    "direction": format!("{:?}", evt.direction),
                    "event_type": evt.event_type,
                    "raw_json": evt.raw_json,
                })
            }).collect();

            // Current turn summary
            let turn_summary = json!({
                "duration_secs": session.current_turn_summary.duration_secs,
                "input_tokens": session.current_turn_summary.input_tokens,
                "output_tokens": session.current_turn_summary.output_tokens,
                "files_changed": session.current_turn_summary.files_changed.iter().map(|f| json!({
                    "filename": f.filename,
                    "additions": f.additions,
                    "deletions": f.deletions,
                })).collect::<Vec<_>>(),
            });

            sessions_data.push(json!({
                "index": idx,
                "id": session.id.to_string(),
                "agent_type": format!("{:?}", session.agent_type),
                "agent_session_id": session.agent_session_id.as_ref().map(|s| s.as_str().to_string()),
                "is_processing": session.is_processing,
                "turn_count": session.turn_count,
                "total_usage": {
                    "input_tokens": session.total_usage.input_tokens,
                    "output_tokens": session.total_usage.output_tokens,
                    "cached_tokens": session.total_usage.cached_tokens,
                    "total_tokens": session.total_usage.total_tokens,
                },
                "current_turn_summary": turn_summary,
                "chat_messages": messages,
                "chat_message_count": session.chat_view.len(),
                "streaming_buffer": session.chat_view.streaming_buffer(),
                "raw_events": raw_events,
                "raw_event_count": session.raw_events_view.len(),
                "input_box_content": session.input_box.input(),
            }));
        }

        let dump = json!({
            "timestamp": Local::now().to_rfc3339(),
            "view_mode": format!("{:?}", self.view_mode),
            "input_mode": format!("{:?}", self.input_mode),
            "active_tab_index": self.tab_manager.active_index(),
            "tab_count": self.tab_manager.len(),
            "sessions": sessions_data,
        });

        let mut file = File::create(&filename)?;
        file.write_all(serde_json::to_string_pretty(&dump)?.as_bytes())?;

        // Show confirmation in chat
        if let Some(session) = self.tab_manager.active_session_mut() {
            session.chat_view.push(ChatMessage::system(format!(
                "Debug state dumped to: {}",
                filename
            )));
        }

        Ok(())
    }
}
