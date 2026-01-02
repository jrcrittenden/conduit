//! Set base projects directory dialog component

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::path::PathBuf;

/// State for the base directory dialog
#[derive(Debug, Clone)]
pub struct BaseDirDialogState {
    /// Current input text (path)
    pub input: String,
    /// Cursor position in the input
    pub cursor: usize,
    /// Whether the dialog is visible
    pub visible: bool,
    /// Validation error message
    pub error: Option<String>,
    /// Whether the path is valid
    pub is_valid: bool,
}

impl Default for BaseDirDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseDirDialogState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            visible: false,
            error: None,
            is_valid: false,
        }
    }

    /// Show the dialog with optional initial value
    pub fn show(&mut self) {
        self.visible = true;
        // Default to ~/code if empty
        if self.input.is_empty() {
            self.input = "~/code".to_string();
            self.cursor = self.input.len();
        }
        self.validate();
    }

    /// Show with a specific initial path
    pub fn show_with_path(&mut self, path: &str) {
        self.visible = true;
        self.input = path.to_string();
        self.cursor = self.input.len();
        self.validate();
    }

    /// Hide the dialog
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += 1;
        self.validate();
    }

    /// Delete character before cursor
    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
            self.validate();
        }
    }

    /// Delete character at cursor
    pub fn delete_forward(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
            self.validate();
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn move_start(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_end(&mut self) {
        self.cursor = self.input.len();
    }

    /// Delete from cursor to start of line
    pub fn delete_to_start(&mut self) {
        self.input = self.input[self.cursor..].to_string();
        self.cursor = 0;
        self.validate();
    }

    /// Delete from cursor to end of line
    pub fn delete_to_end(&mut self) {
        self.input.truncate(self.cursor);
        self.validate();
    }

    /// Validate the current input path
    pub fn validate(&mut self) {
        // Check if path is empty
        if self.input.is_empty() {
            self.error = Some("Path cannot be empty".to_string());
            self.is_valid = false;
            return;
        }

        let expanded_path = self.expanded_path();

        // Check if path exists
        if !expanded_path.exists() {
            self.error = Some("Directory does not exist".to_string());
            self.is_valid = false;
            return;
        }

        // Check if it's a directory
        if !expanded_path.is_dir() {
            self.error = Some("Path is not a directory".to_string());
            self.is_valid = false;
            return;
        }

        self.error = None;
        self.is_valid = true;
    }

    /// Get the expanded path
    pub fn expanded_path(&self) -> PathBuf {
        if self.input.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                return home.join(self.input[1..].trim_start_matches('/'));
            }
        }
        PathBuf::from(&self.input)
    }

    /// Check if dialog is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Base directory dialog widget
pub struct BaseDirDialog;

impl BaseDirDialog {
    pub fn new() -> Self {
        Self
    }

    /// Render the dialog
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &BaseDirDialogState) {
        if !state.visible {
            return;
        }

        // Calculate dialog size and position (centered)
        let dialog_width = 56.min(area.width.saturating_sub(4));
        let dialog_height = 11;

        let x = (area.width.saturating_sub(dialog_width)) / 2;
        let y = (area.height.saturating_sub(dialog_height)) / 2;

        let dialog_area = Rect {
            x,
            y,
            width: dialog_width,
            height: dialog_height,
        };

        // Clear the dialog area
        Clear.render(dialog_area, buf);

        // Render dialog border
        let block = Block::default()
            .title(" Set Projects Directory ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        block.render(dialog_area, buf);

        // Layout inside dialog
        let chunks = Layout::vertical([
            Constraint::Length(1), // Label
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Status/error
            Constraint::Length(1), // Help text
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Instructions
        ])
        .split(inner);

        // Render label
        let label = Paragraph::new("Where do you keep your projects?")
            .style(Style::default().fg(Color::White));
        label.render(chunks[0], buf);

        // Render input field
        let input_style = if state.is_valid {
            Style::default().fg(Color::Green)
        } else if state.error.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::White)
        };

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(input_style);

        let input_inner = input_block.inner(chunks[2]);
        input_block.render(chunks[2], buf);

        // Render input text
        let input_paragraph = Paragraph::new(state.input.as_str())
            .style(Style::default().fg(Color::White));
        input_paragraph.render(input_inner, buf);

        // Render cursor
        if input_inner.width > 0 {
            let cursor_x = input_inner.x + (state.cursor as u16).min(input_inner.width - 1);
            if cursor_x < input_inner.x + input_inner.width {
                buf[(cursor_x, input_inner.y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }

        // Render status/error
        let status_text = if let Some(ref error) = state.error {
            Line::from(Span::styled(
                format!("  {}", error),
                Style::default().fg(Color::Red),
            ))
        } else if state.is_valid {
            Line::from(Span::styled(
                "  Directory found",
                Style::default().fg(Color::Green),
            ))
        } else {
            Line::default()
        };

        let status = Paragraph::new(status_text);
        status.render(chunks[3], buf);

        // Help text
        let help = Paragraph::new("This directory will be scanned for git projects.")
            .style(Style::default().fg(Color::DarkGray));
        help.render(chunks[4], buf);

        // Render instructions
        let instructions = Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" to confirm    "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to cancel"),
        ]))
        .alignment(Alignment::Center);
        instructions.render(chunks[6], buf);
    }
}

impl Default for BaseDirDialog {
    fn default() -> Self {
        Self::new()
    }
}
