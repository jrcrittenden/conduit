use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

/// Tab bar component for switching between sessions
pub struct TabBar {
    tabs: Vec<String>,
    active: usize,
    can_add: bool,
    focused: bool,
}

impl TabBar {
    pub fn new(tabs: Vec<String>, active: usize, can_add: bool) -> Self {
        Self {
            tabs,
            active,
            can_add,
            focused: true,
        }
    }

    /// Set whether the tab bar is focused
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_active = i == self.active;

            // Tab indicator - only show when focused
            if is_active && self.focused {
                spans.push(Span::styled(
                    " ▶ ",
                    Style::default().fg(Color::Cyan),
                ));
            } else {
                spans.push(Span::raw("   "));
            }

            // Tab name - dim the active tab when not focused
            let tab_style = if is_active {
                if self.focused {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    // Active but unfocused - slightly brighter than inactive
                    Style::default().fg(Color::Gray)
                }
            } else {
                Style::default().fg(Color::DarkGray)
            };

            spans.push(Span::styled(format!("[{}] {}", i + 1, tab), tab_style));

            spans.push(Span::raw("  "));
        }

        // Add new tab button
        if self.can_add {
            spans.push(Span::styled(
                " [+] New ",
                Style::default().fg(Color::Green),
            ));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line)
            .style(Style::default().bg(Color::Rgb(20, 20, 20)));

        paragraph.render(area, buf);
    }
}
