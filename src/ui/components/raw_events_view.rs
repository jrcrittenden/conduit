use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};
use serde_json::Value;

/// Direction of the event (sent to agent or received from agent)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDirection {
    Sent,
    Received,
}

impl EventDirection {
    pub fn symbol(&self) -> &'static str {
        match self {
            EventDirection::Sent => "→",
            EventDirection::Received => "←",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            EventDirection::Sent => Color::Green,
            EventDirection::Received => Color::Cyan,
        }
    }
}

/// A single raw event entry
#[derive(Debug, Clone)]
pub struct RawEventEntry {
    /// When the event occurred (relative to session start)
    pub timestamp: Instant,
    /// Direction of the event
    pub direction: EventDirection,
    /// Event type name
    pub event_type: String,
    /// Raw JSON value
    pub raw_json: Value,
    /// Session start time (for relative timestamp display)
    pub session_start: Instant,
}

impl RawEventEntry {
    pub fn new(
        direction: EventDirection,
        event_type: impl Into<String>,
        raw_json: Value,
        session_start: Instant,
    ) -> Self {
        Self {
            timestamp: Instant::now(),
            direction,
            event_type: event_type.into(),
            raw_json,
            session_start,
        }
    }

    /// Format timestamp as MM:SS.mmm relative to session start
    fn format_timestamp(&self) -> String {
        let elapsed = self.timestamp.duration_since(self.session_start);
        let total_secs = elapsed.as_secs();
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        let millis = elapsed.subsec_millis();
        format!("{:02}:{:02}.{:03}", mins, secs, millis)
    }

    /// Render as compact single line with optional prefix
    fn render_compact(&self, prefix: &str, style: Style) -> Line<'static> {
        let timestamp = self.format_timestamp();
        let summary = self.compact_summary();

        Line::from(vec![
            Span::styled(prefix.to_string(), Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("[{}] ", timestamp),
                style.fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{} ", self.direction.symbol()),
                style.fg(self.direction.color()),
            ),
            Span::styled(
                format!("{}: ", self.event_type),
                style
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(summary, style.fg(Color::White)),
        ])
    }

    /// Generate a compact one-line summary of the event
    fn compact_summary(&self) -> String {
        match &self.raw_json {
            Value::Object(map) => {
                // Try to extract meaningful fields
                let mut parts = Vec::new();

                // Common fields to show
                for key in &["prompt", "text", "message", "content", "error", "result", "command", "file_path", "tool_name", "session_id"] {
                    if let Some(val) = map.get(*key) {
                        let val_str = match val {
                            Value::String(s) => {
                                if s.len() > 50 {
                                    format!("\"{}...\"", &s[..47])
                                } else {
                                    format!("\"{}\"", s)
                                }
                            }
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        parts.push(format!("{}={}", key, val_str));
                        if parts.len() >= 3 {
                            break;
                        }
                    }
                }

                if parts.is_empty() {
                    // Fallback: show first few keys
                    let keys: Vec<_> = map.keys().take(3).map(|k| k.as_str()).collect();
                    format!("{{{}}}", keys.join(", "))
                } else {
                    parts.join(", ")
                }
            }
            Value::String(s) => {
                if s.len() > 60 {
                    format!("\"{}...\"", &s[..57])
                } else {
                    format!("\"{}\"", s)
                }
            }
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Array(arr) => format!("[{} items]", arr.len()),
        }
    }

    /// Render JSON lines with syntax highlighting (for expanded view)
    fn render_json_lines(&self, max_lines: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Ok(pretty) = serde_json::to_string_pretty(&self.raw_json) {
            let json_lines: Vec<&str> = pretty.lines().collect();
            let truncated = json_lines.len() > max_lines;
            let display_lines = if truncated {
                &json_lines[..max_lines.saturating_sub(1)]
            } else {
                &json_lines[..]
            };

            for json_line in display_lines {
                let mut highlighted = vec![Span::raw("  ")]; // Indent
                highlighted.extend(Self::highlight_json_line(json_line));
                lines.push(Line::from(highlighted));
            }

            if truncated {
                lines.push(Line::from(Span::styled(
                    format!("  ... ({} more lines)", json_lines.len() - max_lines + 1),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }

        lines
    }

    /// Apply syntax highlighting to a JSON line
    fn highlight_json_line(line: &str) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut chars = line.chars().peekable();
        let mut current = String::new();
        let mut in_string = false;
        let mut is_key = false;

        // Count leading whitespace for indentation (using peek to not consume first non-whitespace)
        let mut indent = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() {
                indent.push(ch);
                chars.next();
            } else {
                break;
            }
        }
        if !indent.is_empty() {
            spans.push(Span::raw(indent));
        }

        for ch in chars {
            match ch {
                '"' => {
                    if in_string {
                        current.push(ch);
                        let color = if is_key { Color::Cyan } else { Color::Green };
                        spans.push(Span::styled(current.clone(), Style::default().fg(color)));
                        current.clear();
                        in_string = false;
                        is_key = false;
                    } else {
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        in_string = true;
                        is_key = true;
                        current.push(ch);
                    }
                }
                ':' if !in_string => {
                    if !current.is_empty() {
                        spans.push(Span::raw(current.clone()));
                        current.clear();
                    }
                    spans.push(Span::styled(":", Style::default().fg(Color::White)));
                }
                '{' | '}' | '[' | ']' | ',' if !in_string => {
                    if !current.is_empty() {
                        let style = Self::get_value_style(&current);
                        spans.push(Span::styled(current.clone(), style));
                        current.clear();
                    }
                    spans.push(Span::styled(
                        ch.to_string(),
                        Style::default().fg(Color::White),
                    ));
                }
                _ => {
                    current.push(ch);
                }
            }
        }

        if !current.is_empty() {
            let style = Self::get_value_style(&current);
            spans.push(Span::styled(current, style));
        }

        spans
    }

    /// Get style for a JSON value based on its type
    fn get_value_style(value: &str) -> Style {
        let trimmed = value.trim();
        if trimmed == "true" || trimmed == "false" {
            Style::default().fg(Color::Magenta)
        } else if trimmed == "null" {
            Style::default().fg(Color::DarkGray)
        } else if trimmed.parse::<f64>().is_ok() {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        }
    }
}

/// View for displaying raw agent events with interactive selection
pub struct RawEventsView {
    /// All recorded events
    events: Vec<RawEventEntry>,
    /// Currently selected event index
    selected_index: usize,
    /// Whether the selected event is expanded to show JSON
    expanded: bool,
    /// Scroll offset for viewport (in lines)
    scroll_offset: usize,
    /// Session start time
    session_start: Instant,
}

impl RawEventsView {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            selected_index: 0,
            expanded: false,
            scroll_offset: 0,
            session_start: Instant::now(),
        }
    }

    /// Add a new event and select it
    pub fn push_event(&mut self, direction: EventDirection, event_type: impl Into<String>, raw_json: Value) {
        self.events.push(RawEventEntry::new(
            direction,
            event_type,
            raw_json,
            self.session_start,
        ));
        // Auto-select new event and collapse any expansion
        self.selected_index = self.events.len().saturating_sub(1);
        self.expanded = false;
        self.scroll_offset = 0; // Reset scroll to show latest
    }

    /// Move selection to previous event
    pub fn select_prev(&mut self) {
        if self.events.is_empty() {
            return;
        }
        // Collapse when navigating
        self.expanded = false;
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    /// Move selection to next event
    pub fn select_next(&mut self) {
        if self.events.is_empty() {
            return;
        }
        // Collapse when navigating
        self.expanded = false;
        self.selected_index = (self.selected_index + 1).min(self.events.len().saturating_sub(1));
    }

    /// Toggle expand/collapse of selected event
    pub fn toggle_expand(&mut self) {
        if !self.events.is_empty() {
            self.expanded = !self.expanded;
        }
    }

    /// Collapse the expanded event
    pub fn collapse(&mut self) {
        self.expanded = false;
    }

    /// Check if currently expanded
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Get event count
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get all events (for debug dump)
    pub fn events(&self) -> &[RawEventEntry] {
        &self.events
    }

    /// Clear all events
    pub fn clear(&mut self) {
        self.events.clear();
        self.selected_index = 0;
        self.expanded = false;
        self.scroll_offset = 0;
        self.session_start = Instant::now();
    }

    /// Build all lines for rendering, tracking which line range corresponds to selected item
    fn build_lines(&self) -> (Vec<Line<'static>>, usize, usize) {
        let mut lines = Vec::new();
        let mut selected_start = 0;
        let mut selected_end = 0;

        if self.events.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No events recorded yet",
                Style::default().fg(Color::DarkGray),
            )));
            return (lines, 0, 0);
        }

        for (i, event) in self.events.iter().enumerate() {
            let is_selected = i == self.selected_index;

            if is_selected {
                selected_start = lines.len();
            }

            if is_selected && self.expanded {
                // Expanded view: header with ▼ prefix, then JSON
                let header = event.render_compact(
                    "▼",
                    Style::default().bg(Color::Rgb(40, 40, 50)),
                );
                lines.push(header);

                // Add JSON lines (limit to 20 lines)
                let json_lines = event.render_json_lines(20);
                lines.extend(json_lines);
            } else if is_selected {
                // Selected but collapsed: ▶ prefix with highlight
                let line = event.render_compact(
                    "▶",
                    Style::default().bg(Color::Rgb(40, 40, 50)),
                );
                lines.push(line);
            } else {
                // Normal: space prefix
                let line = event.render_compact(" ", Style::default());
                lines.push(line);
            }

            if is_selected {
                selected_end = lines.len();
            }
        }

        (lines, selected_start, selected_end)
    }

    /// Render the raw events view
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let title = format!(" Raw Events ({}) ", self.events.len());

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(title);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 3 || inner.height < 1 {
            return;
        }

        let (lines, selected_start, selected_end) = self.build_lines();
        let total_lines = lines.len();
        let visible_height = inner.height as usize;

        // Ensure selected item is visible
        self.ensure_selection_visible(selected_start, selected_end, visible_height, total_lines);

        // Calculate which lines to show
        let start_line = self.scroll_offset;
        let end_line = (self.scroll_offset + visible_height).min(total_lines);

        let visible_lines: Vec<Line<'static>> = lines[start_line..end_line].to_vec();

        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
        paragraph.render(inner, buf);

        // Render scrollbar if content overflows
        if total_lines > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            let max_scroll = total_lines.saturating_sub(visible_height);
            let mut scrollbar_state = ScrollbarState::new(max_scroll)
                .position(self.scroll_offset);

            scrollbar.render(
                Rect {
                    x: inner.x + inner.width,
                    y: inner.y,
                    width: 1,
                    height: inner.height,
                },
                buf,
                &mut scrollbar_state,
            );
        }
    }

    /// Ensure the selected item is visible in the viewport
    fn ensure_selection_visible(
        &mut self,
        selected_start: usize,
        selected_end: usize,
        visible_height: usize,
        total_lines: usize,
    ) {
        // If selected item starts before viewport, scroll up
        if selected_start < self.scroll_offset {
            self.scroll_offset = selected_start;
        }
        // If selected item ends after viewport, scroll down
        else if selected_end > self.scroll_offset + visible_height {
            self.scroll_offset = selected_end.saturating_sub(visible_height);
        }

        // Clamp scroll offset
        let max_scroll = total_lines.saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);
    }
}

impl Default for RawEventsView {
    fn default() -> Self {
        Self::new()
    }
}
