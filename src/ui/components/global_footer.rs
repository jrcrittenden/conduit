use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::ui::events::ViewMode;

/// Global footer showing keyboard shortcuts
pub struct GlobalFooter {
    hints: Vec<(&'static str, &'static str)>,
    view_mode: ViewMode,
}

impl GlobalFooter {
    pub fn new() -> Self {
        Self {
            hints: vec![
                ("Tab", "Switch"),
                ("Ctrl+N", "New"),
                ("Ctrl+W", "Close"),
                ("Ctrl+C", "Interrupt"),
                ("^G", "Debug"),
                ("Ctrl+Q", "Quit"),
            ],
            view_mode: ViewMode::Chat,
        }
    }

    pub fn with_view_mode(mut self, view_mode: ViewMode) -> Self {
        self.view_mode = view_mode;
        // Update hints based on view mode
        self.hints = match view_mode {
            ViewMode::Chat => vec![
                ("Tab", "Switch"),
                ("Ctrl+N", "New"),
                ("Ctrl+W", "Close"),
                ("Ctrl+C", "Interrupt"),
                ("^G", "Debug"),
                ("Ctrl+Q", "Quit"),
            ],
            ViewMode::RawEvents => vec![
                ("↑/↓", "Navigate"),
                ("Enter", "Expand"),
                ("Esc", "Collapse"),
                ("^G", "Chat"),
                ("Ctrl+Q", "Quit"),
            ],
        };
        self
    }

    pub fn with_hints(hints: Vec<(&'static str, &'static str)>) -> Self {
        Self {
            hints,
            view_mode: ViewMode::Chat,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();

        for (i, (key, action)) in self.hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" │ "));
            }

            spans.push(Span::styled(
                format!("[{}]", key),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(*action, Style::default().fg(Color::DarkGray)));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line)
            .style(Style::default().bg(Color::Rgb(15, 15, 15)));

        paragraph.render(area, buf);
    }
}

impl Default for GlobalFooter {
    fn default() -> Self {
        Self::new()
    }
}
