//! Project picker dialog component with fuzzy search

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Paragraph, Widget},
};
use std::path::PathBuf;

use super::{DialogFrame, InstructionBar, TextInputState};

/// A project entry (directory with .git)
#[derive(Debug, Clone)]
pub struct ProjectEntry {
    /// Display name (folder name)
    pub name: String,
    /// Full path to the project
    pub path: PathBuf,
}

/// State for the project picker dialog
#[derive(Debug, Clone)]
pub struct ProjectPickerState {
    /// Whether the dialog is visible
    pub visible: bool,
    /// Search/filter input
    pub search: TextInputState,
    /// All projects found in base directory
    pub projects: Vec<ProjectEntry>,
    /// Indices of projects matching the search filter
    pub filtered: Vec<usize>,
    /// Currently selected index in the filtered list
    pub selected: usize,
    /// Base directory being scanned
    pub base_dir: PathBuf,
    /// Maximum visible items in the list
    pub max_visible: usize,
    /// Scroll offset for the list
    pub scroll_offset: usize,
}

impl Default for ProjectPickerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectPickerState {
    pub fn new() -> Self {
        Self {
            visible: false,
            search: TextInputState::new(),
            projects: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            base_dir: PathBuf::new(),
            max_visible: 10,
            scroll_offset: 0,
        }
    }

    /// Show the picker with projects from the given base directory
    pub fn show(&mut self, base_dir: PathBuf) {
        self.visible = true;
        self.base_dir = base_dir;
        self.search.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.scan_directory();
        self.filter();
    }

    /// Hide the dialog
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Scan the base directory for git projects
    pub fn scan_directory(&mut self) {
        self.projects.clear();

        let Ok(entries) = std::fs::read_dir(&self.base_dir) else {
            return;
        };

        let mut projects: Vec<ProjectEntry> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.path().join(".git").exists())
            .map(|e| ProjectEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
            })
            .collect();

        // Sort alphabetically by name
        projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.projects = projects;
    }

    /// Filter projects based on search string
    pub fn filter(&mut self) {
        let query = self.search.value().to_lowercase();
        self.filtered = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                if query.is_empty() {
                    true
                } else {
                    p.name.to_lowercase().contains(&query)
                }
            })
            .map(|(i, _)| i)
            .collect();

        // Reset selection if out of bounds
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
    }

    // Delegate search input methods
    pub fn insert_char(&mut self, c: char) {
        self.search.insert_char(c);
        self.filter();
    }

    pub fn delete_char(&mut self) {
        self.search.delete_char();
        self.filter();
    }

    pub fn delete_forward(&mut self) {
        self.search.delete_forward();
        self.filter();
    }

    pub fn move_cursor_left(&mut self) {
        self.search.move_left();
    }

    pub fn move_cursor_right(&mut self) {
        self.search.move_right();
    }

    pub fn move_cursor_start(&mut self) {
        self.search.move_start();
    }

    pub fn move_cursor_end(&mut self) {
        self.search.move_end();
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.filter();
    }

    /// Select previous item
    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() && self.selected > 0 {
            self.selected -= 1;
            // Adjust scroll if needed
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            }
        }
    }

    /// Select next item
    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() && self.selected < self.filtered.len() - 1 {
            self.selected += 1;
            // Adjust scroll if needed
            if self.selected >= self.scroll_offset + self.max_visible {
                self.scroll_offset = self.selected - self.max_visible + 1;
            }
        }
    }

    /// Get the currently selected project
    pub fn selected_project(&self) -> Option<&ProjectEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.projects.get(idx))
    }

    /// Check if dialog is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Check if there are no projects found
    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// Project picker dialog widget
pub struct ProjectPicker;

impl ProjectPicker {
    pub fn new() -> Self {
        Self
    }

    /// Render the dialog
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &ProjectPickerState) {
        if !state.visible {
            return;
        }

        // Calculate dialog size
        let list_height = state.max_visible.min(state.filtered.len().max(1)) as u16;
        let dialog_height = 7 + list_height; // header + search + separator + list + footer

        // Render dialog frame
        let frame = DialogFrame::new("Select Project", 60, dialog_height);
        let inner = frame.render(area, buf);

        // Layout inside dialog
        let chunks = Layout::vertical([
            Constraint::Length(1), // Search label
            Constraint::Length(1), // Search input
            Constraint::Length(1), // Separator
            Constraint::Min(1),    // Project list
            Constraint::Length(1), // Spacing
            Constraint::Length(1), // Instructions
        ])
        .split(inner);

        // Render search with placeholder
        let search_display = if state.search.is_empty() {
            "Search: (type to filter)".to_string()
        } else {
            format!("Search: {}", state.search.value())
        };
        let search_style = if state.search.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let search_label = Paragraph::new(search_display).style(search_style);
        search_label.render(chunks[0], buf);

        // Render cursor in search field
        if !state.search.is_empty() || state.search.cursor > 0 {
            let cursor_x = chunks[0].x + 8 + state.search.cursor as u16; // "Search: " is 8 chars
            if cursor_x < chunks[0].x + chunks[0].width {
                use ratatui::style::Modifier;
                buf[(cursor_x, chunks[0].y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        }

        // Render separator
        let separator = "─".repeat(inner.width as usize);
        let sep_paragraph =
            Paragraph::new(separator).style(Style::default().fg(Color::DarkGray));
        sep_paragraph.render(chunks[2], buf);

        // Render project list
        let list_area = chunks[3];
        if state.filtered.is_empty() {
            let empty_msg = if state.projects.is_empty() {
                "No git projects found in this directory"
            } else {
                "No projects match your search"
            };
            let empty = Paragraph::new(empty_msg)
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            empty.render(list_area, buf);
        } else {
            // Render visible items
            let visible_count = list_area.height as usize;
            for (i, &project_idx) in state
                .filtered
                .iter()
                .skip(state.scroll_offset)
                .take(visible_count)
                .enumerate()
            {
                let project = &state.projects[project_idx];
                let is_selected = state.scroll_offset + i == state.selected;

                let y = list_area.y + i as u16;
                if y >= list_area.y + list_area.height {
                    break;
                }

                // Format: "> name          path"
                let prefix = if is_selected { "> " } else { "  " };
                let name = &project.name;

                // Calculate path display (shortened)
                let path_str = project
                    .path
                    .to_string_lossy()
                    .replace(dirs::home_dir().unwrap_or_default().to_string_lossy().as_ref(), "~");

                let available_width = list_area.width as usize;
                let name_width = 20.min(available_width / 2);
                let path_width = available_width.saturating_sub(name_width + 4);

                let name_display = if name.len() > name_width {
                    format!("{}...", &name[..name_width - 3])
                } else {
                    format!("{:width$}", name, width = name_width)
                };

                let path_display = if path_str.len() > path_width {
                    format!("...{}", &path_str[path_str.len() - path_width + 3..])
                } else {
                    path_str.to_string()
                };

                let line_text = format!("{}{} {}", prefix, name_display, path_display);

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(100, 180, 220))
                } else {
                    Style::default().fg(Color::White)
                };

                // Render the line
                for (j, c) in line_text.chars().enumerate() {
                    if j < list_area.width as usize {
                        buf[(list_area.x + j as u16, y)].set_char(c).set_style(style);
                    }
                }
                // Fill rest of line with style for selected item
                if is_selected {
                    for j in line_text.len()..list_area.width as usize {
                        buf[(list_area.x + j as u16, y)].set_style(style);
                    }
                }
            }
        }

        // Render instructions
        let instructions = InstructionBar::new(vec![
            ("↑↓", "Navigate"),
            ("Enter", "Select"),
            ("a", "Custom path"),
            ("Esc", "Cancel"),
        ]);
        instructions.render(chunks[5], buf);
    }
}

impl Default for ProjectPicker {
    fn default() -> Self {
        Self::new()
    }
}
