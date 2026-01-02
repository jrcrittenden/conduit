//! Add repository dialog component

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use std::path::PathBuf;

/// State for the add repository dialog
#[derive(Debug, Clone)]
pub struct AddRepoDialogState {
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
    /// Extracted repository name
    pub repo_name: Option<String>,
}

impl Default for AddRepoDialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl AddRepoDialogState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            visible: false,
            error: None,
            is_valid: false,
            repo_name: None,
        }
    }

    /// Show the dialog
    pub fn show(&mut self) {
        self.visible = true;
        self.input.clear();
        self.cursor = 0;
        self.error = None;
        self.is_valid = false;
        self.repo_name = None;
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

    /// Validate the current input path
    pub fn validate(&mut self) {
        let path = PathBuf::from(&self.input);

        // Check if path is empty
        if self.input.is_empty() {
            self.error = None;
            self.is_valid = false;
            self.repo_name = None;
            return;
        }

        // Expand ~ to home directory
        let expanded_path = if self.input.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                home.join(&self.input[1..].trim_start_matches('/'))
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };

        // Check if path exists
        if !expanded_path.exists() {
            self.error = Some("Path does not exist".to_string());
            self.is_valid = false;
            self.repo_name = None;
            return;
        }

        // Check if it's a directory
        if !expanded_path.is_dir() {
            self.error = Some("Path is not a directory".to_string());
            self.is_valid = false;
            self.repo_name = None;
            return;
        }

        // Check for .git directory
        let git_dir = expanded_path.join(".git");
        if !git_dir.exists() {
            self.error = Some("Not a git repository (no .git directory)".to_string());
            self.is_valid = false;
            self.repo_name = None;
            return;
        }

        // Extract repository name from path
        self.repo_name = expanded_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        self.error = None;
        self.is_valid = true;
    }

    /// Get the expanded path
    pub fn expanded_path(&self) -> PathBuf {
        if self.input.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                return home.join(&self.input[1..].trim_start_matches('/'));
            }
        }
        PathBuf::from(&self.input)
    }

    /// Check if dialog is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Add repository dialog widget
pub struct AddRepoDialog;

impl AddRepoDialog {
    pub fn new() -> Self {
        Self
    }

    /// Render the dialog
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AddRepoDialogState) {
        if !state.visible {
            return;
        }

        // Calculate dialog size and position (centered)
        let dialog_width = 60.min(area.width.saturating_sub(4));
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
            .title(" Add Custom Project ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        block.render(dialog_area, buf);

        // Layout inside dialog
        let chunks = Layout::vertical([
            Constraint::Length(1), // Label
            Constraint::Length(1), // Spacing
            Constraint::Length(3), // Input field (with border)
            Constraint::Length(1), // Status/error
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Instructions
        ])
        .split(inner);

        // Render label
        let label = Paragraph::new("Enter local repository path:")
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

        // Render input text with cursor
        let display_text = if state.input.is_empty() {
            "~/path/to/repo".to_string()
        } else {
            state.input.clone()
        };

        let text_style = if state.input.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let input_paragraph = Paragraph::new(display_text).style(text_style);
        input_paragraph.render(input_inner, buf);

        // Render cursor
        if input_inner.width > 0 && state.cursor <= input_inner.width as usize {
            let cursor_x = input_inner.x + state.cursor as u16;
            if cursor_x < input_inner.x + input_inner.width {
                buf[(cursor_x, input_inner.y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }

        // Render status/error
        let status_text = if let Some(ref error) = state.error {
            Line::from(Span::styled(
                format!("✗ {}", error),
                Style::default().fg(Color::Red),
            ))
        } else if state.is_valid {
            let name = state.repo_name.as_deref().unwrap_or("repository");
            Line::from(Span::styled(
                format!("✓ Valid repository: {}", name),
                Style::default().fg(Color::Green),
            ))
        } else {
            Line::default()
        };

        let status = Paragraph::new(status_text);
        status.render(chunks[4], buf);

        // Render instructions
        let instructions = Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" to add  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" to cancel"),
        ]))
        .alignment(Alignment::Center);
        instructions.render(chunks[6], buf);
    }
}

impl Default for AddRepoDialog {
    fn default() -> Self {
        Self::new()
    }
}
