use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::agent::{AgentType, ModelRegistry, SessionId, TokenUsage};
use crate::ui::components::Spinner;

/// Status bar component showing session info
pub struct StatusBar {
    agent_type: AgentType,
    model: Option<String>,
    session_id: Option<SessionId>,
    token_usage: TokenUsage,
    estimated_cost: f64,
    is_processing: bool,
    spinner: Spinner,
}

impl StatusBar {
    pub fn new(agent_type: AgentType) -> Self {
        Self {
            agent_type,
            model: None,
            session_id: None,
            token_usage: TokenUsage::default(),
            estimated_cost: 0.0,
            is_processing: false,
            spinner: Spinner::dots(),
        }
    }

    /// Advance spinner animation
    pub fn tick(&mut self) {
        if self.is_processing {
            self.spinner.tick();
        }
    }

    pub fn set_session_id(&mut self, id: Option<SessionId>) {
        self.session_id = id;
    }

    pub fn set_agent_type(&mut self, agent_type: AgentType) {
        self.agent_type = agent_type;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
    }

    pub fn set_token_usage(&mut self, usage: TokenUsage) {
        self.token_usage = usage;
        self.update_cost();
    }

    pub fn set_processing(&mut self, processing: bool) {
        self.is_processing = processing;
    }

    fn update_cost(&mut self) {
        // Claude Sonnet pricing: $3/1M input, $15/1M output
        let input_cost = (self.token_usage.input_tokens as f64 / 1_000_000.0) * 3.0;
        let output_cost = (self.token_usage.output_tokens as f64 / 1_000_000.0) * 15.0;
        self.estimated_cost = input_cost + output_cost;
    }

    fn format_tokens(&self, tokens: i64) -> String {
        if tokens >= 1_000_000 {
            format!("{:.1}M", tokens as f64 / 1_000_000.0)
        } else if tokens >= 1_000 {
            format!("{:.1}k", tokens as f64 / 1_000.0)
        } else {
            format!("{}", tokens)
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = Vec::new();

        // Agent type indicator with icon
        let agent_color = match self.agent_type {
            AgentType::Claude => Color::Cyan,
            AgentType::Codex => Color::Magenta,
        };
        let agent_icon = ModelRegistry::agent_icon(self.agent_type);

        spans.push(Span::styled(
            format!(" {} {} ", agent_icon, self.agent_type.display_name()),
            Style::default()
                .bg(agent_color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ));

        spans.push(Span::raw(" "));

        // Model name
        let model_display = if let Some(ref model_id) = self.model {
            // Try to find the model's display name
            ModelRegistry::find_model(self.agent_type, model_id)
                .map(|m| m.display_name)
                .unwrap_or_else(|| model_id.clone())
        } else {
            // Show default model
            ModelRegistry::default_model(self.agent_type)
        };

        spans.push(Span::styled(
            format!("{} ", model_display),
            Style::default().fg(Color::White),
        ));

        // Session ID (smaller, dimmed)
        if let Some(ref id) = self.session_id {
            let short_id = &id.as_str()[..8.min(id.as_str().len())];
            spans.push(Span::styled(
                format!("({})", short_id),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" "));
        }

        // Processing indicator with spinner
        if self.is_processing {
            spans.push(self.spinner.span(Color::Yellow));
            spans.push(Span::styled(
                " thinking... ",
                Style::default().fg(Color::Yellow),
            ));
        }

        spans.push(Span::raw("│ "));

        // Token usage
        let input_str = self.format_tokens(self.token_usage.input_tokens);
        let output_str = self.format_tokens(self.token_usage.output_tokens);

        spans.push(Span::styled("in:", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("{} ", input_str),
            Style::default().fg(Color::White),
        ));

        spans.push(Span::styled("out:", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!("{} ", output_str),
            Style::default().fg(Color::White),
        ));

        // Cached tokens if any
        if self.token_usage.cached_tokens > 0 {
            spans.push(Span::styled(
                format!("(+{} cached) ", self.format_tokens(self.token_usage.cached_tokens)),
                Style::default().fg(Color::Green),
            ));
        }

        spans.push(Span::raw("│ "));

        // Estimated cost
        spans.push(Span::styled(
            format!("${:.4}", self.estimated_cost),
            Style::default().fg(if self.estimated_cost > 0.1 {
                Color::Yellow
            } else {
                Color::Green
            }),
        ));

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line)
            .style(Style::default().bg(Color::Rgb(30, 30, 30)));

        paragraph.render(area, buf);
    }
}
