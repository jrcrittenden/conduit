use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        MouseEventKind,
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
    AppStateDao, Database, Repository, RepositoryDao, SessionTab, SessionTabDao, Workspace,
    WorkspaceDao,
};
use crate::git::WorktreeManager;
use crate::ui::components::{
    AddRepoDialog, AddRepoDialogState, AgentSelector, AgentSelectorState, BaseDirDialog,
    BaseDirDialogState, ChatMessage, EventDirection, GlobalFooter, ModelSelector,
    ModelSelectorState, ProcessingState, ProjectPicker, ProjectPickerState, Sidebar, SidebarData,
    SidebarState, SplashScreen, TabBar,
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
    /// Database connection
    database: Option<Database>,
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
    /// Pending project path (selected in picker, waiting for agent selection)
    pending_project_path: Option<PathBuf>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Initialize database
        let (database, repo_dao, workspace_dao, app_state_dao, session_tab_dao) =
            match Database::open_default() {
                Ok(db) => {
                    let repo_dao = RepositoryDao::new(db.connection());
                    let workspace_dao = WorkspaceDao::new(db.connection());
                    let app_state_dao = AppStateDao::new(db.connection());
                    let session_tab_dao = SessionTabDao::new(db.connection());
                    (
                        Some(db),
                        Some(repo_dao),
                        Some(workspace_dao),
                        Some(app_state_dao),
                        Some(session_tab_dao),
                    )
                }
                Err(e) => {
                    eprintln!("Warning: Failed to open database: {}", e);
                    (None, None, None, None, None)
                }
            };

        // Initialize worktree manager with managed directory
        let worktree_dir = dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
            .map(|d| d.join("conduit").join("worktrees"))
            .unwrap_or_else(|| PathBuf::from(".conduit/worktrees"));

        let worktree_manager = WorktreeManager::with_managed_dir(worktree_dir);

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
            database,
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
            pending_project_path: None,
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

        // Global shortcuts (work in any mode)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.save_session_state();
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('n') => {
                    self.agent_selector_state.show();
                    self.input_mode = InputMode::SelectingAgent;
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
                            session.is_processing = false;
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
                        self.pending_project_path = None;
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
                            session.input_box.insert_char(c);
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
                    KeyCode::Enter | KeyCode::Right => {
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            if node.is_leaf {
                                // Open workspace
                                self.open_workspace(node.id);
                                self.input_mode = InputMode::Normal;
                                self.sidebar_state.set_focused(false);
                            } else {
                                // Toggle expand
                                self.sidebar_data.toggle_at(selected);
                            }
                        }
                    }
                    KeyCode::Left => {
                        // Collapse current node
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            if !node.is_leaf && node.expanded {
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
                    _ => {}
                }
            }
            InputMode::AddingRepository => {
                match key.code {
                    KeyCode::Enter => {
                        if self.add_repo_dialog_state.is_valid {
                            self.add_repository();
                            self.add_repo_dialog_state.hide();
                            self.input_mode = InputMode::Normal;
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
                        // Select the current project
                        if let Some(project) = self.project_picker_state.selected_project() {
                            self.pending_project_path = Some(project.path.clone());
                            self.project_picker_state.hide();
                            // Show agent selector
                            self.agent_selector_state.show();
                            self.input_mode = InputMode::SelectingAgent;
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
                    KeyCode::Char('a') if key.modifiers.is_empty() => {
                        // Open custom path dialog
                        self.project_picker_state.hide();
                        self.add_repo_dialog_state.show();
                        self.input_mode = InputMode::AddingRepository;
                    }
                    KeyCode::Char(c) => {
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
        // Find the workspace
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        let Ok(Some(_workspace)) = workspace_dao.get_by_id(workspace_id) else {
            return;
        };

        // Update last accessed
        let _ = workspace_dao.update_last_accessed(workspace_id);

        // Create a new tab with the workspace's working directory
        // For now, default to Claude agent
        self.tab_manager.new_tab(AgentType::Claude);
        // TODO: Store workspace_id in session and use workspace.path as working_dir
    }

    /// Add a repository from the dialog
    fn add_repository(&mut self) {
        let path = self.add_repo_dialog_state.expanded_path();
        let name = self
            .add_repo_dialog_state
            .repo_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        let Some(repo_dao) = &self.repo_dao else {
            return;
        };
        let Some(workspace_dao) = &self.workspace_dao else {
            return;
        };

        // Create repository
        let repo = Repository::from_local_path(&name, path.clone());
        if repo_dao.create(&repo).is_err() {
            return;
        }

        // Get current branch
        let branch = self
            .worktree_manager
            .get_current_branch(&path)
            .unwrap_or_else(|_| "main".to_string());

        // Create default workspace
        let workspace = Workspace::new_default(repo.id, &branch, &branch, path);
        let _ = workspace_dao.create(&workspace);

        // Refresh sidebar
        self.refresh_sidebar_data();
    }

    /// Create a tab with the selected agent type, using pending_project_path if set
    fn create_tab_with_agent(&mut self, agent_type: AgentType) {
        // If we have a pending project path, add it as a repository first
        if let Some(project_path) = self.pending_project_path.take() {
            let name = project_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string();

            if let (Some(repo_dao), Some(workspace_dao)) =
                (&self.repo_dao, &self.workspace_dao)
            {
                // Create repository
                let repo = Repository::from_local_path(&name, project_path.clone());
                if repo_dao.create(&repo).is_ok() {
                    // Get current branch
                    let branch = self
                        .worktree_manager
                        .get_current_branch(&project_path)
                        .unwrap_or_else(|_| "main".to_string());

                    // Create default workspace
                    let workspace =
                        Workspace::new_default(repo.id, &branch, &branch, project_path);
                    let _ = workspace_dao.create(&workspace);

                    // Refresh sidebar
                    self.refresh_sidebar_data();
                }
            }
        }

        // Create the tab
        self.tab_manager.new_tab(agent_type);

        // Clear first-time splash since we now have a project
        self.show_first_time_splash = false;

        self.input_mode = InputMode::Normal;
    }

    fn handle_mouse_event(&mut self, mouse: event::MouseEvent) {
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

        // Start agent
        let mut config = AgentStartConfig::new(prompt, self.config.working_dir.clone())
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
