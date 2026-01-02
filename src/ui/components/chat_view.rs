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

use super::MarkdownRenderer;

/// Role of a chat message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

/// A single chat message
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<String>,
    pub is_streaming: bool,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            tool_name: None,
            tool_args: None,
            is_streaming: false,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_name: None,
            tool_args: None,
            is_streaming: false,
        }
    }

    pub fn tool(name: impl Into<String>, args: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_name: Some(name.into()),
            tool_args: Some(args.into()),
            is_streaming: false,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            tool_name: None,
            tool_args: None,
            is_streaming: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Error,
            content: content.into(),
            tool_name: None,
            tool_args: None,
            is_streaming: false,
        }
    }

    pub fn streaming(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            tool_name: None,
            tool_args: None,
            is_streaming: true,
        }
    }
}

/// Chat view component displaying message history
pub struct ChatView {
    /// All messages in the chat
    messages: Vec<ChatMessage>,
    /// Scroll offset (0 = bottom, increases upward)
    scroll_offset: usize,
    /// Currently streaming message buffer
    streaming_buffer: Option<String>,
}

impl ChatView {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            scroll_offset: 0,
            streaming_buffer: None,
        }
    }

    /// Add a message to the chat
    pub fn push(&mut self, message: ChatMessage) {
        // If we were streaming, finalize it
        if self.streaming_buffer.is_some() {
            self.finalize_streaming();
        }
        self.messages.push(message);
        // Auto-scroll to bottom on new message
        self.scroll_offset = 0;
    }

    /// Start or append to streaming message
    pub fn stream_append(&mut self, text: &str) {
        match &mut self.streaming_buffer {
            Some(buffer) => {
                buffer.push_str(text);
            }
            None => {
                self.streaming_buffer = Some(text.to_string());
            }
        }
    }

    /// Finalize streaming message and add to history
    pub fn finalize_streaming(&mut self) {
        if let Some(content) = self.streaming_buffer.take() {
            self.messages.push(ChatMessage::assistant(content));
            self.scroll_offset = 0;
        }
    }

    /// Clear all messages
    pub fn clear(&mut self) {
        self.messages.clear();
        self.streaming_buffer = None;
        self.scroll_offset = 0;
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(n);
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll to top
    pub fn scroll_to_top(&mut self) {
        // Will be clamped during render
        self.scroll_offset = usize::MAX;
    }

    /// Scroll to bottom
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// Get message count
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.streaming_buffer.is_none()
    }

    /// Build lines for rendering
    fn build_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        for msg in &self.messages {
            self.format_message(msg, width, &mut lines);
            lines.push(Line::from("")); // Spacing between messages
        }

        // Add streaming buffer if present
        if let Some(ref buffer) = self.streaming_buffer {
            let msg = ChatMessage::streaming(buffer.clone());
            self.format_message(&msg, width, &mut lines);
        }

        lines
    }

    fn format_message(&self, msg: &ChatMessage, _width: usize, lines: &mut Vec<Line<'static>>) {
        match msg.role {
            MessageRole::Tool => self.format_tool_message(msg, lines),
            MessageRole::User => self.format_user_message(msg, lines),
            MessageRole::Assistant => self.format_assistant_message(msg, lines),
            MessageRole::System => self.format_system_message(msg, lines),
            MessageRole::Error => self.format_error_message(msg, lines),
        }
    }

    /// Format user messages with chevron prefix
    fn format_user_message(&self, msg: &ChatMessage, lines: &mut Vec<Line<'static>>) {
        let content_lines: Vec<&str> = msg.content.lines().collect();

        for (i, line) in content_lines.iter().enumerate() {
            if i == 0 {
                // First line with chevron
                lines.push(Line::from(vec![
                    Span::styled("❯ ", Style::default().fg(Color::Green)),
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else {
                // Continuation lines indented
                lines.push(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        }
    }

    /// Format assistant messages - flowing text with markdown
    fn format_assistant_message(&self, msg: &ChatMessage, lines: &mut Vec<Line<'static>>) {
        if msg.content.is_empty() {
            return;
        }

        // Parse markdown with custom renderer
        let renderer = MarkdownRenderer::new();
        let md_text = renderer.render(&msg.content);

        for line in md_text.lines {
            // Indent content slightly
            let mut indented_spans = vec![Span::raw("  ")];
            indented_spans.extend(line.spans.into_iter().map(|s| {
                // Apply a slightly dimmer style for assistant text
                let mut style = s.style;
                if style.fg.is_none() {
                    style = style.fg(Color::Rgb(220, 220, 220)); // Slightly dimmer white
                }
                Span::styled(s.content.into_owned(), style)
            }));
            lines.push(Line::from(indented_spans));
        }

        // Add streaming indicator if still streaming
        if msg.is_streaming {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "...",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]));
        }
    }

    /// Format system messages with info symbol
    fn format_system_message(&self, msg: &ChatMessage, lines: &mut Vec<Line<'static>>) {
        let content_lines: Vec<&str> = msg.content.lines().collect();

        for (i, line) in content_lines.iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled("ℹ ", Style::default().fg(Color::Blue)),
                    Span::styled(line.to_string(), Style::default().fg(Color::Blue)),
                ]));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default().fg(Color::Blue),
                )));
            }
        }
    }

    /// Format error messages with X symbol
    fn format_error_message(&self, msg: &ChatMessage, lines: &mut Vec<Line<'static>>) {
        let content_lines: Vec<&str> = msg.content.lines().collect();

        for (i, line) in content_lines.iter().enumerate() {
            if i == 0 {
                lines.push(Line::from(vec![
                    Span::styled("✗ ", Style::default().fg(Color::Red)),
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }

    /// Format tool messages in Claude Code style
    fn format_tool_message(&self, msg: &ChatMessage, lines: &mut Vec<Line<'static>>) {
        let tool_name = msg.tool_name.as_deref().unwrap_or("Tool");
        let tool_args = msg.tool_args.as_deref().unwrap_or("");

        // Header: ● ToolName(args)
        let header_spans = vec![
            Span::styled("● ", Style::default().fg(Color::Green)),
            Span::styled(
                tool_name.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({})", tool_args),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        lines.push(Line::from(header_spans));

        // Output lines with tree connector
        let content_lines: Vec<&str> = msg.content.lines().collect();
        let last_idx = content_lines.len().saturating_sub(1);

        for (i, line) in content_lines.iter().enumerate() {
            let connector = if i == last_idx { "└ " } else { "│ " };
            lines.push(Line::from(vec![
                Span::styled(connector, Style::default().fg(Color::DarkGray)),
                Span::styled(format!("   {}", line), Style::default().fg(Color::White)),
            ]));
        }
    }

    /// Render the chat view
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.render_with_indicator(area, buf, None);
    }

    /// Render the chat view with an optional thinking indicator
    pub fn render_with_indicator(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        thinking_line: Option<Line<'static>>,
    ) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Chat ");

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 3 || inner.height < 1 {
            return;
        }

        let mut lines = self.build_lines(inner.width as usize);

        // Append thinking indicator if provided
        if let Some(indicator) = thinking_line {
            lines.push(Line::from("")); // Empty line before indicator
            lines.push(indicator);
        }
        let total_lines = lines.len();
        let visible_height = inner.height as usize;

        // Clamp scroll offset
        let max_scroll = total_lines.saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.min(max_scroll);

        // Calculate which lines to show (from bottom)
        let start_line = total_lines.saturating_sub(visible_height + self.scroll_offset);
        let end_line = total_lines.saturating_sub(self.scroll_offset);

        let visible_lines: Vec<Line<'static>> = lines[start_line..end_line].to_vec();

        let paragraph = Paragraph::new(visible_lines).wrap(Wrap { trim: false });

        paragraph.render(inner, buf);

        // Render scrollbar if needed
        if total_lines > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            let mut scrollbar_state = ScrollbarState::new(max_scroll)
                .position(max_scroll.saturating_sub(self.scroll_offset));

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
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}
