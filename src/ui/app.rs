use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    load_claude_history_with_debug, load_codex_history_with_debug, AgentEvent, AgentRunner, AgentStartConfig,
    AgentType, ClaudeCodeRunner, CodexCliRunner, HistoryDebugEntry, MessageDisplay, SessionId,
};
use crate::config::{parse_action, parse_key_notation, Config, KeyCombo, KeyContext, COMMAND_NAMES};
use crate::ui::action::Action;
use crate::data::{
    AppStateDao, Database, Repository, RepositoryDao, SessionTab, SessionTabDao, WorkspaceDao,
};
use crate::git::{PrManager, WorktreeManager};
use crate::ui::components::{
    AddRepoDialog, AddRepoDialogState, AgentSelector, AgentSelectorState, BaseDirDialog,
    BaseDirDialogState, ChatMessage, ConfirmationContext, ConfirmationDialog,
    ConfirmationDialogState, ConfirmationType, ErrorDialog, ErrorDialogState, EventDirection,
    GlobalFooter, HelpDialog, HelpDialogState, ModelSelector, ModelSelectorState, ProcessingState,
    ProjectPicker, ProjectPickerState, Sidebar, SidebarData, SidebarState, SplashScreen, TabBar,
};
use crate::ui::events::{AppEvent, InputMode, ViewMode};
use crate::ui::session::AgentSession;
use crate::ui::tab_manager::TabManager;

/// Performance metrics for monitoring frame timing
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// Time for the last complete frame
    pub frame_time: Duration,
    /// Time spent in terminal.draw()
    pub draw_time: Duration,
    /// Time spent processing events
    pub event_time: Duration,
    /// Calculated FPS (rolling average)
    pub fps: f64,
    /// Input-to-render latency for the last scroll event
    pub scroll_latency: Duration,
    /// Average scroll input-to-render latency
    pub scroll_latency_avg: Duration,
    /// Scroll lines per second (rolling window)
    pub scroll_lines_per_sec: f64,
    /// Scroll events per second (rolling window)
    pub scroll_events_per_sec: f64,
    /// Whether scroll activity happened recently
    pub scroll_active: bool,
    /// History of frame times for FPS calculation
    frame_history: VecDeque<Duration>,
    /// Scroll latency history for averaging
    scroll_latency_history: VecDeque<Duration>,
    /// Scroll events for rolling throughput (timestamp, lines)
    scroll_events: VecDeque<(Instant, usize)>,
    /// Last scroll input time
    last_scroll_input_at: Option<Instant>,
    /// Whether a scroll latency sample is pending next render
    pending_scroll_latency: bool,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMetrics {
    pub fn new() -> Self {
        Self {
            frame_time: Duration::ZERO,
            draw_time: Duration::ZERO,
            event_time: Duration::ZERO,
            fps: 0.0,
            frame_history: VecDeque::with_capacity(60),
            scroll_latency: Duration::ZERO,
            scroll_latency_avg: Duration::ZERO,
            scroll_lines_per_sec: 0.0,
            scroll_events_per_sec: 0.0,
            scroll_active: false,
            scroll_latency_history: VecDeque::with_capacity(120),
            scroll_events: VecDeque::with_capacity(240),
            last_scroll_input_at: None,
            pending_scroll_latency: false,
        }
    }

    /// Record a frame's duration and update FPS
    pub fn record_frame(&mut self, duration: Duration) {
        self.frame_time = duration;
        self.frame_history.push_back(duration);
        if self.frame_history.len() > 60 {
            self.frame_history.pop_front();
        }
        self.update_fps();
    }

    fn update_fps(&mut self) {
        if self.frame_history.is_empty() {
            self.fps = 0.0;
            return;
        }
        let total: Duration = self.frame_history.iter().sum();
        let avg = total.as_secs_f64() / self.frame_history.len() as f64;
        self.fps = if avg > 0.0 { 1.0 / avg } else { 0.0 };
    }

    /// Record a scroll input event with number of lines moved
    pub fn record_scroll_event(&mut self, lines: usize) {
        let now = Instant::now();
        self.last_scroll_input_at = Some(now);
        self.pending_scroll_latency = true;
        self.scroll_events.push_back((now, lines));
    }

    /// Update scroll throughput/active metrics at end of frame
    pub fn on_frame_end(&mut self, frame_end: Instant) {
        let window = Duration::from_secs(1);

        // Prune old scroll events outside the rolling window
        while let Some((ts, _)) = self.scroll_events.front() {
            if frame_end.duration_since(*ts) > window {
                self.scroll_events.pop_front();
            } else {
                break;
            }
        }

        let total_lines: usize = self.scroll_events.iter().map(|(_, lines)| *lines).sum();
        let events = self.scroll_events.len();
        let window_secs = window.as_secs_f64();
        self.scroll_lines_per_sec = total_lines as f64 / window_secs;
        self.scroll_events_per_sec = events as f64 / window_secs;

        self.scroll_active = self
            .last_scroll_input_at
            .map(|ts| frame_end.duration_since(ts) <= window)
            .unwrap_or(false);

    }

    /// Record scroll latency at draw completion
    pub fn on_draw_end(&mut self, draw_end: Instant) {
        if !self.pending_scroll_latency {
            return;
        }

        if let Some(input_at) = self.last_scroll_input_at {
            let latency = draw_end.duration_since(input_at);
            self.scroll_latency = latency;
            self.scroll_latency_history.push_back(latency);
            if self.scroll_latency_history.len() > 120 {
                self.scroll_latency_history.pop_front();
            }
            if !self.scroll_latency_history.is_empty() {
                let total: Duration = self.scroll_latency_history.iter().sum();
                let avg = total.as_secs_f64() / self.scroll_latency_history.len() as f64;
                self.scroll_latency_avg = Duration::from_secs_f64(avg);
            }
        }
        self.pending_scroll_latency = false;
    }
}

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
    /// Error dialog state
    error_dialog_state: ErrorDialogState,
    /// Help dialog state
    help_dialog_state: HelpDialogState,
    /// Command mode buffer
    command_buffer: String,
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
    /// Performance metrics for monitoring frame timing
    metrics: PerformanceMetrics,
    /// Whether to show performance metrics in status bar
    show_metrics: bool,
    /// Spinner frame counter for tab bar animations
    spinner_frame: usize,
    /// Last sidebar click time (for double-click detection)
    last_sidebar_click: Option<(Instant, usize)>, // (time, clicked_index)
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
            error_dialog_state: ErrorDialogState::new(),
            help_dialog_state: HelpDialogState::new(),
            command_buffer: String::new(),
            // Layout areas initialized to None, will be set during draw()
            sidebar_area: None,
            tab_bar_area: None,
            chat_area: None,
            input_area: None,
            status_bar_area: None,
            footer_area: None,
            // Performance metrics
            metrics: PerformanceMetrics::new(),
            show_metrics: false,
            spinner_frame: 0,
            // Double-click tracking
            last_sidebar_click: None,
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

            // Look up workspace to get working_dir, workspace_name, and project_name
            if let Some(workspace_id) = tab.workspace_id {
                if let Some(workspace_dao) = &self.workspace_dao {
                    if let Ok(Some(workspace)) = workspace_dao.get_by_id(workspace_id) {
                        session.working_dir = Some(workspace.path);
                        session.workspace_name = Some(workspace.name.clone());

                        // Look up repository for project name
                        if let Some(repo_dao) = &self.repo_dao {
                            if let Ok(Some(repo)) = repo_dao.get_by_id(workspace.repository_id) {
                                session.project_name = Some(repo.name);
                            }
                        }
                    }
                }
            }

            // Set resume session ID if available
            if let Some(ref session_id_str) = tab.agent_session_id {
                let session_id = SessionId::from_string(session_id_str.clone());
                session.resume_session_id = Some(session_id.clone());
                session.agent_session_id = Some(session_id.clone());

                // Load chat history from agent files
                match tab.agent_type {
                    AgentType::Claude => {
                        if let Ok((msgs, debug_entries, file_path)) =
                            load_claude_history_with_debug(session_id_str)
                        {
                            // Populate debug pane with history load info
                            Self::populate_debug_from_history(
                                &mut session.raw_events_view,
                                &debug_entries,
                                &file_path,
                            );
                            for msg in msgs {
                                session.chat_view.push(msg);
                            }
                        }
                    }
                    AgentType::Codex => {
                        if let Ok((msgs, debug_entries, file_path)) =
                            load_codex_history_with_debug(session_id_str)
                        {
                            // Populate debug pane with history load info
                            Self::populate_debug_from_history(
                                &mut session.raw_events_view,
                                &debug_entries,
                                &file_path,
                            );
                            for msg in msgs {
                                session.chat_view.push(msg);
                            }
                        }
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
        // Capture current expansion state before rebuild
        let expanded_repos = self.sidebar_data.expanded_repo_ids();

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

        // Restore expansion state
        for repo_id in expanded_repos {
            self.sidebar_data.expand_repo(repo_id);
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
            let frame_start = Instant::now();

            // Draw UI with timing
            let draw_start = Instant::now();
            terminal.draw(|f| self.draw(f))?;
            let draw_end = Instant::now();
            self.metrics.draw_time = draw_end.duration_since(draw_start);
            self.metrics.on_draw_end(draw_end);

            // Wait for next frame (16ms target = 60 FPS)
            tokio::select! {
                // Terminal input events + tick
                _ = tokio::time::sleep(Duration::from_millis(16)) => {
                    // Measure event processing time (after sleep)
                    let event_start = Instant::now();

                    // Handle keyboard and mouse input
                    let mut pending_scroll_up = 0usize;
                    let mut pending_scroll_down = 0usize;

                    while event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Key(key) => {
                                self.flush_scroll_deltas(&mut pending_scroll_up, &mut pending_scroll_down);
                                self.handle_key_event(key).await?;
                            }
                            Event::Mouse(mouse) => {
                                match mouse.kind {
                                    MouseEventKind::ScrollUp => {
                                        if self.should_route_scroll_to_chat() {
                                            self.record_chat_scroll(1);
                                        }
                                        pending_scroll_up = pending_scroll_up.saturating_add(1);
                                    }
                                    MouseEventKind::ScrollDown => {
                                        if self.should_route_scroll_to_chat() {
                                            self.record_chat_scroll(1);
                                        }
                                        pending_scroll_down = pending_scroll_down.saturating_add(1);
                                    }
                                    _ => {
                                        self.flush_scroll_deltas(
                                            &mut pending_scroll_up,
                                            &mut pending_scroll_down,
                                        );
                                        if let Some(action) = self.handle_mouse_event(mouse) {
                                            self.execute_action(action).await?;
                                        }
                                    }
                                }
                            }
                            _ => {
                                self.flush_scroll_deltas(&mut pending_scroll_up, &mut pending_scroll_down);
                            }
                        }
                    }

                    self.flush_scroll_deltas(&mut pending_scroll_up, &mut pending_scroll_down);

                    // Tick animations (every 6 frames = ~100ms)
                    self.tick_count += 1;
                    if self.tick_count % 6 == 0 {
                        // Advance spinner frame for PR processing indicator
                        self.spinner_frame = self.spinner_frame.wrapping_add(1);

                        // Tick confirmation dialog spinner (for loading state)
                        self.confirmation_dialog_state.tick();

                        if self.show_first_time_splash {
                            // Animate splash screen
                            self.splash_screen.tick();
                        } else if let Some(session) = self.tab_manager.active_session_mut() {
                            session.tick();
                        }
                    }

                    self.metrics.event_time = event_start.elapsed();
                }

                // App events from channel
                Some(event) = self.event_rx.recv() => {
                    let event_start = Instant::now();
                    self.handle_app_event(event).await?;
                    self.metrics.event_time = event_start.elapsed();
                }
            }

            // Record total frame time (includes sleep for accurate FPS)
            let frame_end = Instant::now();
            self.metrics
                .record_frame(frame_end.duration_since(frame_start));
            self.metrics.on_frame_end(frame_end);

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_key_event(&mut self, key: event::KeyEvent) -> anyhow::Result<()> {
        // Special handling for modes that bypass normal key processing
        if self.input_mode == InputMode::RemovingProject {
            // Ignore all input while removing project
            return Ok(());
        }

        // First-time splash screen handling (only when no dialogs are visible)
        if self.show_first_time_splash
            && key.modifiers.is_empty()
            && !self.base_dir_dialog_state.is_visible()
            && !self.project_picker_state.is_visible()
            && !self.add_repo_dialog_state.is_visible()
            && self.input_mode != InputMode::SelectingAgent
            && self.input_mode != InputMode::ShowingError
        {
            match key.code {
                KeyCode::Enter => {
                    // Start new project workflow
                    self.execute_action(Action::NewProject).await?;
                    return Ok(());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.execute_action(Action::Quit).await?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Global command mode trigger - ':' from most modes enters command mode
        if key.code == KeyCode::Char(':')
            && key.modifiers.is_empty()
            && !matches!(
                self.input_mode,
                InputMode::Command
                    | InputMode::ShowingHelp
                    | InputMode::AddingRepository
                    | InputMode::SettingBaseDir
                    | InputMode::PickingProject
                    | InputMode::ShowingError
                    | InputMode::SelectingAgent
                    | InputMode::Confirming
            )
        {
            self.command_buffer.clear();
            self.input_mode = InputMode::Command;
            return Ok(());
        }

        // Get the current context from input mode and view mode
        let context = KeyContext::from_input_mode(self.input_mode, self.view_mode);

        // Text input (typing characters) handled specially
        if self.should_handle_as_text_input(&key, context) {
            self.handle_text_input(key);
            return Ok(());
        }

        // Convert key event to KeyCombo for lookup
        let key_combo = KeyCombo::from_key_event(&key);

        // Look up action in config (context-specific first, then global)
        if let Some(action) = self.config.keybindings.get_action(&key_combo, context) {
            self.execute_action(action.clone()).await?;
        }

        Ok(())
    }

    /// Execute a keybinding action
    async fn execute_action(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            // ========== Global Actions ==========
            Action::Quit => {
                self.save_session_state();
                self.should_quit = true;
            }
            Action::ToggleSidebar => {
                self.sidebar_state.toggle();
                if self.sidebar_state.visible {
                    self.sidebar_state.set_focused(true);
                    self.input_mode = InputMode::SidebarNavigation;
                    // Focus on the current tab's workspace if it has one
                    if let Some(session) = self.tab_manager.active_session() {
                        if let Some(workspace_id) = session.workspace_id {
                            if let Some(index) = self.sidebar_data.focus_workspace(workspace_id) {
                                self.sidebar_state.tree_state.selected = index;
                            }
                        }
                    }
                } else {
                    self.sidebar_state.set_focused(false);
                    self.input_mode = InputMode::Normal;
                }
            }
            Action::NewProject => {
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
            Action::OpenPr => {
                self.handle_pr_action();
            }
            Action::InterruptAgent => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    if session.is_processing {
                        let display = MessageDisplay::System {
                            content: "Interrupted".to_string(),
                        };
                        session.chat_view.push(display.to_chat_message());
                        session.stop_processing();
                    }
                }
            }
            Action::ToggleViewMode => {
                self.view_mode = match self.view_mode {
                    ViewMode::Chat => ViewMode::RawEvents,
                    ViewMode::RawEvents => ViewMode::Chat,
                };
            }
            Action::ShowModelSelector => {
                if let Some(session) = self.tab_manager.active_session() {
                    self.model_selector_state.show(session.model.clone());
                    self.input_mode = InputMode::SelectingModel;
                }
            }
            Action::ToggleMetrics => {
                self.show_metrics = !self.show_metrics;
            }
            Action::DumpDebugState => {
                self.dump_debug_state();
            }

            // ========== Tab Management ==========
            Action::CloseTab => {
                let active = self.tab_manager.active_index();
                self.tab_manager.close_tab(active);
                if self.tab_manager.is_empty() {
                    self.sidebar_state.visible = true;
                    self.input_mode = InputMode::SidebarNavigation;
                }
            }
            Action::NextTab => {
                // Include sidebar in tab cycle when visible
                if self.input_mode == InputMode::SidebarNavigation {
                    // From sidebar, go to first tab
                    if !self.tab_manager.is_empty() {
                        self.tab_manager.switch_to(0);
                        self.sidebar_state.set_focused(false);
                        self.input_mode = InputMode::Normal;
                    }
                } else if self.sidebar_state.visible {
                    // Check if on last tab - if so, go to sidebar
                    let current = self.tab_manager.active_index();
                    let count = self.tab_manager.len();
                    if count > 0 && current == count - 1 {
                        // On last tab, go to sidebar
                        self.sidebar_state.set_focused(true);
                        self.input_mode = InputMode::SidebarNavigation;
                    } else {
                        self.tab_manager.next_tab();
                    }
                } else {
                    self.tab_manager.next_tab();
                }
            }
            Action::PrevTab => {
                // Include sidebar in tab cycle when visible
                if self.input_mode == InputMode::SidebarNavigation {
                    // From sidebar, go to last tab
                    let count = self.tab_manager.len();
                    if count > 0 {
                        self.tab_manager.switch_to(count - 1);
                        self.sidebar_state.set_focused(false);
                        self.input_mode = InputMode::Normal;
                    }
                } else if self.sidebar_state.visible {
                    // Check if on first tab - if so, go to sidebar
                    let current = self.tab_manager.active_index();
                    if current == 0 {
                        // On first tab, go to sidebar
                        self.sidebar_state.set_focused(true);
                        self.input_mode = InputMode::SidebarNavigation;
                    } else {
                        self.tab_manager.prev_tab();
                    }
                } else {
                    self.tab_manager.prev_tab();
                }
            }
            Action::SwitchToTab(n) => {
                if n > 0 {
                    self.tab_manager.switch_to((n - 1) as usize);
                }
            }

            // ========== Chat Scrolling ==========
            Action::ScrollUp(n) => {
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.scroll_up(n as usize);
                } else {
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.chat_view.scroll_up(n as usize);
                    }
                    self.record_chat_scroll(n as usize);
                }
            }
            Action::ScrollDown(n) => {
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.scroll_down(n as usize);
                } else {
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.chat_view.scroll_down(n as usize);
                    }
                    self.record_chat_scroll(n as usize);
                }
            }
            Action::ScrollPageUp => {
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.page_up();
                } else {
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.chat_view.scroll_up(10);
                    }
                    self.record_chat_scroll(10);
                }
            }
            Action::ScrollPageDown => {
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.page_down();
                } else {
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.chat_view.scroll_down(10);
                    }
                    self.record_chat_scroll(10);
                }
            }
            Action::ScrollToTop => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_to_top();
                }
            }
            Action::ScrollToBottom => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_to_bottom();
                }
            }

            // ========== Input Box Editing ==========
            Action::InsertNewline => {
                // Don't insert newlines in help dialog or command mode
                if self.input_mode != InputMode::ShowingHelp
                    && self.input_mode != InputMode::Command
                {
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.insert_newline();
                    }
                }
            }
            Action::Backspace => {
                match self.input_mode {
                    InputMode::Command => {
                        if self.command_buffer.is_empty() {
                            // Exit command mode if buffer is empty
                            self.input_mode = InputMode::Normal;
                        } else {
                            self.command_buffer.pop();
                        }
                    }
                    InputMode::ShowingHelp => {
                        self.help_dialog_state.delete_char();
                    }
                    _ => {
                        if let Some(session) = self.tab_manager.active_session_mut() {
                            session.input_box.backspace();
                        }
                    }
                }
            }
            Action::Delete => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.delete();
                }
            }
            Action::DeleteWordBack => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.delete_word_back();
                }
            }
            Action::DeleteWordForward => {
                // TODO: implement delete_word_forward in InputBox
            }
            Action::DeleteToStart => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.delete_to_start();
                }
            }
            Action::DeleteToEnd => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.delete_to_end();
                }
            }
            Action::MoveCursorLeft => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_left();
                }
            }
            Action::MoveCursorRight => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_right();
                }
            }
            Action::MoveCursorStart => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_start();
                }
            }
            Action::MoveCursorEnd => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_end();
                }
            }
            Action::MoveWordLeft => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_word_left();
                }
            }
            Action::MoveWordRight => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.move_word_right();
                }
            }
            Action::MoveCursorUp => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    if !session.input_box.move_up() {
                        if session.input_box.is_cursor_on_first_line() {
                            session.input_box.history_prev();
                        }
                    }
                }
            }
            Action::MoveCursorDown => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    if !session.input_box.move_down() {
                        if session.input_box.is_cursor_on_last_line() {
                            session.input_box.history_next();
                        }
                    }
                }
            }
            Action::HistoryPrev => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.history_prev();
                }
            }
            Action::HistoryNext => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.history_next();
                }
            }
            Action::Submit => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    if !session.input_box.is_empty() {
                        let prompt = session.input_box.submit();
                        self.submit_prompt(prompt).await?;
                    }
                }
            }

            // ========== List/Tree Navigation ==========
            Action::SelectNext => {
                match self.input_mode {
                    InputMode::SidebarNavigation => {
                        let visible_count = self.sidebar_data.visible_nodes().len();
                        self.sidebar_state.tree_state.select_next(visible_count);
                    }
                    InputMode::SelectingModel => {
                        self.model_selector_state.select_next();
                    }
                    InputMode::SelectingAgent => {
                        self.agent_selector_state.select_next();
                    }
                    InputMode::PickingProject => {
                        self.project_picker_state.select_next();
                    }
                    _ => {}
                }
            }
            Action::SelectPrev => {
                match self.input_mode {
                    InputMode::SidebarNavigation => {
                        let visible_count = self.sidebar_data.visible_nodes().len();
                        self.sidebar_state.tree_state.select_previous(visible_count);
                    }
                    InputMode::SelectingModel => {
                        self.model_selector_state.select_previous();
                    }
                    InputMode::SelectingAgent => {
                        self.agent_selector_state.select_previous();
                    }
                    InputMode::PickingProject => {
                        self.project_picker_state.select_prev();
                    }
                    _ => {}
                }
            }
            Action::SelectPageDown => {
                if self.input_mode == InputMode::PickingProject {
                    self.project_picker_state.page_down();
                }
            }
            Action::SelectPageUp => {
                if self.input_mode == InputMode::PickingProject {
                    self.project_picker_state.page_up();
                }
            }
            Action::Confirm => {
                match self.input_mode {
                    InputMode::SidebarNavigation => {
                        let selected = self.sidebar_state.tree_state.selected;
                        if let Some(node) = self.sidebar_data.get_at(selected) {
                            use crate::ui::components::{ActionType, NodeType};
                            match node.node_type {
                                NodeType::Action(ActionType::NewWorkspace) => {
                                    if let Some(parent_id) = node.parent_id {
                                        self.start_workspace_creation(parent_id);
                                    }
                                }
                                NodeType::Workspace => {
                                    self.open_workspace(node.id);
                                    self.input_mode = InputMode::Normal;
                                    self.sidebar_state.set_focused(false);
                                }
                                NodeType::Repository => {
                                    self.sidebar_data.toggle_at(selected);
                                }
                            }
                        }
                    }
                    InputMode::SelectingModel => {
                        if let Some(model) = self.model_selector_state.selected_model() {
                            let model_id = model.id.clone();
                            let agent_type = model.agent_type;
                            let display_name = model.display_name.clone();
                            if let Some(session) = self.tab_manager.active_session_mut() {
                                let agent_changed = session.agent_type != agent_type;
                                session.model = Some(model_id.clone());
                                session.agent_type = agent_type;
                                session.update_status();
                                let msg = if agent_changed {
                                    format!("Switched to {} with model: {}", agent_type, display_name)
                                } else {
                                    format!("Model changed to: {}", display_name)
                                };
                                let display = MessageDisplay::System { content: msg };
                                session.chat_view.push(display.to_chat_message());
                            }
                        }
                        self.model_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::SelectingAgent => {
                        let agent_type = self.agent_selector_state.selected_agent();
                        self.agent_selector_state.hide();
                        self.create_tab_with_agent(agent_type);
                    }
                    InputMode::PickingProject => {
                        if let Some(project) = self.project_picker_state.selected_project() {
                            let repo_id = self.add_project_to_sidebar(project.path.clone());
                            self.project_picker_state.hide();
                            if let Some(id) = repo_id {
                                self.sidebar_data.expand_repo(id);
                                if let Some(repo_index) = self.sidebar_data.find_repo_index(id) {
                                    self.sidebar_state.tree_state.selected = repo_index + 1;
                                }
                                self.sidebar_state.show();
                                self.sidebar_state.set_focused(true);
                                self.show_first_time_splash = false;
                                self.input_mode = InputMode::SidebarNavigation;
                            } else {
                                self.input_mode = InputMode::Normal;
                            }
                        }
                    }
                    InputMode::AddingRepository => {
                        if self.add_repo_dialog_state.is_valid {
                            let repo_id = self.add_repository();
                            self.add_repo_dialog_state.hide();
                            if let Some(id) = repo_id {
                                self.sidebar_data.expand_repo(id);
                                if let Some(repo_index) = self.sidebar_data.find_repo_index(id) {
                                    self.sidebar_state.tree_state.selected = repo_index + 1;
                                }
                                self.sidebar_state.show();
                                self.sidebar_state.set_focused(true);
                                self.show_first_time_splash = false;
                                self.input_mode = InputMode::SidebarNavigation;
                            } else {
                                self.input_mode = InputMode::Normal;
                            }
                        }
                    }
                    InputMode::SettingBaseDir => {
                        if self.base_dir_dialog_state.is_valid {
                            if let Some(dao) = &self.app_state_dao {
                                if let Err(e) = dao.set("projects_base_dir", self.base_dir_dialog_state.input()) {
                                    self.base_dir_dialog_state.hide();
                                    self.show_error(
                                        "Failed to Save",
                                        &format!("Could not save projects directory: {}", e),
                                    );
                                    return Ok(());
                                }
                            }
                            let base_path = self.base_dir_dialog_state.expanded_path();
                            self.base_dir_dialog_state.hide();
                            self.project_picker_state.show(base_path);
                            self.input_mode = InputMode::PickingProject;
                        }
                    }
                    InputMode::Confirming => {
                        if self.confirmation_dialog_state.is_confirm_selected() {
                            if let Some(context) = self.confirmation_dialog_state.context.clone() {
                                match context {
                                    ConfirmationContext::ArchiveWorkspace(id) => {
                                        self.execute_archive_workspace(id);
                                        self.confirmation_dialog_state.hide();
                                        self.input_mode = InputMode::SidebarNavigation;
                                        return Ok(());
                                    }
                                    ConfirmationContext::RemoveProject(id) => {
                                        self.execute_remove_project(id);
                                        self.confirmation_dialog_state.hide();
                                        self.input_mode = InputMode::SidebarNavigation;
                                        return Ok(());
                                    }
                                    ConfirmationContext::CreatePullRequest { preflight, .. } => {
                                        self.confirmation_dialog_state.hide();
                                        self.input_mode = InputMode::Normal;
                                        self.submit_pr_workflow(preflight).await?;
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        self.confirmation_dialog_state.hide();
                        self.input_mode = InputMode::SidebarNavigation;
                    }
                    InputMode::ShowingError => {
                        self.error_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    _ => {}
                }
            }
            Action::Cancel => {
                match self.input_mode {
                    InputMode::SidebarNavigation => {
                        self.input_mode = InputMode::Normal;
                        self.sidebar_state.set_focused(false);
                    }
                    InputMode::SelectingModel => {
                        self.model_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::SelectingAgent => {
                        self.agent_selector_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::PickingProject => {
                        self.project_picker_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::AddingRepository => {
                        self.add_repo_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::SettingBaseDir => {
                        self.base_dir_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::Confirming => {
                        self.confirmation_dialog_state.hide();
                        if matches!(
                            self.confirmation_dialog_state.context,
                            Some(ConfirmationContext::CreatePullRequest { .. })
                        ) {
                            self.input_mode = InputMode::Normal;
                        } else {
                            self.input_mode = InputMode::SidebarNavigation;
                        }
                    }
                    InputMode::ShowingError => {
                        self.error_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::Scrolling => {
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::Command => {
                        self.command_buffer.clear();
                        self.input_mode = InputMode::Normal;
                    }
                    InputMode::ShowingHelp => {
                        self.help_dialog_state.hide();
                        self.input_mode = InputMode::Normal;
                    }
                    _ => {}
                }
            }
            Action::ExpandOrSelect => {
                // Same as Confirm for sidebar
                if self.input_mode == InputMode::SidebarNavigation {
                    let selected = self.sidebar_state.tree_state.selected;
                    if let Some(node) = self.sidebar_data.get_at(selected) {
                        use crate::ui::components::{ActionType, NodeType};
                        match node.node_type {
                            NodeType::Action(ActionType::NewWorkspace) => {
                                if let Some(parent_id) = node.parent_id {
                                    self.start_workspace_creation(parent_id);
                                }
                            }
                            NodeType::Workspace => {
                                self.open_workspace(node.id);
                                self.input_mode = InputMode::Normal;
                                self.sidebar_state.set_focused(false);
                            }
                            NodeType::Repository => {
                                self.sidebar_data.toggle_at(selected);
                            }
                        }
                    }
                }
            }
            Action::Collapse => {
                if self.input_mode == InputMode::SidebarNavigation {
                    let selected = self.sidebar_state.tree_state.selected;
                    if let Some(node) = self.sidebar_data.get_at(selected) {
                        if !node.is_leaf() && node.expanded {
                            self.sidebar_data.toggle_at(selected);
                        }
                    }
                }
            }
            Action::AddRepository => {
                match self.input_mode {
                    InputMode::SidebarNavigation => {
                        self.add_repo_dialog_state.show();
                        self.input_mode = InputMode::AddingRepository;
                    }
                    InputMode::PickingProject => {
                        self.project_picker_state.hide();
                        self.add_repo_dialog_state.show();
                        self.input_mode = InputMode::AddingRepository;
                    }
                    _ => {}
                }
            }
            Action::OpenSettings => {
                if self.input_mode == InputMode::SidebarNavigation {
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
            }
            Action::ArchiveOrRemove => {
                if self.input_mode == InputMode::SidebarNavigation {
                    let selected = self.sidebar_state.tree_state.selected;
                    if let Some(node) = self.sidebar_data.get_at(selected) {
                        use crate::ui::components::NodeType;
                        match node.node_type {
                            NodeType::Workspace => {
                                self.initiate_archive_workspace(node.id);
                            }
                            NodeType::Repository => {
                                self.initiate_remove_project(node.id);
                            }
                            _ => {}
                        }
                    }
                }
            }

            // ========== Sidebar Navigation ==========
            Action::EnterSidebarMode => {
                self.sidebar_state.show();
                self.sidebar_state.set_focused(true);
                self.input_mode = InputMode::SidebarNavigation;
            }
            Action::ExitSidebarMode => {
                self.sidebar_state.set_focused(false);
                self.input_mode = InputMode::Normal;
            }

            // ========== Raw Events View ==========
            Action::RawEventsSelectNext => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.raw_events_view.select_next();
                }
            }
            Action::RawEventsSelectPrev => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.raw_events_view.select_prev();
                }
            }
            Action::RawEventsToggleExpand => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.raw_events_view.toggle_expand();
                }
            }
            Action::RawEventsCollapse => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.raw_events_view.collapse();
                }
            }

            // ========== Confirmation Dialog ==========
            Action::ConfirmYes => {
                if self.input_mode == InputMode::Confirming {
                    if let Some(context) = self.confirmation_dialog_state.context.clone() {
                        match context {
                            ConfirmationContext::ArchiveWorkspace(id) => {
                                self.execute_archive_workspace(id);
                                self.confirmation_dialog_state.hide();
                                self.input_mode = InputMode::SidebarNavigation;
                            }
                            ConfirmationContext::RemoveProject(id) => {
                                self.execute_remove_project(id);
                                self.confirmation_dialog_state.hide();
                                self.input_mode = InputMode::SidebarNavigation;
                            }
                            ConfirmationContext::CreatePullRequest { preflight, .. } => {
                                self.confirmation_dialog_state.hide();
                                self.input_mode = InputMode::Normal;
                                self.submit_pr_workflow(preflight).await?;
                            }
                        }
                    }
                }
            }
            Action::ConfirmNo => {
                if self.input_mode == InputMode::Confirming {
                    self.confirmation_dialog_state.hide();
                    self.input_mode = InputMode::SidebarNavigation;
                }
            }
            Action::ConfirmToggle => {
                if self.input_mode == InputMode::Confirming {
                    self.confirmation_dialog_state.toggle_selection();
                }
            }
            Action::ToggleDetails => {
                if self.input_mode == InputMode::ShowingError {
                    self.error_dialog_state.toggle_details();
                }
            }

            // ========== Agent Selection ==========
            Action::SelectAgent => {
                if self.input_mode == InputMode::SelectingAgent {
                    let agent_type = self.agent_selector_state.selected_agent();
                    self.agent_selector_state.hide();
                    self.create_tab_with_agent(agent_type);
                }
            }

            // ========== Command Mode ==========
            Action::ShowHelp => {
                self.help_dialog_state.show(&self.config.keybindings);
                self.input_mode = InputMode::ShowingHelp;
            }
            Action::ExecuteCommand => {
                if self.input_mode == InputMode::Command {
                    if let Some(action) = self.execute_command() {
                        // Prevent recursion - ExecuteCommand can't call itself
                        if !matches!(action, Action::ExecuteCommand) {
                            Box::pin(self.execute_action(action)).await?;
                        }
                    }
                }
            }
            Action::CompleteCommand => {
                if self.input_mode == InputMode::Command {
                    self.complete_command();
                }
            }
        }

        Ok(())
    }

    /// Check if a key event should be handled as text input
    /// Returns true if the key is a printable character without Control/Alt modifiers
    /// and we're in a text-input context
    fn should_handle_as_text_input(&self, key: &event::KeyEvent, context: KeyContext) -> bool {
        // Only handle plain characters (no Ctrl or Alt)
        let has_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
            || key.modifiers.contains(KeyModifiers::ALT);

        if has_modifier {
            return false;
        }

        // Check if this is a character key
        let is_char = matches!(key.code, KeyCode::Char(_));

        if !is_char {
            return false;
        }

        // Only treat as text input in appropriate contexts
        matches!(
            context,
            KeyContext::Chat
                | KeyContext::AddRepository
                | KeyContext::BaseDir
                | KeyContext::ProjectPicker
                | KeyContext::Command
                | KeyContext::HelpDialog
        )
    }

    /// Handle text input for text-input contexts
    fn handle_text_input(&mut self, key: event::KeyEvent) {
        let KeyCode::Char(c) = key.code else {
            return;
        };

        match self.input_mode {
            InputMode::Normal => {
                // Note: ':' is handled globally in handle_key_event
                // Check for help trigger (? on empty input)
                if c == '?' {
                    if let Some(session) = self.tab_manager.active_session() {
                        if session.input_box.input().is_empty() {
                            self.help_dialog_state.show(&self.config.keybindings);
                            self.input_mode = InputMode::ShowingHelp;
                            return;
                        }
                    }
                }
                if let Some(session) = self.tab_manager.active_session_mut() {
                    session.input_box.insert_char(c);
                }
            }
            InputMode::Command => {
                self.command_buffer.push(c);
            }
            InputMode::ShowingHelp => {
                self.help_dialog_state.insert_char(c);
            }
            InputMode::AddingRepository => {
                self.add_repo_dialog_state.insert_char(c);
            }
            InputMode::SettingBaseDir => {
                self.base_dir_dialog_state.insert_char(c);
            }
            InputMode::PickingProject => {
                self.project_picker_state.insert_char(c);
            }
            _ => {}
        }
    }

    /// Execute a command from command mode
    /// Returns an action to execute if the command maps to one
    fn execute_command(&mut self) -> Option<Action> {
        let command = self.command_buffer.trim().to_lowercase();
        self.command_buffer.clear();
        self.input_mode = InputMode::Normal;

        // First check for built-in command aliases
        match command.as_str() {
            "help" | "h" | "?" => {
                self.help_dialog_state.show(&self.config.keybindings);
                self.input_mode = InputMode::ShowingHelp;
                return None;
            }
            "q" => {
                return Some(Action::Quit);
            }
            _ => {}
        }

        // Try to parse as an action name
        parse_action(&command)
    }

    /// Autocomplete the command buffer
    fn complete_command(&mut self) {
        let prefix = self.command_buffer.trim().to_lowercase();
        if prefix.is_empty() {
            return;
        }

        // Find all matching commands
        let matches: Vec<&str> = COMMAND_NAMES
            .iter()
            .filter(|cmd| cmd.starts_with(&prefix))
            .copied()
            .collect();

        if matches.is_empty() {
            return;
        }

        if matches.len() == 1 {
            // Single match - complete fully
            self.command_buffer = matches[0].to_string();
        } else {
            // Multiple matches - complete to longest common prefix
            let common = Self::longest_common_prefix(&matches);
            if common.len() > prefix.len() {
                self.command_buffer = common;
            } else {
                // Already at common prefix - cycle to next match
                let current = &self.command_buffer;
                let next = matches
                    .iter()
                    .find(|&&cmd| cmd > current.as_str())
                    .or(matches.first())
                    .unwrap();
                self.command_buffer = (*next).to_string();
            }
        }
    }

    /// Find longest common prefix among strings
    fn longest_common_prefix(strings: &[&str]) -> String {
        if strings.is_empty() {
            return String::new();
        }
        if strings.len() == 1 {
            return strings[0].to_string();
        }

        let first = strings[0];
        let mut prefix_len = first.len();

        for s in &strings[1..] {
            prefix_len = first
                .chars()
                .zip(s.chars())
                .take_while(|(a, b)| a == b)
                .count()
                .min(prefix_len);
        }

        first[..prefix_len].to_string()
    }

    /// Open a workspace (create or switch to tab)
    /// If `close_sidebar` is true, the sidebar will be hidden after opening.
    fn open_workspace_with_options(&mut self, workspace_id: uuid::Uuid, close_sidebar: bool) {
        // Check if there's already a tab with this workspace - switch to it
        if let Some(existing_index) = self.find_tab_for_workspace(workspace_id) {
            self.tab_manager.switch_to(existing_index);
            if close_sidebar {
                self.sidebar_state.hide();
                self.input_mode = InputMode::Normal;
            }
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

        // Get the repository name for the tab title
        let project_name = self
            .repo_dao
            .as_ref()
            .and_then(|dao| dao.get_by_id(workspace.repository_id).ok().flatten())
            .map(|repo| repo.name);

        // Check if there's a saved session for this workspace (to restore chat history)
        let saved_tab = self
            .session_tab_dao
            .as_ref()
            .and_then(|dao| dao.get_by_workspace_id(workspace_id).ok().flatten());

        // Update last accessed
        let _ = workspace_dao.update_last_accessed(workspace_id);

        // Create a new tab with the workspace's working directory
        self.tab_manager
            .new_tab_with_working_dir(AgentType::Claude, workspace.path.clone());

        // Store workspace info in session and restore chat history if available
        if let Some(session) = self.tab_manager.active_session_mut() {
            session.workspace_id = Some(workspace_id);
            session.project_name = project_name;
            session.workspace_name = Some(workspace.name.clone());

            // Restore saved session data if available
            if let Some(saved) = saved_tab {
                session.model = saved.model;

                // Restore chat history from agent files
                if let Some(ref session_id_str) = saved.agent_session_id {
                    let session_id = SessionId::from_string(session_id_str.clone());
                    session.resume_session_id = Some(session_id.clone());
                    session.agent_session_id = Some(session_id);

                    // Load chat history
                    match saved.agent_type {
                        AgentType::Claude => {
                            if let Ok((msgs, debug_entries, file_path)) =
                                load_claude_history_with_debug(session_id_str)
                            {
                                // Populate debug pane with history load info
                                Self::populate_debug_from_history(
                                    &mut session.raw_events_view,
                                    &debug_entries,
                                    &file_path,
                                );
                                for msg in msgs {
                                    session.chat_view.push(msg);
                                }
                            }
                        }
                        AgentType::Codex => {
                            if let Ok((msgs, debug_entries, file_path)) =
                                load_codex_history_with_debug(session_id_str)
                            {
                                // Populate debug pane with history load info
                                Self::populate_debug_from_history(
                                    &mut session.raw_events_view,
                                    &debug_entries,
                                    &file_path,
                                );
                                for msg in msgs {
                                    session.chat_view.push(msg);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Close the sidebar and switch to normal mode (if requested)
        if close_sidebar {
            self.sidebar_state.hide();
            self.input_mode = InputMode::Normal;
        }
    }

    /// Open a workspace (create or switch to tab), closing the sidebar
    fn open_workspace(&mut self, workspace_id: uuid::Uuid) {
        self.open_workspace_with_options(workspace_id, true);
    }

    /// Find the tab index for a workspace if it's already open
    fn find_tab_for_workspace(&self, workspace_id: uuid::Uuid) -> Option<usize> {
        self.tab_manager
            .sessions()
            .iter()
            .position(|session| session.workspace_id == Some(workspace_id))
    }

    /// Populate the debug pane with history loading debug entries
    fn populate_debug_from_history(
        raw_events_view: &mut crate::ui::components::RawEventsView,
        debug_entries: &[HistoryDebugEntry],
        file_path: &std::path::Path,
    ) {
        use crate::ui::components::EventDirection;

        // First, add a header entry showing the file being loaded
        let header_json = serde_json::json!({
            "action": "history_load",
            "file": file_path.to_string_lossy(),
            "total_entries": debug_entries.len(),
            "included": debug_entries.iter().filter(|e| e.status == "INCLUDE").count(),
            "skipped": debug_entries.iter().filter(|e| e.status == "SKIP").count(),
        });
        raw_events_view.push_event(EventDirection::Received, "history_load", header_json);

        // Add each debug entry
        for entry in debug_entries {
            // Create a summary JSON that includes status info
            let summary_json = serde_json::json!({
                "line": entry.line_number,
                "type": entry.entry_type,
                "status": entry.status,
                "reason": entry.reason,
                "raw": entry.raw_json,
            });

            let event_type = format!("L{} {} {}", entry.line_number, entry.status, entry.entry_type);
            raw_events_view.push_event(EventDirection::Received, event_type, summary_json);
        }
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
                self.show_error(
                    "Workspace Creation Failed",
                    &format!("Failed to save workspace to database: {}", e),
                );
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

        // Open the workspace but keep sidebar open and focused
        // User can press Enter to close sidebar or continue navigating
        self.open_workspace_with_options(workspace_id, false);
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
            Some(ConfirmationContext::ArchiveWorkspace(workspace_id)),
        );
        self.input_mode = InputMode::Confirming;
    }

    /// Show an error dialog with a simple message
    fn show_error(&mut self, title: &str, message: &str) {
        self.error_dialog_state.show(title, message);
        self.input_mode = InputMode::ShowingError;
    }

    /// Show an error dialog with technical details
    fn show_error_with_details(&mut self, title: &str, message: &str, details: &str) {
        self.error_dialog_state.show_with_details(title, message, details);
        self.input_mode = InputMode::ShowingError;
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
        let mut worktree_error: Option<String> = None;
        if let Some(base_path) = repo_base_path {
            if let Err(e) = self.worktree_manager.remove_worktree(&base_path, &workspace.path) {
                tracing::error!(error = %e, "Failed to remove worktree");
                worktree_error = Some(format!("Failed to remove worktree: {}", e));
                // Continue anyway to mark as archived in DB
            }
        }

        // Mark workspace as archived in database
        if let Err(e) = workspace_dao.archive(workspace_id) {
            tracing::error!(error = %e, "Failed to archive workspace in database");
            self.show_error(
                "Archive Failed",
                &format!("Failed to archive workspace in database: {}", e),
            );
            return;
        }

        // Show worktree error if one occurred (but archive succeeded)
        if let Some(error_msg) = worktree_error {
            self.show_error_with_details(
                "Worktree Warning",
                "Workspace archived but worktree removal failed",
                &error_msg,
            );
            // Don't return - continue with cleanup since archive succeeded
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

    /// Initiate project removal - shows confirmation dialog
    fn initiate_remove_project(&mut self, repo_id: uuid::Uuid) {
        // Get repository info
        let Some(repo_dao) = &self.repo_dao else {
            return;
        };

        let Ok(Some(repo)) = repo_dao.get_by_id(repo_id) else {
            tracing::error!(repo_id = %repo_id, "Repository not found");
            return;
        };

        // Get all workspaces for this repository
        let workspaces = if let Some(workspace_dao) = &self.workspace_dao {
            workspace_dao.get_by_repository(repo_id).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Check git status for each workspace
        let mut warnings = Vec::new();
        let mut has_dirty = false;
        let mut has_unmerged = false;

        for ws in &workspaces {
            if let Ok(status) = self.worktree_manager.get_branch_status(&ws.path) {
                if status.is_dirty {
                    has_dirty = true;
                }
                if !status.is_merged {
                    has_unmerged = true;
                }
            }
        }

        // Build warning messages
        let workspace_count = workspaces.len();
        if workspace_count > 0 {
            warnings.push(format!(
                "{} workspace{} will be archived",
                workspace_count,
                if workspace_count == 1 { "" } else { "s" }
            ));
        }
        if has_dirty {
            warnings.push("Some workspaces have uncommitted changes".to_string());
        }
        if has_unmerged {
            warnings.push("Some branches are not merged to main".to_string());
        }

        // Determine confirmation type based on risk
        let confirmation_type = match (has_dirty, has_unmerged) {
            (true, true) => ConfirmationType::Danger,
            (true, false) | (false, true) => ConfirmationType::Warning,
            (false, false) => {
                if workspace_count > 0 {
                    ConfirmationType::Warning
                } else {
                    ConfirmationType::Info
                }
            }
        };

        // Show confirmation dialog
        self.confirmation_dialog_state.show(
            format!("Remove \"{}\"?", repo.name),
            "This will archive all workspaces and remove the project.",
            warnings,
            confirmation_type,
            "Remove",
            Some(ConfirmationContext::RemoveProject(repo_id)),
        );
        self.input_mode = InputMode::Confirming;
    }

    /// Execute project removal after confirmation
    fn execute_remove_project(&mut self, repo_id: uuid::Uuid) {
        // Set spinner mode
        self.input_mode = InputMode::RemovingProject;

        // Collect errors during removal
        let mut errors: Vec<String> = Vec::new();

        // Get repository info for base path and name
        let (repo_base_path, repo_name) = if let Some(repo_dao) = &self.repo_dao {
            match repo_dao.get_by_id(repo_id) {
                Ok(Some(repo)) => (repo.base_path, repo.name.clone()),
                _ => (None, String::from("Unknown")),
            }
        } else {
            (None, String::from("Unknown"))
        };

        // Get all workspaces for this repository
        let workspaces = if let Some(workspace_dao) = &self.workspace_dao {
            workspace_dao.get_by_repository(repo_id).unwrap_or_default()
        } else {
            Vec::new()
        };

        // Archive each workspace
        for ws in workspaces {
            // Remove git worktree
            if let Some(ref base_path) = repo_base_path {
                if let Err(e) = self.worktree_manager.remove_worktree(base_path, &ws.path) {
                    errors.push(format!("Failed to remove worktree '{}': {}", ws.name, e));
                }
            }

            // Archive in database
            if let Some(workspace_dao) = &self.workspace_dao {
                if let Err(e) = workspace_dao.archive(ws.id) {
                    errors.push(format!("Failed to archive workspace '{}': {}", ws.name, e));
                }
            }

            // Close any open tabs
            self.close_tabs_for_workspace(ws.id);
        }

        // Also remove the project folder from worktrees directory
        let worktrees_dir = crate::util::worktrees_dir();
        let project_worktrees_path = worktrees_dir.join(&repo_name);
        if project_worktrees_path.exists() {
            if let Err(e) = std::fs::remove_dir_all(&project_worktrees_path) {
                errors.push(format!("Failed to remove project folder: {}", e));
            }
        }

        // Delete repository from database
        if let Some(repo_dao) = &self.repo_dao {
            if let Err(e) = repo_dao.delete(repo_id) {
                tracing::error!(error = %e, "Failed to delete repository");
                errors.push(format!("Failed to delete repository from database: {}", e));
            }
        }

        // Show errors if any occurred
        if !errors.is_empty() {
            self.show_error_with_details(
                "Project Removal Errors",
                "Some operations failed during project removal",
                &errors.join("\n"),
            );
        } else {
            tracing::info!(repo_id = %repo_id, "Project removed successfully");
        }

        // Remember current selection
        let current_selection = self.sidebar_state.tree_state.selected;

        // Refresh sidebar
        self.refresh_sidebar_data();

        // Adjust selection (move up if needed)
        let visible_count = self.sidebar_data.visible_nodes().len();
        if visible_count > 0 {
            let new_selection = if current_selection > 0 {
                current_selection - 1
            } else {
                0
            };
            self.sidebar_state.tree_state.selected = new_selection.min(visible_count - 1);
            // Still have projects, go to sidebar navigation
            self.input_mode = InputMode::SidebarNavigation;
        } else {
            // No projects left, show splash screen
            self.sidebar_state.tree_state.selected = 0;
            self.show_first_time_splash = true;
            self.input_mode = InputMode::Normal;
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

        // Switch to sidebar navigation if all tabs are closed
        // But don't override if we're showing an error dialog
        if self.tab_manager.is_empty() && self.input_mode != InputMode::ShowingError {
            self.sidebar_state.visible = true;
            self.input_mode = InputMode::SidebarNavigation;
        }
    }

    /// Add a project to the sidebar (repository only, no workspace)
    /// Returns the repository ID - either existing or newly created
    fn add_project_to_sidebar(&mut self, path: std::path::PathBuf) -> Option<uuid::Uuid> {
        let Some(repo_dao) = &self.repo_dao else {
            return None;
        };

        // Check if project already exists
        if let Ok(Some(existing_repo)) = repo_dao.get_by_path(&path) {
            // Project already exists, just return its ID (caller will expand/select it)
            return Some(existing_repo.id);
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

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
    /// Returns the repository ID - either existing or newly created
    fn add_repository(&mut self) -> Option<uuid::Uuid> {
        let path = self.add_repo_dialog_state.expanded_path();

        let Some(repo_dao) = &self.repo_dao else {
            return None;
        };

        // Check if project already exists
        if let Ok(Some(existing_repo)) = repo_dao.get_by_path(&path) {
            // Project already exists, just return its ID (caller will expand/select it)
            return Some(existing_repo.id);
        }

        let name = self
            .add_repo_dialog_state
            .repo_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

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

    fn record_chat_scroll(&mut self, lines: usize) {
        if lines > 0 {
            self.metrics.record_scroll_event(lines);
        }
    }

    fn should_route_scroll_to_chat(&self) -> bool {
        self.input_mode != InputMode::ShowingHelp
            && !(self.input_mode == InputMode::PickingProject && self.project_picker_state.is_visible())
    }

    fn flush_scroll_deltas(&mut self, pending_up: &mut usize, pending_down: &mut usize) {
        if *pending_up == 0 && *pending_down == 0 {
            return;
        }

        if self.input_mode == InputMode::ShowingHelp {
            // Route scroll to help dialog
            if *pending_up > 0 {
                self.help_dialog_state.scroll_up(*pending_up);
            }
            if *pending_down > 0 {
                self.help_dialog_state.scroll_down(*pending_down);
            }
        } else if self.input_mode == InputMode::PickingProject && self.project_picker_state.is_visible() {
            for _ in 0..*pending_up {
                self.project_picker_state.select_prev();
            }
            for _ in 0..*pending_down {
                self.project_picker_state.select_next();
            }
        } else if let Some(session) = self.tab_manager.active_session_mut() {
            if *pending_up > 0 {
                session.chat_view.scroll_up(*pending_up);
            }
            if *pending_down > 0 {
                session.chat_view.scroll_down(*pending_down);
            }
        }

        *pending_up = 0;
        *pending_down = 0;
    }

    fn handle_mouse_event(&mut self, mouse: event::MouseEvent) -> Option<Action> {
        let x = mouse.column;
        let y = mouse.row;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                // Route scroll to appropriate component based on mode
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.scroll_up(3);
                } else if self.input_mode == InputMode::PickingProject
                    && self.project_picker_state.is_visible()
                {
                    self.project_picker_state.select_prev();
                } else if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_up(1);
                    self.record_chat_scroll(1);
                }
                None
            }
            MouseEventKind::ScrollDown => {
                // Route scroll to appropriate component based on mode
                if self.input_mode == InputMode::ShowingHelp {
                    self.help_dialog_state.scroll_down(3);
                } else if self.input_mode == InputMode::PickingProject
                    && self.project_picker_state.is_visible()
                {
                    self.project_picker_state.select_next();
                } else if let Some(session) = self.tab_manager.active_session_mut() {
                    session.chat_view.scroll_down(1);
                    self.record_chat_scroll(1);
                }
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Handle left clicks based on position
                self.handle_mouse_click(x, y)
            }
            _ => None,
        }
    }

    /// Handle a mouse click at the given position
    /// Returns an action to execute if the click triggered one
    fn handle_mouse_click(&mut self, x: u16, y: u16) -> Option<Action> {
        // Handle project picker clicks first (it's a modal dialog)
        if self.input_mode == InputMode::PickingProject
            && self.project_picker_state.is_visible()
        {
            self.handle_project_picker_click(x, y);
            return None;
        }

        // Check sidebar first (if visible)
        if let Some(sidebar_area) = self.sidebar_area {
            if Self::point_in_rect(x, y, sidebar_area) {
                self.handle_sidebar_click(x, y, sidebar_area);
                return None;
            }
        }

        // Check tab bar
        if let Some(tab_bar_area) = self.tab_bar_area {
            if Self::point_in_rect(x, y, tab_bar_area) {
                self.handle_tab_bar_click(x, y, tab_bar_area);
                return None;
            }
        }

        // Check input area
        if let Some(input_area) = self.input_area {
            if Self::point_in_rect(x, y, input_area) {
                self.handle_input_click(x, y, input_area);
                return None;
            }
        }

        // Check status bar
        if let Some(status_bar_area) = self.status_bar_area {
            if Self::point_in_rect(x, y, status_bar_area) {
                self.handle_status_bar_click(x, y, status_bar_area);
                return None;
            }
        }

        // Check footer
        if let Some(footer_area) = self.footer_area {
            if Self::point_in_rect(x, y, footer_area) {
                return self.handle_footer_click(x, y, footer_area);
            }
        }

        // Click in chat area - could be used for text selection in future
        // For now, clicking in chat area while in sidebar mode returns to normal
        if self.input_mode == InputMode::SidebarNavigation {
            self.input_mode = InputMode::Normal;
            self.sidebar_state.set_focused(false);
        }

        None
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

        // Always focus sidebar when clicking on it
        self.sidebar_state.set_focused(true);
        self.input_mode = InputMode::SidebarNavigation;

        let clicked_row = (y - inner_y) as usize;
        let clicked_index = clicked_row + self.sidebar_state.tree_state.offset;

        // Detect double-click (same index within 500ms)
        let now = Instant::now();
        let is_double_click = if let Some((last_time, last_index)) = self.last_sidebar_click {
            last_index == clicked_index && now.duration_since(last_time) < Duration::from_millis(500)
        } else {
            false
        };

        // Update last click tracking
        self.last_sidebar_click = Some((now, clicked_index));

        // Get the node at this index
        if let Some(node) = self.sidebar_data.get_at(clicked_index) {
            use crate::ui::components::{ActionType, NodeType};

            // Update selection
            self.sidebar_state.tree_state.selected = clicked_index;

            // Handle based on node type
            match node.node_type {
                NodeType::Repository => {
                    // Toggle expand/collapse
                    self.sidebar_data.toggle_at(clicked_index);
                }
                NodeType::Workspace => {
                    // Single click: open workspace but keep sidebar open
                    // Double click: open workspace and close sidebar
                    self.open_workspace_with_options(node.id, is_double_click);
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
        // Click on agent or model portion to open model selector
        let relative_x = x.saturating_sub(status_bar_area.x) as usize;

        // Approximate positions - agent type is ~6 chars, separator is 3, model starts around 9
        // Click on either agent tag (0-8) or model area (9-25) opens the model selector
        if relative_x < 25 {
            if let Some(session) = self.tab_manager.active_session() {
                self.model_selector_state.show(session.model.clone());
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
    /// Returns an action to execute if a valid hint was clicked
    fn handle_footer_click(&mut self, x: u16, _y: u16, footer_area: Rect) -> Option<Action> {
        // Use the same hints as GlobalFooter to stay in sync
        let hints: Vec<(&str, &str)> = match self.view_mode {
            ViewMode::Chat => vec![
                ("Tab", "Switch"),
                ("C-t", "Sidebar"),
                ("C-n", "Project"),
                ("C-S-w", "Close"),
                ("C-c", "Stop"),
                ("C-q", "Quit"),
            ],
            ViewMode::RawEvents => vec![
                ("j/k", "Nav"),
                ("l/CR", "Expand"),
                ("h/Esc", "Collapse"),
                ("C-g", "Chat"),
                ("C-q", "Quit"),
            ],
        };

        // Calculate click position relative to footer
        let relative_x = x.saturating_sub(footer_area.x) as usize;

        // Match the layout from GlobalFooter::render:
        // " [key] action   [key] action ..."
        // Leading space = 1, key has " key " (len+2), action has " action" (len+1), spacing = 3
        let mut current_x: usize = 1; // Leading space

        for (key, action_name) in hints {
            // Format: " key " (key.len + 2) + " action" (action_name.len + 1) + spacing (3)
            let key_width = key.len() + 2;
            let action_width = action_name.len() + 1;
            let hint_width = key_width + action_width + 3;

            if relative_x >= current_x && relative_x < current_x + hint_width {
                // Clicked on this hint - look up action from keybinding config
                return self.lookup_footer_action(key);
            }
            current_x += hint_width;
        }
        None
    }

    /// Look up the action for a footer key hint using the keybinding config
    fn lookup_footer_action(&self, key: &str) -> Option<Action> {
        // Handle compound keys like "j/k" by taking the first one
        let primary_key = key.split('/').next().unwrap_or(key);

        // Special case for "CR" which should be "<CR>"
        let key_notation = if primary_key == "CR" {
            "<CR>".to_string()
        } else {
            primary_key.to_string()
        };

        // Parse the key notation
        let key_combo = parse_key_notation(&key_notation).ok()?;

        // Determine context from current mode
        let context = KeyContext::from_input_mode(self.input_mode, self.view_mode);

        // Look up action in keybinding config
        self.config.keybindings.get_action(&key_combo, context).cloned()
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
                    let display = MessageDisplay::Error { content: msg };
                    session.chat_view.push(display.to_chat_message());
                    session.stop_processing();
                }
            }
            AppEvent::PrPreflightCompleted {
                tab_index,
                working_dir,
                result,
            } => {
                self.handle_pr_preflight_result(tab_index, working_dir, result);
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
        // Check if this is a non-active tab receiving content - mark as needing attention
        let is_active_tab = self.tab_manager.active_index() == tab_index;
        let is_content_event = matches!(
            &event,
            AgentEvent::AssistantMessage(_)
                | AgentEvent::ToolCompleted(_)
                | AgentEvent::TurnCompleted(_)
                | AgentEvent::TurnFailed(_)
        );

        let Some(session) = self.tab_manager.session_mut(tab_index) else {
            return Ok(());
        };

        // Mark non-active tabs as needing attention when content arrives
        if !is_active_tab && is_content_event {
            session.needs_attention = true;
        }

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
                let display = MessageDisplay::Error {
                    content: failed.error,
                };
                session.chat_view.push(display.to_chat_message());
            }
            AgentEvent::AssistantMessage(msg) => {
                // Track streaming tokens (rough estimate: ~4 chars per token)
                let token_estimate = (msg.text.len() / 4).max(1);
                session.add_streaming_tokens(token_estimate);

                if msg.is_final {
                    let display = MessageDisplay::Assistant {
                        content: msg.text,
                        is_streaming: false,
                    };
                    session.chat_view.push(display.to_chat_message());
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
                let display = MessageDisplay::Tool {
                    name: MessageDisplay::tool_display_name_owned(&tool.tool_name),
                    args: args_str,
                    output: "Running...".to_string(),
                    exit_code: None,
                };
                session.chat_view.push(display.to_chat_message());
            }
            AgentEvent::ToolCompleted(tool) => {
                // Return to thinking state
                session.set_processing_state(ProcessingState::Thinking);

                // Track file changes for write/edit tools
                if tool.success {
                    let tool_name_lower = tool.tool_id.to_lowercase();
                    if tool_name_lower.contains("edit")
                        || tool_name_lower.contains("write")
                        || tool_name_lower.contains("multiedit")
                    {
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

                let output = if tool.success {
                    tool.result.unwrap_or_else(|| "Completed".to_string())
                } else {
                    format!("Error: {}", tool.error.unwrap_or_default())
                };
                let display = MessageDisplay::Tool {
                    name: MessageDisplay::tool_display_name_owned(&tool.tool_id),
                    args: String::new(),
                    output,
                    exit_code: None,
                };
                session.chat_view.push(display.to_chat_message());
            }
            AgentEvent::CommandOutput(cmd) => {
                let display = MessageDisplay::Tool {
                    name: "Bash".to_string(),
                    args: cmd.command.clone(),
                    output: cmd.output.clone(),
                    exit_code: cmd.exit_code,
                };
                session.chat_view.push(display.to_chat_message());
            }
            AgentEvent::Error(err) => {
                let display = MessageDisplay::Error {
                    content: err.message,
                };
                session.chat_view.push(display.to_chat_message());
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
        let display = MessageDisplay::User {
            content: prompt.clone(),
        };
        session.chat_view.push(display.to_chat_message());
        session.start_processing();

        // Capture session state before releasing borrow
        let agent_type = session.agent_type;
        let model = session.model.clone();
        // Use agent_session_id if available (set by agent after first prompt)
        // Fall back to resume_session_id only for initial session restoration
        let session_id_to_use = session
            .agent_session_id
            .clone()
            .or_else(|| session.resume_session_id.take());
        // Use session's working_dir if set, otherwise fall back to config
        let working_dir = session
            .working_dir
            .clone()
            .unwrap_or_else(|| self.config.working_dir.clone());

        // Validate working directory exists
        if !working_dir.exists() {
            if let Some(session) = self.tab_manager.active_session_mut() {
                let display = MessageDisplay::Error {
                    content: format!(
                        "Working directory does not exist: {}",
                        working_dir.display()
                    ),
                };
                session.chat_view.push(display.to_chat_message());
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

        // Add session ID to continue existing conversation
        if let Some(session_id) = session_id_to_use {
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

    /// Handle Ctrl+P: Open existing PR or create new one
    fn handle_pr_action(&mut self) {
        let tab_index = self.tab_manager.active_index();
        let session = match self.tab_manager.active_session() {
            Some(s) => s,
            None => return, // No active tab
        };

        let working_dir = match &session.working_dir {
            Some(d) => d.clone(),
            None => return, // No working dir
        };

        // Show loading dialog immediately
        self.confirmation_dialog_state
            .show_loading("Create Pull Request", "Checking repository status...");
        self.input_mode = InputMode::Confirming;

        // Spawn preflight check in background
        let event_tx = self.event_tx.clone();
        let wd = working_dir.clone();
        tokio::spawn(async move {
            let result = PrManager::preflight_check(&wd);
            let _ = event_tx.send(AppEvent::PrPreflightCompleted {
                tab_index,
                working_dir: wd,
                result,
            });
        });
    }

    /// Handle the result of the PR preflight check
    fn handle_pr_preflight_result(
        &mut self,
        _tab_index: usize,
        working_dir: std::path::PathBuf,
        preflight: crate::git::PrPreflightResult,
    ) {
        // Handle blocking errors
        if !preflight.gh_installed {
            self.confirmation_dialog_state.hide();
            self.error_dialog_state.show_with_details(
                "GitHub CLI Not Found",
                "The 'gh' command is not installed.",
                "Install from: https://cli.github.com/\n\nbrew install gh  # macOS\napt install gh   # Debian/Ubuntu",
            );
            self.input_mode = InputMode::ShowingError;
            return;
        }

        if !preflight.gh_authenticated {
            self.confirmation_dialog_state.hide();
            self.error_dialog_state.show_with_details(
                "Not Authenticated",
                "GitHub CLI is not authenticated.",
                "Run: gh auth login",
            );
            self.input_mode = InputMode::ShowingError;
            return;
        }

        if preflight.on_main_branch {
            self.confirmation_dialog_state.hide();
            self.error_dialog_state.show(
                "Cannot Create PR",
                &format!(
                    "You're on the '{}' branch. Create a feature branch first.",
                    preflight.branch_name
                ),
            );
            self.input_mode = InputMode::ShowingError;
            return;
        }

        // If PR exists, just open it in browser
        if let Some(ref pr) = preflight.existing_pr {
            if pr.exists {
                self.confirmation_dialog_state.hide();
                self.input_mode = InputMode::Normal;
                if let Err(e) = PrManager::open_pr_in_browser(&working_dir) {
                    self.error_dialog_state.show(
                        "Failed to Open PR",
                        &format!("Could not open PR in browser: {}", e),
                    );
                    self.input_mode = InputMode::ShowingError;
                }
                return;
            }
        }

        // Build warnings for confirmation dialog
        let mut warnings = Vec::new();
        if preflight.uncommitted_count > 0 {
            warnings.push(format!(
                "{} file(s) will be auto-committed",
                preflight.uncommitted_count
            ));
        }
        if !preflight.has_upstream {
            warnings.push("Branch will be pushed to remote".to_string());
        }

        // Show confirmation dialog (replace loading state)
        self.confirmation_dialog_state.show(
            "Create Pull Request",
            format!(
                "Branch: {}\nTarget: {}",
                preflight.branch_name, preflight.target_branch
            ),
            warnings,
            ConfirmationType::Info,
            "Create PR",
            Some(ConfirmationContext::CreatePullRequest {
                tab_index: _tab_index,
                working_dir,
                preflight,
            }),
        );
        // Already in Confirming mode
    }

    /// Submit the PR workflow prompt to the current chat
    async fn submit_pr_workflow(
        &mut self,
        preflight: crate::git::PrPreflightResult,
    ) -> anyhow::Result<()> {
        // Generate prompt for PR creation
        let prompt = PrManager::generate_pr_prompt(&preflight);

        // Submit to current chat session
        self.submit_prompt(prompt).await
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

                // Draw tab bar (unfocused when sidebar is focused)
                let tabs_focused = self.input_mode != InputMode::SidebarNavigation;

                // Collect tab states for PR indicators
                let pr_numbers: Vec<Option<u32>> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.pr_number)
                    .collect();
                let processing_flags: Vec<bool> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.is_processing)
                    .collect();
                let attention_flags: Vec<bool> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.needs_attention)
                    .collect();

                let tab_bar = TabBar::new(
                    self.tab_manager.tab_names(),
                    self.tab_manager.active_index(),
                    self.tab_manager.can_add_tab(),
                )
                .focused(tabs_focused)
                .with_tab_states(pr_numbers, processing_flags, attention_flags)
                .with_spinner_frame(self.spinner_frame);
                tab_bar.render(chunks[0], f.buffer_mut());

                // Draw active session components
                let is_command_mode = self.input_mode == InputMode::Command;
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

                    // Render input box (not in command mode)
                    if !is_command_mode {
                        session.input_box.render(chunks[2], f.buffer_mut());
                    }
                    // Update status bar with performance metrics
                    session.status_bar.set_metrics(
                        self.show_metrics,
                        self.metrics.draw_time,
                        self.metrics.event_time,
                        self.metrics.fps,
                        self.metrics.scroll_latency,
                        self.metrics.scroll_latency_avg,
                        self.metrics.scroll_lines_per_sec,
                        self.metrics.scroll_events_per_sec,
                        self.metrics.scroll_active,
                    );
                    session.status_bar.render(chunks[3], f.buffer_mut());

                    // Set cursor position (accounting for scroll)
                    if self.input_mode == InputMode::Normal {
                        let scroll_offset = session.input_box.scroll_offset();
                        let (cx, cy) = session.input_box.cursor_position(chunks[2], scroll_offset);
                        f.set_cursor_position((cx, cy));
                    }
                }

                // Render command prompt if in command mode (outside session borrow)
                if is_command_mode {
                    self.render_command_prompt(chunks[2], f.buffer_mut());
                    // Cursor at end of command buffer (after ":" inside border)
                    // +1 for left border, +1 for colon, then buffer length
                    let cx = chunks[2].x + 2 + self.command_buffer.len() as u16;
                    let cy = chunks[2].y + 1; // +1 for top border
                    f.set_cursor_position((cx, cy));
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

                // Draw tab bar (unfocused when sidebar is focused)
                let tabs_focused = self.input_mode != InputMode::SidebarNavigation;
                // Collect tab states for PR indicators
                let pr_numbers: Vec<Option<u32>> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.pr_number)
                    .collect();
                let processing_flags: Vec<bool> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.is_processing)
                    .collect();
                let attention_flags: Vec<bool> = self
                    .tab_manager
                    .sessions()
                    .iter()
                    .map(|s| s.needs_attention)
                    .collect();
                let tab_bar = TabBar::new(
                    self.tab_manager.tab_names(),
                    self.tab_manager.active_index(),
                    self.tab_manager.can_add_tab(),
                )
                .focused(tabs_focused)
                .with_tab_states(pr_numbers, processing_flags, attention_flags)
                .with_spinner_frame(self.spinner_frame);
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

        // Draw error dialog (on top of everything except spinner)
        if self.error_dialog_state.visible {
            use ratatui::widgets::Widget;
            let dialog = ErrorDialog::new(&self.error_dialog_state);
            dialog.render(size, f.buffer_mut());
        }

        // Draw help dialog (on top of everything)
        if self.help_dialog_state.is_visible() {
            HelpDialog::new().render(size, f.buffer_mut(), &mut self.help_dialog_state);
        }

        // Draw removing project spinner overlay
        if self.input_mode == InputMode::RemovingProject {
            use crate::ui::components::Spinner;
            use ratatui::layout::Alignment;
            use ratatui::style::{Color, Style};
            use ratatui::text::Line;
            use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

            let dialog_width: u16 = 30;
            let dialog_height: u16 = 3;

            // Center the dialog
            let x = size.width.saturating_sub(dialog_width) / 2;
            let y = size.height.saturating_sub(dialog_height) / 2;

            let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

            // Clear the area first
            Clear.render(dialog_area, f.buffer_mut());

            // Render dialog box
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let inner = block.inner(dialog_area);
            block.render(dialog_area, f.buffer_mut());

            // Render spinner and message
            let spinner = Spinner::dots();
            let line = Line::from(vec![
                spinner.span(Color::Cyan),
                ratatui::text::Span::raw(" Removing project..."),
            ]);

            let para = Paragraph::new(line).alignment(Alignment::Center);
            para.render(inner, f.buffer_mut());
        }
    }

    /// Render command mode prompt
    fn render_command_prompt(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::style::{Color, Style};
        use ratatui::widgets::{Block, Borders, Paragraph, Widget};

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Command ");

        let inner = block.inner(area);
        block.render(area, buf);

        let prompt = format!(":{}", self.command_buffer);
        let para = Paragraph::new(prompt).style(Style::default().fg(Color::White));
        para.render(inner, buf);
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
    fn dump_debug_state(&mut self) {
        use chrono::Local;
        use serde_json::json;

        let timestamp = Local::now().format("%Y%m%d_%H%M%S");

        // Save to ~/.conduit/debug/ directory
        let debug_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".conduit")
            .join("debug");

        // Create directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&debug_dir) {
            self.error_dialog_state.show(
                "Export Failed",
                &format!("Could not create debug directory: {}", e),
            );
            self.input_mode = InputMode::ShowingError;
            return;
        }

        let filepath = debug_dir.join(format!("conduit_debug_{}.json", timestamp));

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

        // Write to file
        let full_path = filepath.display().to_string();
        match File::create(&filepath) {
            Ok(mut file) => {
                match serde_json::to_string_pretty(&dump) {
                    Ok(json_str) => {
                        if let Err(e) = file.write_all(json_str.as_bytes()) {
                            self.error_dialog_state.show(
                                "Export Failed",
                                &format!("Could not write to file: {}", e),
                            );
                            self.input_mode = InputMode::ShowingError;
                            return;
                        }
                    }
                    Err(e) => {
                        self.error_dialog_state.show(
                            "Export Failed",
                            &format!("Could not serialize debug data: {}", e),
                        );
                        self.input_mode = InputMode::ShowingError;
                        return;
                    }
                }
            }
            Err(e) => {
                self.error_dialog_state.show(
                    "Export Failed",
                    &format!("Could not create file: {}", e),
                );
                self.input_mode = InputMode::ShowingError;
                return;
            }
        }

        // Show success popup with full path
        self.error_dialog_state.show_with_details(
            "Debug Export Complete",
            "Session debug info has been exported.",
            &format!("File saved to:\n{}", full_path),
        );
        self.input_mode = InputMode::ShowingError;
    }
}
