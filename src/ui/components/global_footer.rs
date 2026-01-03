use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::ui::events::ViewMode;

/// Global footer showing keyboard shortcuts in neovim style
pub struct GlobalFooter {
    hints: Vec<(&'static str, &'static str)>,
    view_mode: ViewMode,
}

impl GlobalFooter {
    pub fn new() -> Self {
        Self {
            hints: vec![
                ("Tab", "Switch"),
                ("C-t", "Sidebar"),
                ("C-n", "Project"),
                ("C-w", "Close"),
                ("C-c", "Stop"),
                ("C-q", "Quit"),
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
                ("C-t", "Sidebar"),
                ("C-n", "Project"),
                ("C-w", "Close"),
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

        // Leading space
        spans.push(Span::raw(" "));

        for (i, (key, action)) in self.hints.iter().enumerate() {
            if i > 0 {
                // Spacing between items
                spans.push(Span::raw("   "));
            }

            // Key with subtle background highlight
            spans.push(Span::styled(
                format!(" {} ", key),
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(60, 60, 60)),
            ));

            // Action text
            spans.push(Span::styled(
                format!(" {}", action),
                Style::default().fg(Color::Rgb(120, 120, 120)),
            ));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line)
            .style(Style::default().bg(Color::Rgb(25, 25, 25)));

        paragraph.render(area, buf);
    }
}

impl Default for GlobalFooter {
    fn default() -> Self {
        Self::new()
    }
}
