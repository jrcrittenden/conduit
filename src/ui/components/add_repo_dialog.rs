//! Add repository dialog component

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};
use std::path::PathBuf;

use super::{DialogFrame, InstructionBar, TextInputState};

/// State for the add repository dialog
#[derive(Debug, Clone)]
pub struct AddRepoDialogState {
    /// Text input state
    pub text: TextInputState,
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
            text: TextInputState::new(),
            visible: false,
            error: None,
            is_valid: false,
            repo_name: None,
        }
    }

    /// Show the dialog
    pub fn show(&mut self) {
        self.visible = true;
        self.text.clear();
        self.error = None;
        self.is_valid = false;
        self.repo_name = None;
    }

    /// Hide the dialog
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Get the current input value
    pub fn input(&self) -> &str {
        self.text.value()
    }

    // Delegate text input methods with validation
    pub fn insert_char(&mut self, c: char) {
        self.text.insert_char(c);
        self.validate();
    }

    pub fn delete_char(&mut self) {
        self.text.delete_char();
        self.validate();
    }

    pub fn delete_forward(&mut self) {
        self.text.delete_forward();
        self.validate();
    }

    pub fn move_left(&mut self) {
        self.text.move_left();
    }

    pub fn move_right(&mut self) {
        self.text.move_right();
    }

    pub fn move_start(&mut self) {
        self.text.move_start();
    }

    pub fn move_end(&mut self) {
        self.text.move_end();
    }

    /// Validate the current input path
    pub fn validate(&mut self) {
        let input = self.text.value();

        // Check if path is empty
        if input.is_empty() {
            self.error = None;
            self.is_valid = false;
            self.repo_name = None;
            return;
        }

        // Expand ~ to home directory
        let expanded_path = self.expanded_path();

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
        let input = self.text.value();
        if input.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                return home.join(input[1..].trim_start_matches('/'));
            }
        }
        PathBuf::from(input)
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

        // Render dialog frame
        let frame = DialogFrame::new("Add Custom Project", 60, 11);
        let inner = frame.render(area, buf);

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

        // Render input text with cursor and placeholder
        state.text.render_with_placeholder(
            input_inner,
            buf,
            Style::default().fg(Color::White),
            "~/path/to/repo",
            Style::default().fg(Color::DarkGray),
        );

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

        Paragraph::new(status_text).render(chunks[3], buf);

        // Render instructions
        let instructions = InstructionBar::new(vec![("Enter", "add"), ("Esc", "cancel")]);
        instructions.render(chunks[5], buf);
    }
}

impl Default for AddRepoDialog {
    fn default() -> Self {
        Self::new()
    }
}
