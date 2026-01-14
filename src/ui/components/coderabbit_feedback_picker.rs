//! CodeRabbit feedback picker dialog component

use std::collections::HashSet;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Paragraph, Widget},
};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::data::{CodeRabbitItem, CodeRabbitItemKind, CodeRabbitItemSource};

use super::{
    dialog_bg, dialog_content_area, ensure_contrast_bg, ensure_contrast_fg,
    render_minimal_scrollbar, selected_bg, text_muted, text_primary, DialogFrame, ScrollbarMetrics,
};

const DIALOG_WIDTH_PERCENT: u16 = 80;
const DIALOG_HEIGHT_PERCENT: u16 = 75;
const DIALOG_MIN_WIDTH: u16 = 60;
const DIALOG_MAX_WIDTH: u16 = 120;
const DIALOG_MIN_HEIGHT: u16 = 15;
const DIALOG_MAX_HEIGHT: u16 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRabbitFeedbackFilter {
    All,
    Actionable,
    Nitpick,
    OutsideDiff,
}

impl CodeRabbitFeedbackFilter {
    pub fn next(self) -> Self {
        match self {
            CodeRabbitFeedbackFilter::All => CodeRabbitFeedbackFilter::Actionable,
            CodeRabbitFeedbackFilter::Actionable => CodeRabbitFeedbackFilter::Nitpick,
            CodeRabbitFeedbackFilter::Nitpick => CodeRabbitFeedbackFilter::OutsideDiff,
            CodeRabbitFeedbackFilter::OutsideDiff => CodeRabbitFeedbackFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CodeRabbitFeedbackFilter::All => "All",
            CodeRabbitFeedbackFilter::Actionable => "Actionable",
            CodeRabbitFeedbackFilter::Nitpick => "Nitpicks",
            CodeRabbitFeedbackFilter::OutsideDiff => "Outside diff",
        }
    }

    pub fn matches(self, item: &CodeRabbitItem) -> bool {
        match self {
            CodeRabbitFeedbackFilter::All => true,
            CodeRabbitFeedbackFilter::Actionable => item.kind == CodeRabbitItemKind::Actionable,
            CodeRabbitFeedbackFilter::Nitpick => item.kind == CodeRabbitItemKind::Nitpick,
            CodeRabbitFeedbackFilter::OutsideDiff => item.kind == CodeRabbitItemKind::OutsideDiff,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeRabbitFeedbackPickerState {
    pub visible: bool,
    pub round_id: Option<Uuid>,
    pub pr_number: Option<i64>,
    pub head_sha: Option<String>,
    pub items: Vec<CodeRabbitItem>,
    pub list: super::SearchableListState,
    pub selected: HashSet<usize>,
    pub filter: CodeRabbitFeedbackFilter,
}

impl CodeRabbitFeedbackPickerState {
    pub fn new() -> Self {
        Self {
            visible: false,
            round_id: None,
            pr_number: None,
            head_sha: None,
            items: Vec::new(),
            list: super::SearchableListState::new(12),
            selected: HashSet::new(),
            filter: CodeRabbitFeedbackFilter::All,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(
        &mut self,
        round_id: Uuid,
        pr_number: i64,
        head_sha: String,
        items: Vec<CodeRabbitItem>,
    ) {
        self.visible = true;
        self.round_id = Some(round_id);
        self.pr_number = Some(pr_number);
        self.head_sha = Some(head_sha);
        self.items = items;
        self.filter = CodeRabbitFeedbackFilter::All;
        self.list.reset();
        self.selected.clear();
        for idx in 0..self.items.len() {
            self.selected.insert(idx);
        }
        self.apply_filter();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.round_id = None;
        self.pr_number = None;
        self.head_sha = None;
        self.items.clear();
        self.selected.clear();
        self.list.reset();
    }

    pub fn apply_filter(&mut self) {
        let filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| self.filter.matches(item))
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        self.list.set_filtered(filtered);
    }

    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
        self.apply_filter();
    }

    pub fn toggle_selected(&mut self) {
        if let Some(item_idx) = self.selected_item_index() {
            if self.selected.contains(&item_idx) {
                self.selected.remove(&item_idx);
            } else {
                self.selected.insert(item_idx);
            }
        }
    }

    pub fn select_all_filtered(&mut self) {
        for idx in &self.list.filtered {
            self.selected.insert(*idx);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    pub fn selected_item_index(&self) -> Option<usize> {
        if self.list.filtered.is_empty() {
            None
        } else {
            Some(self.list.filtered[self.list.selected])
        }
    }

    pub fn selected_items(&self) -> Vec<CodeRabbitItem> {
        let mut items = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
            if self.selected.contains(&idx) {
                items.push(item.clone());
            }
        }
        items
    }
}

impl Default for CodeRabbitFeedbackPickerState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
struct PickerLayout {
    header_area: Rect,
    list_area: Rect,
    scrollbar_area: Rect,
}

fn calculate_layout(area: Rect, visible_rows: usize) -> Option<(Rect, PickerLayout)> {
    let dialog_width = (area.width * DIALOG_WIDTH_PERCENT / 100)
        .clamp(DIALOG_MIN_WIDTH, DIALOG_MAX_WIDTH)
        .min(area.width.saturating_sub(4));
    let dialog_height = (area.height * DIALOG_HEIGHT_PERCENT / 100)
        .clamp(DIALOG_MIN_HEIGHT, DIALOG_MAX_HEIGHT)
        .min(area.height.saturating_sub(2));

    let dialog_x = area.width.saturating_sub(dialog_width) / 2;
    let dialog_y = area.height.saturating_sub(dialog_height) / 2;

    let dialog_area = Rect {
        x: dialog_x,
        y: dialog_y,
        width: dialog_width,
        height: dialog_height,
    };

    let inner = dialog_content_area(dialog_area);
    if inner.height < 2 || inner.width < 10 {
        return None;
    }

    let header_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };

    let list_height = inner.height.saturating_sub(1);
    if list_height == 0 {
        return None;
    }

    let list_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width.saturating_sub(1),
        height: list_height,
    };

    let scrollbar_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y + 1,
        width: 1,
        height: list_height,
    };

    let max_visible = visible_rows.min(list_height as usize).max(1);
    Some((
        dialog_area,
        PickerLayout {
            header_area,
            list_area: Rect {
                height: max_visible as u16,
                ..list_area
            },
            scrollbar_area,
        },
    ))
}

pub struct CodeRabbitFeedbackPicker;

impl CodeRabbitFeedbackPicker {
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &CodeRabbitFeedbackPickerState) {
        if !state.visible {
            return;
        }

        let instructions = vec![
            ("Space", "toggle"),
            ("A", "all"),
            ("N", "none"),
            ("F", "filter"),
            ("Enter", "send"),
            ("Esc", "close"),
        ];
        let dialog = DialogFrame::new("CodeRabbit Feedback", 0, 0).instructions(instructions);

        let max_visible = state.list.max_visible;
        let Some((dialog_area, layout)) = calculate_layout(area, max_visible) else {
            return;
        };

        dialog.render(dialog_area, buf);

        let total = state.items.len();
        let selected = state.selected_count();
        let pr_label = state
            .pr_number
            .map(|num| format!("PR #{}", num))
            .unwrap_or_else(|| "PR".to_string());
        let header_text = format!(
            "{} | Filter: {} | Selected: {}/{}",
            pr_label,
            state.filter.label(),
            selected,
            total
        );
        Paragraph::new(header_text)
            .style(Style::default().fg(text_primary()).bg(dialog_bg()))
            .render(layout.header_area, buf);

        let mut lines = Vec::new();
        let start = state.list.scroll_offset;
        let end = (start + state.list.max_visible).min(state.list.filtered.len());
        let visible_indices = &state.list.filtered[start..end];

        for (row, item_idx) in visible_indices.iter().enumerate() {
            let item = &state.items[*item_idx];
            let selected_mark = if state.selected.contains(item_idx) {
                "[x]"
            } else {
                "[ ]"
            };
            let kind_label = match item.kind {
                CodeRabbitItemKind::Actionable => "A",
                CodeRabbitItemKind::Nitpick => "N",
                CodeRabbitItemKind::OutsideDiff => "O",
            };
            let location = format_location(item);
            let summary = summarize_body(&item.body);
            let mut line = format!(
                "{} {} {} - {}",
                selected_mark, kind_label, location, summary
            );
            let max_width = layout.list_area.width as usize;
            if max_width > 0 {
                line = truncate_to_width(&line, max_width);
            }

            let is_selected = state.list.selected == start + row;
            let row_style = if is_selected {
                let bg = ensure_contrast_bg(selected_bg(), dialog_bg(), 1.3);
                Style::default()
                    .bg(bg)
                    .fg(ensure_contrast_fg(text_primary(), bg, 4.5))
            } else {
                Style::default().fg(text_primary()).bg(dialog_bg())
            };
            lines.push(Line::styled(line, row_style));
        }

        let list_widget = Paragraph::new(lines).style(Style::default().bg(dialog_bg()));
        list_widget.render(layout.list_area, buf);

        let scrollbar_metrics = ScrollbarMetrics {
            area: layout.scrollbar_area,
            total: state.list.filtered.len(),
            visible: state.list.max_visible,
        };
        if scrollbar_metrics.area.height > 0 {
            render_minimal_scrollbar(
                scrollbar_metrics.area,
                buf,
                scrollbar_metrics.total.max(1),
                scrollbar_metrics.visible,
                state.list.scroll_offset,
            );
        }

        if state.items.is_empty() {
            let empty_msg = Paragraph::new("No CodeRabbit feedback items.")
                .style(Style::default().fg(text_muted()).bg(dialog_bg()));
            empty_msg.render(layout.list_area, buf);
        }
    }
}

fn format_location(item: &CodeRabbitItem) -> String {
    if let Some(path) = item.file_path.as_ref() {
        let mut location = path.clone();
        if let Some(line_start) = item.line_start {
            if let Some(line_end) = item.line_end {
                if line_end != line_start {
                    location.push_str(&format!(":{}-{}", line_start, line_end));
                } else {
                    location.push_str(&format!(":{}", line_start));
                }
            } else {
                location.push_str(&format!(":{}", line_start));
            }
        }
        return location;
    }
    if let Some(section) = item.section.as_ref() {
        return section.clone();
    }
    format_source(item.source)
}

fn format_source(source: CodeRabbitItemSource) -> String {
    match source {
        CodeRabbitItemSource::ReviewComment => "review-comment".to_string(),
        CodeRabbitItemSource::IssueComment => "issue-comment".to_string(),
        CodeRabbitItemSource::Review => "review".to_string(),
    }
}

fn summarize_body(body: &str) -> String {
    let line = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        "<no comment>".to_string()
    } else {
        line.to_string()
    }
}

fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let ch_width = UnicodeWidthStr::width(ch.to_string().as_str());
        if width + ch_width >= max_width.saturating_sub(1) {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    if max_width > 3 {
        result.push_str("...");
    }
    result
}
