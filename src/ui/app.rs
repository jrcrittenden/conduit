use std::io;
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
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use tokio::sync::mpsc;

use crate::agent::{
    AgentEvent, AgentRunner, AgentStartConfig, AgentType, ClaudeCodeRunner, CodexCliRunner,
};
use crate::config::Config;
use crate::ui::components::{ChatMessage, GlobalFooter, TabBar};
use crate::ui::events::{AppEvent, InputMode};
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
    /// Agent runners
    claude_runner: Arc<ClaudeCodeRunner>,
    codex_runner: Arc<CodexCliRunner>,
    /// Event channel sender
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Event channel receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    /// Tick counter for spinner animation
    tick_count: u32,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let mut app = Self {
            config: config.clone(),
            should_quit: false,
            tab_manager: TabManager::new(config.max_tabs),
            input_mode: InputMode::Normal,
            claude_runner: Arc::new(ClaudeCodeRunner::new()),
            codex_runner: Arc::new(CodexCliRunner::new()),
            event_tx,
            event_rx,
            tick_count: 0,
        };

        // Create initial tab
        app.tab_manager.new_tab(config.default_agent);

        app
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

                    // Tick spinner animation (every 6 frames = ~100ms)
                    self.tick_count += 1;
                    if self.tick_count % 6 == 0 {
                        if let Some(session) = self.tab_manager.active_session_mut() {
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
        // Global shortcuts (work in any mode)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('n') => {
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
                    if self.tab_manager.is_empty() {
                        self.should_quit = true;
                    }
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
                    // Ctrl+B: Move cursor back (same as Left)
                    if let Some(session) = self.tab_manager.active_session_mut() {
                        session.input_box.move_left();
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
                    KeyCode::Char('1') | KeyCode::Char('c') => {
                        self.tab_manager.new_tab(AgentType::Claude);
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Char('2') | KeyCode::Char('x') => {
                        self.tab_manager.new_tab(AgentType::Codex);
                        self.input_mode = InputMode::Normal;
                    }
                    KeyCode::Esc => {
                        self.input_mode = InputMode::Normal;
                    }
                    _ => {}
                }
            }
            InputMode::Normal => {
                if let Some(session) = self.tab_manager.active_session_mut() {
                    match key.code {
                        KeyCode::Enter => {
                            if key.modifiers.contains(KeyModifiers::SHIFT)
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
                            // Try to move up in multi-line input
                            // If can't move (single line or at top), try history
                            if !session.input_box.move_up() {
                                if session.input_box.is_cursor_on_first_line() {
                                    session.input_box.history_prev();
                                }
                            }
                        }
                        KeyCode::Down => {
                            // Try to move down in multi-line input
                            // If can't move (single line or at bottom), try history
                            if !session.input_box.move_down() {
                                if session.input_box.is_cursor_on_last_line() {
                                    session.input_box.history_next();
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
                            if session.input_box.is_empty() {
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
                            session.chat_view.scroll_to_bottom();
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
        }

        Ok(())
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
                session.is_processing = false;
                session.add_usage(completed.usage);
                session.chat_view.finalize_streaming();
            }
            AgentEvent::TurnFailed(failed) => {
                session.is_processing = false;
                session.update_status();
                session.chat_view.push(ChatMessage::error(failed.error));
            }
            AgentEvent::AssistantMessage(msg) => {
                if msg.is_final {
                    session.chat_view.push(ChatMessage::assistant(msg.text));
                } else {
                    session.chat_view.stream_append(&msg.text);
                }
            }
            AgentEvent::ToolStarted(tool) => {
                let args_str = if tool.arguments.is_null() {
                    String::new()
                } else {
                    serde_json::to_string_pretty(&tool.arguments).unwrap_or_default()
                };
                session.chat_view.push(ChatMessage::tool(
                    &tool.tool_name,
                    format!("Started: {}", args_str),
                ));
            }
            AgentEvent::ToolCompleted(tool) => {
                let content = if tool.success {
                    tool.result.unwrap_or_else(|| "Completed".to_string())
                } else {
                    format!("Error: {}", tool.error.unwrap_or_default())
                };
                session
                    .chat_view
                    .push(ChatMessage::tool(&tool.tool_id, content));
            }
            AgentEvent::CommandOutput(cmd) => {
                session.chat_view.push(ChatMessage::tool(
                    "Bash",
                    format!(
                        "$ {}\n{}{}",
                        cmd.command,
                        cmd.output,
                        cmd.exit_code
                            .map(|c| format!("\n[exit: {}]", c))
                            .unwrap_or_default()
                    ),
                ));
            }
            AgentEvent::Error(err) => {
                session.chat_view.push(ChatMessage::error(err.message));
                if err.is_fatal {
                    session.is_processing = false;
                    session.update_status();
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

        // Add user message to chat
        session.chat_view.push(ChatMessage::user(&prompt));
        session.is_processing = true;
        session.update_status();

        // Start agent
        let config = AgentStartConfig::new(prompt, self.config.working_dir.clone())
            .with_tools(self.config.claude_allowed_tools.clone());

        let runner: Arc<dyn AgentRunner> = match session.agent_type {
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

        // Calculate dynamic input height (max 30% of screen)
        let max_input_height = (size.height as f32 * 0.30).ceil() as u16;
        let input_height = if let Some(session) = self.tab_manager.active_session() {
            session.input_box.desired_height(max_input_height)
        } else {
            3 // Minimum height
        };

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),            // Tab bar
                Constraint::Min(5),               // Chat view
                Constraint::Length(input_height), // Input box (dynamic)
                Constraint::Length(1),            // Status bar
                Constraint::Length(1),            // Footer
            ])
            .split(size);

        // Draw tab bar
        let tab_bar = TabBar::new(
            self.tab_manager.tab_names(),
            self.tab_manager.active_index(),
            self.tab_manager.can_add_tab(),
        );
        tab_bar.render(chunks[0], f.buffer_mut());

        // Draw active session components
        if let Some(session) = self.tab_manager.active_session_mut() {
            session.chat_view.render(chunks[1], f.buffer_mut());
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
        let footer = GlobalFooter::new();
        footer.render(chunks[4], f.buffer_mut());

        // Draw agent selector dialog if needed
        if self.input_mode == InputMode::SelectingAgent {
            self.draw_agent_selector(f, size);
        }
    }

    fn draw_agent_selector(&self, f: &mut Frame, area: Rect) {
        let width = 40;
        let height = 8;
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;

        let dialog_area = Rect::new(x, y, width, height);

        // Clear background
        f.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(" Select Agent ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        f.render_widget(block, dialog_area);

        let text = vec![
            "",
            "  [1] Claude Code",
            "  [2] Codex CLI",
            "",
            "  [Esc] Cancel",
        ];

        let paragraph = Paragraph::new(text.join("\n"))
            .style(Style::default().fg(Color::White));

        f.render_widget(paragraph, inner);
    }
}
