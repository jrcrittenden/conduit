//! Knight Rider style bidirectional scanner animation.
//!
//! A scanning bar animation with gradient trail that bounces back and forth.

use ratatui::{style::Color, style::Style, text::Span};

use super::{
    SPINNER_ACTIVE, SPINNER_INACTIVE, SPINNER_TRAIL_1, SPINNER_TRAIL_2, SPINNER_TRAIL_3,
    SPINNER_TRAIL_4,
};

/// Which endpoint we're holding at
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldPosition {
    /// Holding at left/start (with pulsation)
    Left,
    /// Holding at right/end (brief pause)
    Right,
}

/// Knight Rider style bidirectional scanner animation
pub struct KnightRiderSpinner {
    /// Current active position (can be negative or beyond width to let trail exit)
    position: isize,
    /// Direction (true = forward/right, false = backward/left)
    forward: bool,
    /// Bar width (number of characters)
    width: usize,
    /// Hold frame counter
    hold_counter: usize,
    /// Currently holding at endpoint
    holding: Option<HoldPosition>,
    /// Frames to hold at end (right side)
    hold_end_frames: usize,
    /// Frames to hold at start (left side) - includes pulsation time
    hold_start_frames: usize,
    /// Trail length (how many positions the trail extends)
    trail_length: isize,
}

impl KnightRiderSpinner {
    /// Create a new spinner with default width of 8
    pub fn new() -> Self {
        Self::with_width(8)
    }

    /// Create a new spinner with specified width
    pub fn with_width(width: usize) -> Self {
        Self {
            position: 0,
            forward: true,
            width: width.max(3), // Minimum width of 3
            hold_counter: 0,
            holding: None,
            hold_end_frames: 12,   // Hold at right end (brief pause while hidden)
            hold_start_frames: 60, // Hold at left end (longer pause with pulsation)
            trail_length: 4,       // Trail extends 4 positions behind active
        }
    }

    /// Advance animation by one tick
    pub fn tick(&mut self) {
        // If holding at endpoint, count down
        if self.holding.is_some() {
            if self.hold_counter > 0 {
                self.hold_counter -= 1;
                return;
            }
            // Done holding, continue movement
            self.holding = None;
        }

        // Move position - continue past visible bounds to let trail exit
        let width = self.width as isize;
        if self.forward {
            // Moving right: continue until trail has fully exited right side
            let exit_position = width - 1 + self.trail_length;
            if self.position >= exit_position {
                // Trail has exited, start holding then reverse
                self.holding = Some(HoldPosition::Right);
                self.hold_counter = self.hold_end_frames;
                self.forward = false;
            } else {
                self.position += 1;
            }
        } else {
            // Moving left: continue until trail has fully exited left side
            let exit_position = -self.trail_length;
            if self.position <= exit_position {
                // Trail has exited, start holding then reverse
                self.holding = Some(HoldPosition::Left);
                self.hold_counter = self.hold_start_frames;
                self.forward = true;
            } else {
                self.position -= 1;
            }
        }
    }

    /// Get the color for a position based on distance from active position
    fn color_for_distance(&self, distance: usize) -> Color {
        match distance {
            0 => SPINNER_ACTIVE,
            1 => SPINNER_TRAIL_1,
            2 => SPINNER_TRAIL_2,
            3 => SPINNER_TRAIL_3,
            4 => SPINNER_TRAIL_4,
            _ => SPINNER_INACTIVE,
        }
    }

    /// Calculate pulse factor (0.0 to 1.0) for left-side hold pulsation
    /// Uses sine wave for smooth breathing effect
    fn pulse_factor(&self) -> f64 {
        if let Some(HoldPosition::Left) = self.holding {
            // Progress through hold (0.0 at start, 1.0 at end)
            let progress = 1.0 - (self.hold_counter as f64 / self.hold_start_frames.max(1) as f64);
            // Use sine wave for smooth pulse: starts dim, brightens, dims again
            // sin(0) = 0, sin(π/2) = 1, sin(π) = 0
            let phase = progress * std::f64::consts::PI;
            phase.sin()
        } else {
            1.0 // No pulsation, full brightness
        }
    }

    /// Dim a color by a factor (0.0 = black, 1.0 = original)
    fn dim_color(&self, color: Color, factor: f64) -> Color {
        match color {
            Color::Rgb(r, g, b) => {
                // Interpolate towards very dark (almost black)
                let min_brightness = 0.15; // Don't go completely black
                let adjusted_factor = min_brightness + (1.0 - min_brightness) * factor;
                Color::Rgb(
                    (r as f64 * adjusted_factor) as u8,
                    (g as f64 * adjusted_factor) as u8,
                    (b as f64 * adjusted_factor) as u8,
                )
            }
            _ => color, // Non-RGB colors pass through unchanged
        }
    }

    /// Render the spinner as a vector of styled spans
    pub fn render(&self) -> Vec<Span<'static>> {
        let active_char = "■";
        let inactive_char = "⬝";

        let mut spans = Vec::with_capacity(self.width);
        let pulse = self.pulse_factor();

        for i in 0..self.width {
            let i_signed = i as isize;

            // Calculate distance from active position
            // Trail follows behind the direction of movement
            let distance = if i_signed == self.position {
                0
            } else if self.forward {
                // Moving right: trail is to the left (positions < active)
                if i_signed < self.position {
                    (self.position - i_signed) as usize
                } else {
                    usize::MAX // No trail ahead
                }
            } else {
                // Moving left: trail is to the right (positions > active)
                if i_signed > self.position {
                    (i_signed - self.position) as usize
                } else {
                    usize::MAX // No trail ahead
                }
            };

            let base_color = self.color_for_distance(distance);

            // Apply pulsation during left hold (all squares are inactive, so all pulse)
            let color = if self.holding == Some(HoldPosition::Left) {
                self.dim_color(SPINNER_INACTIVE, pulse)
            } else {
                base_color
            };

            let ch = if distance <= 4 {
                active_char
            } else {
                inactive_char
            };

            spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }

        spans
    }

    /// Reset spinner to initial state
    pub fn reset(&mut self) {
        self.position = 0;
        self.forward = true;
        self.hold_counter = 0;
        self.holding = None;
    }
}

impl Default for KnightRiderSpinner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_creation() {
        let spinner = KnightRiderSpinner::new();
        assert_eq!(spinner.width, 8);
        assert_eq!(spinner.position, 0);
        assert!(spinner.forward);
    }

    #[test]
    fn test_spinner_movement() {
        let mut spinner = KnightRiderSpinner::with_width(4);

        // Should start at position 0
        assert_eq!(spinner.position, 0);

        // Move forward through visible area
        spinner.tick();
        assert_eq!(spinner.position, 1);

        spinner.tick();
        assert_eq!(spinner.position, 2);

        spinner.tick();
        assert_eq!(spinner.position, 3);

        // Continue past visible area to let trail exit (trail_length = 4)
        spinner.tick();
        assert_eq!(spinner.position, 4);
        assert!(spinner.holding.is_none()); // Not holding yet, trail still visible

        spinner.tick();
        assert_eq!(spinner.position, 5);

        spinner.tick();
        assert_eq!(spinner.position, 6);

        spinner.tick();
        assert_eq!(spinner.position, 7); // exit_position = 3 + 4 = 7

        // Now should start holding at right side and reverse
        spinner.tick();
        assert_eq!(spinner.holding, Some(HoldPosition::Right));
        assert!(!spinner.forward);
    }

    #[test]
    fn test_render_output() {
        let spinner = KnightRiderSpinner::with_width(4);
        let spans = spinner.render();
        assert_eq!(spans.len(), 4);
    }
}
