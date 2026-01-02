//! Reusable text input state with cursor management

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Paragraph, Widget},
};

/// Reusable text input state with cursor management
#[derive(Debug, Clone, Default)]
pub struct TextInputState {
    /// Current input text
    pub input: String,
    /// Cursor position in the input
    pub cursor: usize,
}

impl TextInputState {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
        }
    }

    /// Create with initial value
    pub fn with_value(value: &str) -> Self {
        Self {
            input: value.to_string(),
            cursor: value.len(),
        }
    }

    /// Set the input value and move cursor to end
    pub fn set(&mut self, value: &str) {
        self.input = value.to_string();
        self.cursor = self.input.len();
    }

    /// Clear the input
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    /// Get the current value
    pub fn value(&self) -> &str {
        &self.input
    }

    /// Check if input is empty
    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += 1;
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    /// Delete character at cursor (delete)
    pub fn delete_forward(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
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

    /// Delete from cursor to start of line (Ctrl+U)
    pub fn delete_to_start(&mut self) {
        self.input = self.input[self.cursor..].to_string();
        self.cursor = 0;
    }

    /// Delete from cursor to end of line (Ctrl+K)
    pub fn delete_to_end(&mut self) {
        self.input.truncate(self.cursor);
    }

    /// Move cursor to previous word boundary (Alt+B)
    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Skip any spaces before cursor
        while self.cursor > 0 && self.input.chars().nth(self.cursor - 1) == Some(' ') {
            self.cursor -= 1;
        }
        // Move to start of word
        while self.cursor > 0 && self.input.chars().nth(self.cursor - 1) != Some(' ') {
            self.cursor -= 1;
        }
    }

    /// Move cursor to next word boundary (Alt+F)
    pub fn move_word_right(&mut self) {
        let len = self.input.len();
        if self.cursor >= len {
            return;
        }
        // Skip current word
        while self.cursor < len && self.input.chars().nth(self.cursor) != Some(' ') {
            self.cursor += 1;
        }
        // Skip spaces
        while self.cursor < len && self.input.chars().nth(self.cursor) == Some(' ') {
            self.cursor += 1;
        }
    }

    /// Delete word before cursor (Ctrl+W)
    pub fn delete_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.cursor;
        // Skip spaces
        while self.cursor > 0 && self.input.chars().nth(self.cursor - 1) == Some(' ') {
            self.cursor -= 1;
        }
        // Delete word
        while self.cursor > 0 && self.input.chars().nth(self.cursor - 1) != Some(' ') {
            self.cursor -= 1;
        }
        self.input.drain(self.cursor..end);
    }

    /// Render the text with cursor at given area
    pub fn render(&self, area: Rect, buf: &mut Buffer, style: Style) {
        let text = Paragraph::new(self.input.as_str()).style(style);
        text.render(area, buf);

        // Render cursor
        if area.width > 0 {
            let cursor_x = area.x + (self.cursor as u16).min(area.width.saturating_sub(1));
            if cursor_x < area.x + area.width {
                buf[(cursor_x, area.y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }
    }

    /// Render with placeholder text when empty
    pub fn render_with_placeholder(
        &self,
        area: Rect,
        buf: &mut Buffer,
        style: Style,
        placeholder: &str,
        placeholder_style: Style,
    ) {
        if self.input.is_empty() {
            let text = Paragraph::new(placeholder).style(placeholder_style);
            text.render(area, buf);
        } else {
            let text = Paragraph::new(self.input.as_str()).style(style);
            text.render(area, buf);
        }

        // Render cursor
        if area.width > 0 {
            let cursor_x = area.x + (self.cursor as u16).min(area.width.saturating_sub(1));
            if cursor_x < area.x + area.width {
                buf[(cursor_x, area.y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

impl std::fmt::Display for TextInputState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.input)
    }
}
