//! Slash command menu component.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{
    accent_primary, bg_highlight, dialog_bg, ensure_contrast_bg, ensure_contrast_fg,
    render_minimal_scrollbar, text_muted, text_primary, SearchableListState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashCommand {
    Model,
    NewSession,
}

impl SlashCommand {
    pub fn label(&self) -> &'static str {
        match self {
            SlashCommand::Model => "/model",
            SlashCommand::NewSession => "/new",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            SlashCommand::Model => "Select model",
            SlashCommand::NewSession => "Start a new session",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlashCommandEntry {
    pub command: SlashCommand,
    pub label: &'static str,
    pub description: &'static str,
}

impl SlashCommandEntry {
    fn new(command: SlashCommand) -> Self {
        Self {
            command,
            label: command.label(),
            description: command.description(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    pub visible: bool,
    pub commands: Vec<SlashCommandEntry>,
    pub list: SearchableListState,
}

impl SlashMenuState {
    pub fn new() -> Self {
        Self {
            visible: false,
            commands: Vec::new(),
            list: SearchableListState::new(6),
        }
    }

    pub fn show(&mut self) {
        self.visible = true;
        self.commands = Self::build_commands();
        self.list.reset();
        self.list.filtered = (0..self.commands.len()).collect();
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn filtered_len(&self) -> usize {
        self.list.filtered.len()
    }

    pub fn set_max_visible(&mut self, max_visible: usize) {
        let max_visible = max_visible.max(1);
        self.list.max_visible = max_visible;
        self.list.clamp_selection();
        if self.list.selected < self.list.scroll_offset {
            self.list.scroll_offset = self.list.selected;
        } else if self.list.selected >= self.list.scroll_offset + self.list.max_visible {
            self.list.scroll_offset = self
                .list
                .selected
                .saturating_sub(self.list.max_visible.saturating_sub(1));
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.list.search.insert_char(c);
        self.filter();
    }

    pub fn delete_char(&mut self) {
        self.list.search.delete_char();
        self.filter();
    }

    pub fn delete_forward(&mut self) {
        self.list.search.delete_forward();
        self.filter();
    }

    pub fn select_next(&mut self) {
        self.list.select_next();
    }

    pub fn select_prev(&mut self) {
        self.list.select_prev();
    }

    pub fn selected_entry(&self) -> Option<&SlashCommandEntry> {
        if self.list.filtered.is_empty() {
            return None;
        }
        let idx = self.list.filtered.get(self.list.selected)?;
        self.commands.get(*idx)
    }

    fn build_commands() -> Vec<SlashCommandEntry> {
        vec![
            SlashCommandEntry::new(SlashCommand::Model),
            SlashCommandEntry::new(SlashCommand::NewSession),
        ]
    }

    fn filter(&mut self) {
        let query = self.list.search.value().to_lowercase();
        let filtered: Vec<usize> = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                if query.is_empty() {
                    return true;
                }
                cmd.label.to_lowercase().contains(&query)
                    || cmd.description.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        self.list.set_filtered(filtered);
    }
}

impl Default for SlashMenuState {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SlashMenu;

impl SlashMenu {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &SlashMenuState) {
        if !state.visible {
            return;
        }

        if area.height < 5 || area.width < 10 {
            return;
        }

        Clear.render(area, buf);
        buf.set_style(area, Style::default().bg(dialog_bg()));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(Style::default().fg(accent_primary()).bg(dialog_bg()))
            .style(Style::default().bg(dialog_bg()));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width == 0 {
            return;
        }

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

        self.render_search(chunks[0], buf, state);
        self.render_separator(chunks[1], buf);
        self.render_list(chunks[2], buf, state);
    }

    fn render_search(&self, area: Rect, buf: &mut Buffer, state: &SlashMenuState) {
        let prompt = "/";
        let input = state.list.search.value();

        if input.is_empty() {
            let placeholder = format!("{prompt} Type a command...");
            Paragraph::new(placeholder)
                .style(Style::default().fg(text_muted()))
                .render(area, buf);
        } else {
            let line = Line::from(vec![
                Span::styled(prompt, Style::default().fg(accent_primary())),
                Span::styled(input, Style::default().fg(text_primary())),
            ]);
            Paragraph::new(line).render(area, buf);
        }

        let prompt_width = UnicodeWidthStr::width(prompt) as u16;
        let cursor_offset = input
            .chars()
            .take(state.list.search.cursor)
            .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1) as u16)
            .sum::<u16>();
        let cursor_x = area.x + prompt_width + cursor_offset;
        if cursor_x < area.x + area.width {
            buf[(cursor_x, area.y)]
                .set_style(Style::default().add_modifier(ratatui::style::Modifier::REVERSED));
        }
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let separator = "\u{2500}".repeat(area.width as usize);
        Paragraph::new(separator)
            .style(Style::default().fg(text_muted()))
            .render(area, buf);
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer, state: &SlashMenuState) {
        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                buf[(x, y)].set_bg(dialog_bg());
            }
        }

        if state.list.filtered.is_empty() {
            let msg = if state.commands.is_empty() {
                "No commands available"
            } else {
                "No matching commands"
            };
            Paragraph::new(msg)
                .style(Style::default().fg(text_muted()))
                .render(area, buf);
            return;
        }

        let visible_count = area.height as usize;
        let has_scrollbar = state.list.filtered.len() > visible_count;
        let content_width = if has_scrollbar {
            area.width.saturating_sub(1)
        } else {
            area.width
        };

        let selected_bg = ensure_contrast_bg(bg_highlight(), dialog_bg(), 2.0);
        let selected_fg = ensure_contrast_fg(text_primary(), selected_bg, 4.5);
        let selected_muted = ensure_contrast_fg(text_muted(), selected_bg, 3.0);
        let selected_accent = ensure_contrast_fg(accent_primary(), selected_bg, 3.0);

        for (i, &cmd_idx) in state
            .list
            .filtered
            .iter()
            .skip(state.list.scroll_offset)
            .take(visible_count)
            .enumerate()
        {
            let entry = &state.commands[cmd_idx];
            let is_selected = state.list.scroll_offset + i == state.list.selected;
            let y = area.y + i as u16;

            let prefix = if is_selected { "> " } else { "  " };
            let prefix_width = UnicodeWidthStr::width(prefix);
            let available_cmd_width = (content_width as usize).saturating_sub(prefix_width);
            let cmd_display = truncate_to_width(entry.label, available_cmd_width);
            let cmd_width = UnicodeWidthStr::width(cmd_display.as_str());
            let has_desc = !entry.description.is_empty();
            let gap = if has_desc { 3 } else { 0 };
            let available_desc_width =
                (content_width as usize).saturating_sub(prefix_width + cmd_width + gap);
            let desc_display = if has_desc && available_desc_width > 0 {
                truncate_to_width(entry.description, available_desc_width)
            } else {
                String::new()
            };

            let (prefix_style, cmd_style, desc_style) = if is_selected {
                (
                    Style::default().fg(selected_accent).bg(selected_bg),
                    Style::default().fg(selected_fg).bg(selected_bg),
                    Style::default().fg(selected_muted).bg(selected_bg),
                )
            } else {
                (
                    Style::default().fg(text_muted()),
                    Style::default().fg(text_primary()),
                    Style::default().fg(text_muted()),
                )
            };

            if is_selected {
                for x in area.x..area.x + content_width {
                    buf[(x, y)].set_bg(selected_bg);
                }
            }

            let mut spans = Vec::new();
            spans.push(Span::styled(prefix, prefix_style));
            spans.push(Span::styled(cmd_display, cmd_style));
            if !desc_display.is_empty() {
                spans.push(Span::styled(" - ", desc_style));
                spans.push(Span::styled(desc_display, desc_style));
            }

            let line = Line::from(spans);
            let line_area = Rect {
                x: area.x,
                y,
                width: content_width,
                height: 1,
            };
            Paragraph::new(line).render(line_area, buf);
        }

        if has_scrollbar {
            let scrollbar_area = Rect {
                x: area.x + area.width - 1,
                y: area.y,
                width: 1,
                height: area.height,
            };
            render_minimal_scrollbar(
                scrollbar_area,
                buf,
                state.list.filtered.len(),
                visible_count,
                state.list.scroll_offset,
            );
        }
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let current_width = UnicodeWidthStr::width(s);
    if current_width <= max_width {
        return s.to_string();
    }

    let ellipsis = "...";
    let ellipsis_width = UnicodeWidthStr::width(ellipsis);
    if max_width <= ellipsis_width {
        return ellipsis.chars().take(max_width).collect();
    }

    let target_width = max_width - ellipsis_width;
    let mut result = String::new();
    let mut width = 0;

    for ch in s.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);
        if width + ch_width > target_width {
            break;
        }
        result.push(ch);
        width += ch_width;
    }

    result.push_str(ellipsis);
    result
}

impl Default for SlashMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slash_menu_filters_by_label() {
        let mut state = SlashMenuState::new();
        state.show();
        state.insert_char('m');

        let entry = state.selected_entry().expect("Should have a match");
        assert_eq!(entry.command, SlashCommand::Model);
    }

    #[test]
    fn test_slash_menu_filters_by_description() {
        let mut state = SlashMenuState::new();
        state.show();
        state.insert_char('s');
        state.insert_char('e');
        state.insert_char('l');

        let entry = state.selected_entry().expect("Should have a match");
        assert_eq!(entry.command, SlashCommand::Model);
    }
}
