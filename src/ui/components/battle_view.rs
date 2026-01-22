//! Battle View - Split-pane rendering for head-to-head agent battles

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Widget},
};

use crate::agent::AgentType;
use crate::ui::battle::{BattleResults, BattleSession, BattleState};

use super::theme::{
    accent_error, accent_primary, accent_success, accent_warning, agent_claude, agent_codex,
    bg_base, bg_elevated, bg_surface, border_default, text_bright, text_faint, text_muted,
    text_primary, text_secondary,
};

/// Battle view widget - renders split-pane battle UI
pub struct BattleView<'a> {
    session: &'a BattleSession,
    focused: bool,
}

impl<'a> BattleView<'a> {
    pub fn new(session: &'a BattleSession) -> Self {
        Self {
            session,
            focused: true,
        }
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Render the battle header with timer and status
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let style = Style::default().bg(bg_elevated());

        // Clear background
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_style(style);
            }
        }

        // Battle title with swords emoji
        let title = match &self.session.state {
            BattleState::Idle => "⚔️  BATTLE MODE: Ready",
            BattleState::Countdown { remaining } => {
                return self.render_countdown(*remaining, area, buf);
            }
            BattleState::Racing => "⚔️  BATTLE MODE: Racing!",
            BattleState::OneComplete { first, .. } => match first {
                AgentType::Claude => "⚔️  BATTLE MODE: Claude finished!",
                AgentType::Codex => "⚔️  BATTLE MODE: Codex finished!",
            },
            BattleState::Completed { winner, .. } => match winner {
                AgentType::Claude => "🏆  BATTLE COMPLETE: Claude wins!",
                AgentType::Codex => "🏆  BATTLE COMPLETE: Codex wins!",
            },
            BattleState::Error { failed, .. } => match failed {
                AgentType::Claude => "⚠️  BATTLE: Claude errored",
                AgentType::Codex => "⚠️  BATTLE: Codex errored",
            },
            BattleState::BothErrored { .. } => "❌  BATTLE: Both agents failed",
            BattleState::ViewingResults => "📊  BATTLE RESULTS",
        };

        let title_style = Style::default()
            .fg(text_bright())
            .bg(bg_elevated())
            .add_modifier(Modifier::BOLD);

        // Render title on the left
        let title_span = Span::styled(title, title_style);
        buf.set_span(
            area.x + 2,
            area.y,
            &title_span,
            area.width.saturating_sub(4),
        );

        // Render timer on the right
        let timer = format!("⏱️  {}", self.session.elapsed_display());
        let timer_style = Style::default().fg(text_secondary()).bg(bg_elevated());
        let timer_x = area.x + area.width.saturating_sub(timer.len() as u16 + 2);
        buf.set_span(timer_x, area.y, &Span::styled(timer, timer_style), 15);
    }

    /// Render countdown animation
    fn render_countdown(&self, remaining: u8, area: Rect, buf: &mut Buffer) {
        let style = Style::default()
            .fg(accent_warning())
            .bg(bg_elevated())
            .add_modifier(Modifier::BOLD);

        let text = format!("⚔️  Starting in {}...", remaining);
        buf.set_span(area.x + 2, area.y, &Span::styled(text, style), area.width);
    }

    /// Render agent panel header with stats
    fn render_agent_header(&self, agent_type: AgentType, area: Rect, buf: &mut Buffer) {
        let agent = self.session.agent(agent_type);
        let agent_color = match agent_type {
            AgentType::Claude => agent_claude(),
            AgentType::Codex => agent_codex(),
        };

        let style = Style::default().bg(bg_surface());

        // Clear background
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_style(style);
            }
        }

        // Agent name with status emoji
        let name_line = Line::from(vec![
            Span::styled(
                format!("  {} ", agent.status_emoji()),
                Style::default().bg(bg_surface()),
            ),
            Span::styled(
                agent.display_name(),
                Style::default()
                    .fg(agent_color)
                    .bg(bg_surface())
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(area.x, area.y, &name_line, area.width);

        // Model name
        if let Some(model) = &agent.model {
            let model_text = format!("  Model: {}", model);
            let model_style = Style::default().fg(text_muted()).bg(bg_surface());
            buf.set_span(
                area.x,
                area.y + 1,
                &Span::styled(model_text, model_style),
                area.width,
            );
        }

        // Stats line (tokens, cost, time)
        let time_str = if agent.is_complete() || agent.is_processing {
            format!("{:.1}s", agent.elapsed().as_secs_f64())
        } else {
            "—".to_string()
        };

        let stats_line = Line::from(vec![
            Span::styled("  ", Style::default().bg(bg_surface())),
            Span::styled(
                format!("⏱️ {} ", time_str),
                Style::default().fg(text_secondary()).bg(bg_surface()),
            ),
            Span::styled(
                format!("💰 ${:.3} ", agent.estimated_cost()),
                Style::default().fg(text_secondary()).bg(bg_surface()),
            ),
            Span::styled(
                format!("📁 {} ", agent.files_modified.len()),
                Style::default().fg(text_secondary()).bg(bg_surface()),
            ),
        ]);
        buf.set_line(area.x, area.y + 2, &stats_line, area.width);

        // Token usage
        let tokens_line = Line::from(vec![
            Span::styled(
                "  Tokens: ",
                Style::default().fg(text_muted()).bg(bg_surface()),
            ),
            Span::styled(
                format!(
                    "{} in / {} out",
                    agent.usage.input_tokens, agent.usage.output_tokens
                ),
                Style::default().fg(text_faint()).bg(bg_surface()),
            ),
        ]);
        buf.set_line(area.x, area.y + 3, &tokens_line, area.width);
    }

    /// Render the prompt bar at the bottom
    fn render_prompt_bar(&self, area: Rect, buf: &mut Buffer) {
        let style = Style::default().bg(bg_surface());

        // Clear background
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_style(style);
            }
        }

        // Truncate prompt if too long
        let max_len = area.width.saturating_sub(12) as usize;
        let prompt_display = if self.session.prompt.len() > max_len {
            format!("{}...", &self.session.prompt[..max_len.saturating_sub(3)])
        } else {
            self.session.prompt.clone()
        };

        let prompt_line = Line::from(vec![
            Span::styled(
                "  Prompt: ",
                Style::default()
                    .fg(text_muted())
                    .bg(bg_surface())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("\"{}\"", prompt_display),
                Style::default().fg(text_primary()).bg(bg_surface()),
            ),
        ]);
        buf.set_line(area.x, area.y, &prompt_line, area.width);
    }

    /// Render the center divider
    fn render_divider(&self, area: Rect, buf: &mut Buffer) {
        let style = Style::default().fg(border_default()).bg(bg_base());

        for y in area.y..area.y + area.height {
            buf[(area.x, y)].set_char('│');
            buf[(area.x, y)].set_style(style);
        }
    }

    /// Render results overlay
    fn render_results_overlay(&self, area: Rect, buf: &mut Buffer) {
        let Some(results) = self.session.results() else {
            return;
        };

        // Center the results box
        let box_width = 60.min(area.width.saturating_sub(4));
        let box_height = 16.min(area.height.saturating_sub(4));
        let box_x = area.x + (area.width.saturating_sub(box_width)) / 2;
        let box_y = area.y + (area.height.saturating_sub(box_height)) / 2;

        let results_area = Rect::new(box_x, box_y, box_width, box_height);

        // Clear area and draw border
        Clear.render(results_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(accent_primary()))
            .title(" 🏆 BATTLE RESULTS 🏆 ")
            .title_style(
                Style::default()
                    .fg(text_bright())
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::default().bg(bg_elevated()));

        let inner = block.inner(results_area);
        block.render(results_area, buf);

        // Render results content
        self.render_results_content(&results, inner, buf);
    }

    fn render_results_content(&self, results: &BattleResults, area: Rect, buf: &mut Buffer) {
        let mut y = area.y;
        let _style = Style::default().bg(bg_elevated());

        // Winner announcement
        if let Some(winner) = results.winner_stats() {
            let winner_line = Line::from(vec![
                Span::styled(
                    "  🥇 WINNER: ",
                    Style::default().fg(accent_success()).bg(bg_elevated()),
                ),
                Span::styled(
                    winner.display_name(),
                    Style::default()
                        .fg(text_bright())
                        .bg(bg_elevated())
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            buf.set_line(area.x, y, &winner_line, area.width);
            y += 1;

            // Winner stats
            let stats_line = format!(
                "     ⏱️ {}  💰 {}  📁 {} files  🔧 {} tools",
                winner.time_display(),
                winner.cost_display(),
                winner.files_modified,
                winner.tool_calls
            );
            buf.set_span(
                area.x,
                y,
                &Span::styled(
                    stats_line,
                    Style::default().fg(text_secondary()).bg(bg_elevated()),
                ),
                area.width,
            );
            y += 2;
        }

        // Loser stats
        if let Some(loser) = results.loser_stats() {
            let loser_line = Line::from(vec![
                Span::styled("  🥈 ", Style::default().bg(bg_elevated())),
                Span::styled(
                    loser.display_name(),
                    Style::default().fg(text_muted()).bg(bg_elevated()),
                ),
            ]);
            buf.set_line(area.x, y, &loser_line, area.width);
            y += 1;

            let stats_line = format!(
                "     ⏱️ {}  💰 {}  📁 {} files  🔧 {} tools",
                loser.time_display(),
                loser.cost_display(),
                loser.files_modified,
                loser.tool_calls
            );
            buf.set_span(
                area.x,
                y,
                &Span::styled(
                    stats_line,
                    Style::default().fg(text_faint()).bg(bg_elevated()),
                ),
                area.width,
            );
            y += 2;
        }

        // Margin
        if let Some(margin) = results.margin {
            if margin.as_millis() > 100 {
                let margin_line = format!("  ⚡ Winning margin: {:.1}s", margin.as_secs_f64());
                buf.set_span(
                    area.x,
                    y,
                    &Span::styled(
                        margin_line,
                        Style::default().fg(accent_warning()).bg(bg_elevated()),
                    ),
                    area.width,
                );
                let _ = y; // Silence unused assignment warning
            }
        }

        // Key hints
        let y = area.y + area.height.saturating_sub(2);
        let hints = "  [S] Share  [C] Compare  [Esc] Close";
        buf.set_span(
            area.x,
            y,
            &Span::styled(hints, Style::default().fg(text_muted()).bg(bg_elevated())),
            area.width,
        );
    }
}

impl Widget for BattleView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 40 || area.height < 10 {
            // Too small to render battle view
            let msg = "Terminal too small for Battle Mode";
            buf.set_string(area.x, area.y, msg, Style::default().fg(accent_error()));
            return;
        }

        // Layout:
        // ┌─────────────────────────────────────────────────────────────────┐
        // │  Header (timer, status)                                    1    │
        // ├─────────────────────────────┬───────────────────────────────────┤
        // │  Left Agent Header     4    │  Right Agent Header          4    │
        // ├─────────────────────────────┼───────────────────────────────────┤
        // │  Left Chat View             │  Right Chat View                  │
        // │                             │                                   │
        // ├─────────────────────────────┴───────────────────────────────────┤
        // │  Prompt Bar                                                 1    │
        // └─────────────────────────────────────────────────────────────────┘

        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(4), // Agent headers
                Constraint::Min(5),    // Chat views
                Constraint::Length(1), // Prompt bar
            ])
            .split(area);

        // Render header
        self.render_header(vertical_chunks[0], buf);

        // Split agent areas horizontally
        let agent_header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(vertical_chunks[1]);

        let chat_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(vertical_chunks[2]);

        // Render agent headers
        self.render_agent_header(AgentType::Claude, agent_header_chunks[0], buf);
        self.render_agent_header(AgentType::Codex, agent_header_chunks[1], buf);

        // Render divider between chat views
        let divider_x = chat_chunks[0].x + chat_chunks[0].width;
        let divider_area = Rect::new(divider_x, chat_chunks[0].y, 1, chat_chunks[0].height);
        self.render_divider(divider_area, buf);

        // Render chat views
        // Note: ChatView needs to be rendered via its own widget impl
        // For now, render placeholder or delegate to session's chat views

        // Background for chat areas
        let chat_style = Style::default().bg(bg_base());
        for chunk in [chat_chunks[0], chat_chunks[1]] {
            for y in chunk.y..chunk.y + chunk.height {
                for x in chunk.x..chunk.x + chunk.width {
                    buf[(x, y)].set_style(chat_style);
                }
            }
        }

        // Render prompt bar
        self.render_prompt_bar(vertical_chunks[3], buf);

        // Render results overlay if viewing results
        if matches!(self.session.state, BattleState::ViewingResults) {
            self.render_results_overlay(area, buf);
        }
    }
}

/// Battle header widget for use in tab bar
pub struct BattleTabIndicator {
    pub is_racing: bool,
    pub winner: Option<AgentType>,
}

impl BattleTabIndicator {
    pub fn from_session(session: &BattleSession) -> Self {
        Self {
            is_racing: session.is_racing(),
            winner: session.state.winner(),
        }
    }

    /// Get tab prefix
    pub fn prefix(&self) -> &'static str {
        if self.is_racing {
            "⚔️"
        } else if self.winner.is_some() {
            "🏆"
        } else {
            "⚔️"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_battle_view_creation() {
        let session = BattleSession::new(PathBuf::from("/test"));
        let view = BattleView::new(&session);
        assert!(view.focused);
    }

    #[test]
    fn test_tab_indicator() {
        let mut session = BattleSession::new(PathBuf::from("/test"));
        let indicator = BattleTabIndicator::from_session(&session);
        assert!(!indicator.is_racing);
        assert_eq!(indicator.prefix(), "⚔️");

        session.start("test".into());
        let indicator = BattleTabIndicator::from_session(&session);
        assert!(indicator.is_racing);
    }
}
