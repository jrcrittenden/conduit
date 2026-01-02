//! Tree view widget for repository/workspace navigation

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, StatefulWidget, Widget},
};
use uuid::Uuid;

/// A node in the tree view
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Unique identifier
    pub id: Uuid,
    /// Display label
    pub label: String,
    /// Optional suffix (e.g., branch name)
    pub suffix: Option<String>,
    /// Child nodes
    pub children: Vec<TreeNode>,
    /// Whether this node is expanded (for parent nodes)
    pub expanded: bool,
    /// Depth in the tree (0 for root nodes)
    pub depth: usize,
    /// Whether this is a leaf node (workspace) or branch node (repository)
    pub is_leaf: bool,
}

impl TreeNode {
    /// Create a new parent node (repository)
    pub fn parent(id: Uuid, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            suffix: None,
            children: Vec::new(),
            expanded: false,
            depth: 0,
            is_leaf: false,
        }
    }

    /// Create a new leaf node (workspace)
    pub fn leaf(id: Uuid, label: impl Into<String>, suffix: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            suffix: Some(suffix.into()),
            children: Vec::new(),
            expanded: false,
            depth: 1,
            is_leaf: true,
        }
    }

    /// Add a child node
    pub fn with_child(mut self, mut child: TreeNode) -> Self {
        child.depth = self.depth + 1;
        self.children.push(child);
        self
    }

    /// Toggle expanded state
    pub fn toggle_expanded(&mut self) {
        if !self.is_leaf {
            self.expanded = !self.expanded;
        }
    }

    /// Get all visible nodes as a flat list
    pub fn flatten(&self) -> Vec<&TreeNode> {
        let mut result = vec![self];
        if self.expanded {
            for child in &self.children {
                result.extend(child.flatten());
            }
        }
        result
    }

    /// Get all visible nodes as a mutable flat list
    pub fn flatten_mut(&mut self) -> Vec<&mut TreeNode> {
        let expanded = self.expanded;
        let mut result = vec![self];
        if expanded {
            // Need to use raw pointers to work around borrow checker
            let children_ptr = result[0].children.as_mut_ptr();
            let children_len = result[0].children.len();
            for i in 0..children_len {
                unsafe {
                    let child = &mut *children_ptr.add(i);
                    result.extend(child.flatten_mut());
                }
            }
        }
        result
    }
}

/// State for the tree view
#[derive(Debug, Default)]
pub struct TreeViewState {
    /// Currently selected index in the flattened view
    pub selected: usize,
    /// Scroll offset
    pub offset: usize,
}

impl TreeViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Move selection up
    pub fn select_previous(&mut self, visible_count: usize) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = visible_count.saturating_sub(1);
        }
    }

    /// Move selection down
    pub fn select_next(&mut self, visible_count: usize) {
        if visible_count > 0 {
            self.selected = (self.selected + 1) % visible_count;
        }
    }
}

/// Tree view widget
pub struct TreeView<'a> {
    /// Root nodes
    nodes: &'a [TreeNode],
    /// Block for border and title
    block: Option<Block<'a>>,
    /// Style for normal items
    style: Style,
    /// Style for selected item
    selected_style: Style,
    /// Style for expanded indicator
    expand_style: Style,
    /// Style for suffix (branch name)
    suffix_style: Style,
}

impl<'a> TreeView<'a> {
    pub fn new(nodes: &'a [TreeNode]) -> Self {
        Self {
            nodes,
            block: None,
            style: Style::default(),
            selected_style: Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
            expand_style: Style::default().fg(Color::Yellow),
            suffix_style: Style::default().fg(Color::DarkGray),
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn selected_style(mut self, style: Style) -> Self {
        self.selected_style = style;
        self
    }

    /// Get all visible nodes flattened
    fn visible_nodes(&self) -> Vec<&TreeNode> {
        self.nodes.iter().flat_map(|n| n.flatten()).collect()
    }
}

impl StatefulWidget for TreeView<'_> {
    type State = TreeViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Render block if present
        let inner = if let Some(block) = &self.block {
            let inner = block.inner(area);
            block.clone().render(area, buf);
            inner
        } else {
            area
        };

        if inner.width < 3 || inner.height < 1 {
            return;
        }

        let visible = self.visible_nodes();
        let visible_count = visible.len();

        if visible_count == 0 {
            return;
        }

        // Ensure selected is in bounds
        if state.selected >= visible_count {
            state.selected = visible_count.saturating_sub(1);
        }

        // Ensure selected item is visible
        let max_visible = inner.height as usize;
        if state.selected < state.offset {
            state.offset = state.selected;
        } else if state.selected >= state.offset + max_visible {
            state.offset = state.selected - max_visible + 1;
        }

        // Render visible items
        for (i, node) in visible.iter().enumerate().skip(state.offset).take(max_visible) {
            let y = inner.y + (i - state.offset) as u16;

            // Build the line
            let indent = "  ".repeat(node.depth);
            let expand_marker = if node.is_leaf {
                "  "
            } else if node.expanded {
                "▼ "
            } else {
                "▶ "
            };

            let mut spans = vec![
                Span::raw(indent),
                Span::styled(expand_marker, self.expand_style),
                Span::styled(&node.label, self.style),
            ];

            if let Some(suffix) = &node.suffix {
                spans.push(Span::styled(format!(" ({})", suffix), self.suffix_style));
            }

            let line = Line::from(spans);

            // Apply selection style
            let style = if i == state.selected {
                self.selected_style
            } else {
                Style::default()
            };

            // Fill background for selection
            if i == state.selected {
                for x in inner.x..inner.x + inner.width {
                    buf[(x, y)].set_style(style);
                }
            }

            // Render the line
            let line_area = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            };
            line.render(line_area, buf);

            // Re-apply selection style to ensure it covers the text
            if i == state.selected {
                for x in inner.x..inner.x + inner.width {
                    let cell = &mut buf[(x, y)];
                    cell.set_bg(self.selected_style.bg.unwrap_or(Color::Reset));
                }
            }
        }
    }
}

/// Data structure to build tree from repositories and workspaces
#[derive(Debug, Clone)]
pub struct SidebarData {
    pub nodes: Vec<TreeNode>,
}

impl Default for SidebarData {
    fn default() -> Self {
        Self::new()
    }
}

impl SidebarData {
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Add a repository with its workspaces
    pub fn add_repository(
        &mut self,
        repo_id: Uuid,
        repo_name: &str,
        workspaces: Vec<(Uuid, String, String)>, // (id, name, branch)
    ) {
        let mut repo_node = TreeNode::parent(repo_id, repo_name);

        for (ws_id, ws_name, branch) in workspaces {
            let ws_node = TreeNode::leaf(ws_id, ws_name, branch);
            repo_node = repo_node.with_child(ws_node);
        }

        self.nodes.push(repo_node);
    }

    /// Get flattened visible nodes
    pub fn visible_nodes(&self) -> Vec<&TreeNode> {
        self.nodes.iter().flat_map(|n| n.flatten()).collect()
    }

    /// Toggle expand state of a node by index
    pub fn toggle_at(&mut self, index: usize) {
        let mut current = 0;
        for node in &mut self.nodes {
            if current == index {
                node.toggle_expanded();
                return;
            }
            current += 1;
            if node.expanded {
                for child in &mut node.children {
                    if current == index {
                        child.toggle_expanded();
                        return;
                    }
                    current += 1;
                }
            }
        }
    }

    /// Get the node at a given index
    pub fn get_at(&self, index: usize) -> Option<&TreeNode> {
        self.visible_nodes().get(index).copied()
    }
}
