use std::collections::HashSet;
use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget, Widget, Wrap,
    },
};
use serde_json::Value;

/// State for the event detail panel
#[derive(Debug, Clone, Default)]
pub struct EventDetailState {
    /// Whether the detail panel is visible
    pub visible: bool,
    /// Index of the event being viewed
    pub event_index: usize,
    /// Scroll offset within the detail content
    pub scroll_offset: usize,
}

impl EventDetailState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the detail panel for a specific event
    pub fn open(&mut self, event_index: usize) {
        self.visible = true;
        self.event_index = event_index;
        self.scroll_offset = 0;
    }

    /// Close the detail panel
    pub fn close(&mut self) {
        self.visible = false;
    }

    /// Toggle the detail panel visibility
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.scroll_offset = 0;
    }

    /// Sync to a new event index (resets scroll)
    pub fn sync_to_event(&mut self, event_index: usize) {
        if self.event_index != event_index {
            self.event_index = event_index;
            self.scroll_offset = 0;
        }
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll down by n lines (clamped to max)
    pub fn scroll_down(&mut self, n: usize, content_height: usize, visible_height: usize) {
        let max_scroll = content_height.saturating_sub(visible_height);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Scroll up by a page
    pub fn page_up(&mut self, visible_height: usize) {
        self.scroll_up(visible_height.saturating_sub(2));
    }

    /// Scroll down by a page
    pub fn page_down(&mut self, visible_height: usize, content_height: usize) {
        self.scroll_down(visible_height.saturating_sub(2), content_height, visible_height);
    }

    /// Jump to top
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// Jump to bottom
    pub fn scroll_to_bottom(&mut self, content_height: usize, visible_height: usize) {
        self.scroll_offset = content_height.saturating_sub(visible_height);
    }
}

/// Minimum width for split layout (below this, use overlay)
pub const DETAIL_PANEL_BREAKPOINT: u16 = 100;

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
                    // Check if this quote is escaped (preceded by odd number of backslashes)
                    let is_escaped = Self::is_escaped(&current);

                    if in_string && !is_escaped {
                        // End of string (unescaped quote)
                        current.push(ch);
                        let color = if is_key { Color::Cyan } else { Color::Green };
                        spans.push(Span::styled(current.clone(), Style::default().fg(color)));
                        current.clear();
                        in_string = false;
                        is_key = false;
                    } else if !in_string {
                        // Start of string
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        in_string = true;
                        is_key = true;
                        current.push(ch);
                    } else {
                        // Escaped quote inside string - just add it
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

    /// Check if the current position is escaped (odd number of trailing backslashes)
    fn is_escaped(current: &str) -> bool {
        let trailing_backslashes = current.chars().rev().take_while(|&c| c == '\\').count();
        trailing_backslashes % 2 == 1
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
    /// Set of expanded event indices (allows multiple items to be expanded)
    expanded_indices: HashSet<usize>,
    /// Scroll offset for viewport (in lines)
    scroll_offset: usize,
    /// Session start time
    session_start: Instant,
    /// Event detail panel state
    pub event_detail: EventDetailState,
}

impl RawEventsView {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            selected_index: 0,
            expanded_indices: HashSet::new(),
            scroll_offset: 0,
            session_start: Instant::now(),
            event_detail: EventDetailState::new(),
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
        // Auto-select new event (keep existing expansions)
        self.selected_index = self.events.len().saturating_sub(1);
    }

    /// Move selection to previous event
    pub fn select_prev(&mut self) {
        if self.events.is_empty() {
            return;
        }
        self.selected_index = self.selected_index.saturating_sub(1);
        // Sync detail panel to follow selection
        self.event_detail.sync_to_event(self.selected_index);
    }

    /// Move selection to next event
    pub fn select_next(&mut self) {
        if self.events.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1).min(self.events.len().saturating_sub(1));
        // Sync detail panel to follow selection
        self.event_detail.sync_to_event(self.selected_index);
    }

    /// Toggle expand/collapse of selected event
    pub fn toggle_expand(&mut self) {
        if !self.events.is_empty() {
            if self.expanded_indices.contains(&self.selected_index) {
                self.expanded_indices.remove(&self.selected_index);
            } else {
                self.expanded_indices.insert(self.selected_index);
            }
        }
    }

    /// Collapse all expanded events
    pub fn collapse(&mut self) {
        self.expanded_indices.clear();
    }

    /// Check if the selected event is expanded
    pub fn is_expanded(&self) -> bool {
        self.expanded_indices.contains(&self.selected_index)
    }

    /// Check if a specific event is expanded
    fn is_index_expanded(&self, index: usize) -> bool {
        self.expanded_indices.contains(&index)
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
        self.expanded_indices.clear();
        self.scroll_offset = 0;
        self.session_start = Instant::now();
        self.event_detail = EventDetailState::new();
    }

    /// Toggle detail panel visibility
    pub fn toggle_detail(&mut self) {
        if !self.events.is_empty() {
            // Sync to current selection before toggling
            self.event_detail.sync_to_event(self.selected_index);
            self.event_detail.toggle();
        }
    }

    /// Check if detail panel is visible
    pub fn is_detail_visible(&self) -> bool {
        self.event_detail.visible
    }

    /// Get the selected event's JSON as pretty-printed string (for copy action)
    pub fn get_selected_json(&self) -> Option<String> {
        self.events
            .get(self.selected_index)
            .and_then(|event| serde_json::to_string_pretty(&event.raw_json).ok())
    }

    /// Get the selected event index
    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    /// Build detail content lines for the current event
    fn build_detail_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        let Some(event) = self.events.get(self.event_detail.event_index) else {
            lines.push(Line::from(Span::styled(
                "No event selected",
                Style::default().fg(Color::DarkGray),
            )));
            return lines;
        };

        // Header with direction and timestamp
        let direction_symbol = event.direction.symbol();
        let direction_label = match event.direction {
            EventDirection::Sent => "Outgoing",
            EventDirection::Received => "Incoming",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} {} ", direction_symbol, direction_label),
                Style::default().fg(event.direction.color()),
            ),
            Span::styled(
                event.format_timestamp(),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        // Event type
        lines.push(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                event.event_type.clone(),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]));

        // Separator
        lines.push(Line::from(""));

        // Full JSON with syntax highlighting (no line limit)
        if let Ok(pretty) = serde_json::to_string_pretty(&event.raw_json) {
            for json_line in pretty.lines() {
                let mut highlighted = Vec::new();
                highlighted.extend(RawEventEntry::highlight_json_line(json_line));
                lines.push(Line::from(highlighted));
            }
        }

        lines
    }

    /// Get total content height for detail panel
    pub fn detail_content_height(&self) -> usize {
        self.build_detail_lines().len()
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize, visible_height: usize) {
        let (lines, _, _) = self.build_lines();
        let max_scroll = lines.len().saturating_sub(visible_height);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Handle a click at the given position within the view area
    /// Returns Some(event_index) if a click was handled, None otherwise
    pub fn handle_click(&mut self, x: u16, y: u16, area: Rect) -> Option<usize> {
        // Check if click is within the inner area (accounting for border)
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_height = area.height.saturating_sub(2);

        if x < inner_x || y < inner_y || y >= inner_y + inner_height {
            return None;
        }

        // Calculate which line was clicked
        let clicked_line = (y - inner_y) as usize + self.scroll_offset;

        // Map the clicked line to an event index
        let (lines, _, _) = self.build_lines();
        if clicked_line >= lines.len() {
            return None;
        }

        // Walk through events to find which one corresponds to the clicked line
        let mut current_line: usize = 0;
        for (i, _event) in self.events.iter().enumerate() {
            let is_expanded = self.is_index_expanded(i);
            let event_start = current_line;

            // Calculate how many lines this event takes
            let event_lines = if is_expanded {
                // Header + JSON lines (max 20) + possible truncation line
                1 + self.events[i].render_json_lines(20).len()
            } else {
                1
            };

            let event_end = current_line + event_lines;

            if clicked_line >= event_start && clicked_line < event_end {
                // Clicked on this event - select it
                self.selected_index = i;
                // Sync detail panel to follow selection
                self.event_detail.sync_to_event(self.selected_index);
                return Some(i);
            }

            current_line = event_end;
        }

        None
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
            let is_expanded = self.is_index_expanded(i);

            if is_selected {
                selected_start = lines.len();
            }

            if is_expanded {
                // Expanded view: header with ▼ prefix, then JSON
                let bg_style = if is_selected {
                    Style::default().bg(Color::Rgb(40, 40, 50))
                } else {
                    Style::default()
                };
                let header = event.render_compact("▼", bg_style);
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

    /// Render the raw events view with optional detail panel
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let use_split = self.event_detail.visible && area.width >= DETAIL_PANEL_BREAKPOINT;
        let use_overlay = self.event_detail.visible && area.width < DETAIL_PANEL_BREAKPOINT;

        if use_split {
            // Split layout: event list on left, detail panel on right
            let chunks = Layout::horizontal([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(area);

            self.render_event_list(chunks[0], buf);
            self.render_detail_panel(chunks[1], buf);
        } else {
            // Full width event list
            self.render_event_list(area, buf);

            // Overlay detail panel if visible
            if use_overlay {
                self.render_detail_overlay(area, buf);
            }
        }
    }

    /// Render the event list
    fn render_event_list(&mut self, area: Rect, buf: &mut Buffer) {
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

    /// Render the detail panel (split mode - right side)
    fn render_detail_panel(&mut self, area: Rect, buf: &mut Buffer) {
        let event_type = self
            .events
            .get(self.event_detail.event_index)
            .map(|e| e.event_type.as_str())
            .unwrap_or("Event");
        let title = format!(" {} ", event_type);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);

        let inner = block.inner(area);
        block.render(area, buf);

        self.render_detail_content(inner, buf);
    }

    /// Render the detail overlay (centered floating dialog)
    fn render_detail_overlay(&mut self, area: Rect, buf: &mut Buffer) {
        // Calculate overlay size (90% width, 80% height)
        let overlay_width = (area.width as f32 * 0.9) as u16;
        let overlay_height = (area.height as f32 * 0.8) as u16;

        let overlay_x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
        let overlay_y = area.y + (area.height.saturating_sub(overlay_height)) / 2;

        let overlay_area = Rect {
            x: overlay_x,
            y: overlay_y,
            width: overlay_width,
            height: overlay_height,
        };

        // Clear the overlay area
        Clear.render(overlay_area, buf);

        let event_type = self
            .events
            .get(self.event_detail.event_index)
            .map(|e| e.event_type.as_str())
            .unwrap_or("Event");
        let title = format!(" {} ", event_type);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);

        let inner = block.inner(overlay_area);
        block.render(overlay_area, buf);

        self.render_detail_content(inner, buf);
    }

    /// Render the detail content (shared by panel and overlay)
    fn render_detail_content(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 3 || area.height < 1 {
            return;
        }

        let lines = self.build_detail_lines();
        let total_lines = lines.len();
        let visible_height = area.height as usize;

        // Clamp scroll offset
        let max_scroll = total_lines.saturating_sub(visible_height);
        if self.event_detail.scroll_offset > max_scroll {
            self.event_detail.scroll_offset = max_scroll;
        }

        // Calculate which lines to show
        let start_line = self.event_detail.scroll_offset;
        let end_line = (self.event_detail.scroll_offset + visible_height).min(total_lines);

        let visible_lines: Vec<Line<'static>> = if start_line < end_line && end_line <= lines.len()
        {
            lines[start_line..end_line].to_vec()
        } else {
            vec![]
        };

        let paragraph = Paragraph::new(visible_lines);
        paragraph.render(area, buf);

        // Render scrollbar if content overflows
        if total_lines > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            let mut scrollbar_state =
                ScrollbarState::new(max_scroll).position(self.event_detail.scroll_offset);

            scrollbar.render(
                Rect {
                    x: area.x + area.width,
                    y: area.y,
                    width: 1,
                    height: area.height,
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
