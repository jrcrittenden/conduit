use std::time::{Duration, Instant};

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

/// Processing words used by Claude Code (subset of ~90 words)
const PROCESSING_WORDS: &[&str] = &[
    "Accomplishing",
    "Baking",
    "Booping",
    "Brewing",
    "Calculating",
    "Cerebrating",
    "Churning",
    "Clauding",
    "Cogitating",
    "Combobulating",
    "Computing",
    "Concocting",
    "Conjuring",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Crunching",
    "Deciphering",
    "Deliberating",
    "Divining",
    "Enchanting",
    "Finagling",
    "Forging",
    "Frolicking",
    "Generating",
    "Hatching",
    "Hustling",
    "Ideating",
    "Imagining",
    "Incubating",
    "Inferring",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Mulling",
    "Musing",
    "Noodling",
    "Percolating",
    "Pondering",
    "Pontificating",
    "Processing",
    "Puzzling",
    "Ruminating",
    "Scheming",
    "Shimmying",
    "Simmering",
    "Spelunking",
    "Spinning",
    "Stewing",
    "Sussing",
    "Synthesizing",
    "Thinking",
    "Tinkering",
    "Transmuting",
    "Unfurling",
    "Vibing",
    "Wandering",
    "Whirring",
    "Wizarding",
    "Working",
    "Wrangling",
];

/// Spinner animation frames (Claude Code style)
const SPINNER_FRAMES: &[&str] = &["·", "✢", "✳", "∗", "✻", "✽"];

/// Current processing state
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessingState {
    Thinking,
    ToolUse(String),
    Reading,
    Writing,
    Searching,
}

impl ProcessingState {
    pub fn as_str(&self) -> &str {
        match self {
            ProcessingState::Thinking => "thinking",
            ProcessingState::ToolUse(name) => name,
            ProcessingState::Reading => "reading",
            ProcessingState::Writing => "writing",
            ProcessingState::Searching => "searching",
        }
    }
}

/// Thinking indicator that shows while agent is processing
pub struct ThinkingIndicator {
    /// Current processing word
    word: &'static str,
    /// Spinner frame index
    spinner_frame: usize,
    /// When processing started
    start_time: Instant,
    /// Tokens received so far
    tokens: usize,
    /// Current processing state
    state: ProcessingState,
}

impl ThinkingIndicator {
    /// Create a new thinking indicator with a random word
    pub fn new() -> Self {
        Self {
            word: Self::random_word(),
            spinner_frame: 0,
            start_time: Instant::now(),
            tokens: 0,
            state: ProcessingState::Thinking,
        }
    }

    /// Get a random processing word
    fn random_word() -> &'static str {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;

        // Simple pseudo-random selection based on current time
        let mut hasher = DefaultHasher::new();
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .hash(&mut hasher);
        let index = (hasher.finish() as usize) % PROCESSING_WORDS.len();
        PROCESSING_WORDS[index]
    }

    /// Advance the spinner animation
    pub fn tick(&mut self) {
        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }

    /// Add tokens to the count
    pub fn add_tokens(&mut self, count: usize) {
        self.tokens += count;
    }

    /// Set the current processing state
    pub fn set_state(&mut self, state: ProcessingState) {
        self.state = state;
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Reset with a new random word
    pub fn reset(&mut self) {
        self.word = Self::random_word();
        self.spinner_frame = 0;
        self.start_time = Instant::now();
        self.tokens = 0;
        self.state = ProcessingState::Thinking;
    }

    /// Render as a Line for display in chat view
    pub fn render(&self) -> Line<'static> {
        let elapsed = self.elapsed();
        let seconds = elapsed.as_secs();

        let spinner = SPINNER_FRAMES[self.spinner_frame];
        let word = self.word;
        let state = self.state.as_str();

        // Format: ✳ Tinkering... (esc to interrupt · 6s · ↓ 49 tokens · thinking)
        Line::from(vec![
            Span::styled(
                format!("{} ", spinner),
                Style::default().fg(Color::Rgb(255, 165, 0)), // Orange
            ),
            Span::styled(
                format!("{}... ", word),
                Style::default().fg(Color::Rgb(255, 165, 0)), // Orange
            ),
            Span::styled("(", Style::default().fg(Color::DarkGray)),
            Span::styled("esc", Style::default().fg(Color::Gray)),
            Span::styled(" to interrupt · ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}s", seconds), Style::default().fg(Color::Gray)),
            Span::styled(" · ↓ ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", self.tokens), Style::default().fg(Color::Gray)),
            Span::styled(" tokens · ", Style::default().fg(Color::DarkGray)),
            Span::styled(state.to_string(), Style::default().fg(Color::Gray)),
            Span::styled(")", Style::default().fg(Color::DarkGray)),
        ])
    }
}

impl Default for ThinkingIndicator {
    fn default() -> Self {
        Self::new()
    }
}
