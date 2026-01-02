//! Model selector dialog component

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};

use crate::agent::{AgentType, ModelInfo, ModelRegistry};

/// State for the model selector dialog
#[derive(Debug, Clone)]
pub struct ModelSelectorState {
    /// Whether the dialog is visible
    pub visible: bool,
    /// Currently selected index
    pub selected: usize,
    /// Current agent type (determines which models to show)
    pub agent_type: AgentType,
    /// Available models for the current agent
    pub models: Vec<ModelInfo>,
}

impl Default for ModelSelectorState {
    fn default() -> Self {
        Self::new(AgentType::Claude)
    }
}

impl ModelSelectorState {
    pub fn new(agent_type: AgentType) -> Self {
        let models = ModelRegistry::models_for(agent_type);
        Self {
            visible: false,
            selected: 0,
            agent_type,
            models,
        }
    }

    /// Show the dialog for a specific agent type
    pub fn show(&mut self, agent_type: AgentType) {
        self.visible = true;
        self.agent_type = agent_type;
        self.models = ModelRegistry::models_for(agent_type);
        self.selected = 0;
    }

    /// Hide the dialog
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Move selection up
    pub fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else if !self.models.is_empty() {
            self.selected = self.models.len() - 1;
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.models.is_empty() {
            self.selected = (self.selected + 1) % self.models.len();
        }
    }

    /// Get the currently selected model
    pub fn selected_model(&self) -> Option<&ModelInfo> {
        self.models.get(self.selected)
    }

    /// Check if dialog is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Model selector dialog widget
pub struct ModelSelector;

impl ModelSelector {
    pub fn new() -> Self {
        Self
    }

    /// Render the dialog
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &ModelSelectorState) {
        if !state.visible {
            return;
        }

        // Calculate dialog size
        let dialog_width = 50.min(area.width.saturating_sub(4));
        let dialog_height = (state.models.len() as u16 + 6).min(area.height.saturating_sub(2));

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
        let title = format!(" Select {} Model ", state.agent_type);
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(dialog_area);
        block.render(dialog_area, buf);

        // Layout inside dialog
        let mut constraints = vec![Constraint::Length(1)]; // Header
        for _ in &state.models {
            constraints.push(Constraint::Length(2));
        }
        constraints.push(Constraint::Length(1)); // Spacing
        constraints.push(Constraint::Length(1)); // Instructions

        let chunks = Layout::vertical(constraints).split(inner);

        // Render header
        let header = Paragraph::new("Choose a model:")
            .style(Style::default().fg(Color::White));
        header.render(chunks[0], buf);

        // Render model options
        for (i, model) in state.models.iter().enumerate() {
            let chunk_idx = i + 1;
            if chunk_idx >= chunks.len() - 2 {
                break;
            }

            let is_selected = i == state.selected;

            let style = if is_selected {
                Style::default()
                    .bg(Color::Rgb(40, 60, 80))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            // Model name and alias
            let name_line = Line::from(vec![
                Span::styled(
                    if is_selected { "▶ " } else { "  " },
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(&model.display_name, style),
                Span::styled(
                    format!(" ({})", model.alias),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            // Description
            let desc_line = Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    &model.description,
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            // Render both lines
            let option = Paragraph::new(vec![name_line, desc_line]);
            option.render(chunks[chunk_idx], buf);

            // Highlight background for selected
            if is_selected {
                for dy in 0..2 {
                    let row_y = chunks[chunk_idx].y + dy;
                    if row_y < chunks[chunk_idx].y + chunks[chunk_idx].height {
                        for dx in 0..chunks[chunk_idx].width {
                            buf[(chunks[chunk_idx].x + dx, row_y)]
                                .set_bg(Color::Rgb(40, 60, 80));
                        }
                    }
                }
            }
        }

        // Render instructions
        let instructions_idx = chunks.len() - 1;
        let instructions = Paragraph::new(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(Color::Cyan)),
            Span::raw(" select  "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" confirm  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" cancel"),
        ]))
        .alignment(Alignment::Center);
        instructions.render(chunks[instructions_idx], buf);
    }
}

impl Default for ModelSelector {
    fn default() -> Self {
        Self::new()
    }
}
