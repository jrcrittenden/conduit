//! Knight Rider style bidirectional scanner animation.
//!
//! A scanning bar animation with gradient trail that bounces back and forth.

use ratatui::{style::Style, text::Span};

use super::{
    SPINNER_ACTIVE, SPINNER_INACTIVE, SPINNER_TRAIL_1, SPINNER_TRAIL_2, SPINNER_TRAIL_3,
    SPINNER_TRAIL_4,
};

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
    holding: bool,
    /// Frames to hold at end (right side)
    hold_end_frames: usize,
    /// Frames to hold at start (left side)
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
            holding: false,
            hold_end_frames: 9,    // Hold at right end
            hold_start_frames: 30, // Hold at left end (longer pause)
            trail_length: 4,       // Trail extends 4 positions behind active
        }
    }

    /// Advance animation by one tick
    pub fn tick(&mut self) {
        // If holding at endpoint, count down
        if self.holding {
            if self.hold_counter > 0 {
                self.hold_counter -= 1;
                return;
            }
            // Done holding, continue movement
            self.holding = false;
        }

        // Move position - continue past visible bounds to let trail exit
        let width = self.width as isize;
        if self.forward {
            // Moving right: continue until trail has fully exited right side
            let exit_position = width - 1 + self.trail_length;
            if self.position >= exit_position {
                // Trail has exited, start holding then reverse
                self.holding = true;
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
                self.holding = true;
                self.hold_counter = self.hold_start_frames;
                self.forward = true;
            } else {
                self.position -= 1;
            }
        }
    }

    /// Get the color for a position based on distance from active position
    fn color_for_distance(&self, distance: usize) -> ratatui::style::Color {
        match distance {
            0 => SPINNER_ACTIVE,
            1 => SPINNER_TRAIL_1,
            2 => SPINNER_TRAIL_2,
            3 => SPINNER_TRAIL_3,
            4 => SPINNER_TRAIL_4,
            _ => SPINNER_INACTIVE,
        }
    }

    /// Render the spinner as a vector of styled spans
    pub fn render(&self) -> Vec<Span<'static>> {
        let active_char = "■";
        let inactive_char = "⬝";

        let mut spans = Vec::with_capacity(self.width);

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

            let color = self.color_for_distance(distance);
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
        self.holding = false;
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
        assert!(!spinner.holding); // Not holding yet, trail still visible

        spinner.tick();
        assert_eq!(spinner.position, 5);

        spinner.tick();
        assert_eq!(spinner.position, 6);

        spinner.tick();
        assert_eq!(spinner.position, 7); // exit_position = 3 + 4 = 7

        // Now should start holding and reverse
        spinner.tick();
        assert!(spinner.holding);
        assert!(!spinner.forward);
    }

    #[test]
    fn test_render_output() {
        let spinner = KnightRiderSpinner::with_width(4);
        let spans = spinner.render();
        assert_eq!(spans.len(), 4);
    }
}
