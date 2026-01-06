use ratatui::{
    style::{Color, Style},
    text::Span,
};

/// Animated spinner for loading states
pub struct Spinner {
    frames: &'static [&'static str],
    tick: usize,
}

impl Spinner {
    /// Create a new spinner with default frames
    pub fn new() -> Self {
        Self {
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            tick: 0,
        }
    }

    /// Create a dots spinner
    pub fn dots() -> Self {
        Self {
            frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            tick: 0,
        }
    }

    /// Create a simple spinner
    pub fn simple() -> Self {
        Self {
            frames: &["|", "/", "-", "\\"],
            tick: 0,
        }
    }

    /// Create a bounce spinner
    pub fn bounce() -> Self {
        Self {
            frames: &["⠁", "⠂", "⠄", "⠂"],
            tick: 0,
        }
    }

    /// Create a growing spinner
    pub fn growing() -> Self {
        Self {
            frames: &[
                "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂",
            ],
            tick: 0,
        }
    }

    /// Advance to the next frame
    pub fn tick(&mut self) {
        self.tick = (self.tick + 1) % self.frames.len();
    }

    /// Get current frame
    pub fn frame(&self) -> &'static str {
        self.frames[self.tick % self.frames.len()]
    }

    /// Get current frame as a styled span
    pub fn span(&self, color: Color) -> Span<'static> {
        Span::styled(self.frame().to_string(), Style::default().fg(color))
    }

    /// Get current frame with label
    pub fn with_label(&self, label: &str, color: Color) -> Vec<Span<'static>> {
        vec![
            Span::styled(self.frame().to_string(), Style::default().fg(color)),
            Span::raw(" "),
            Span::styled(label.to_string(), Style::default().fg(color)),
        ]
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}
