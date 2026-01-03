use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

/// Width of the shimmer wave (smaller = more granular/tighter)
const SHIMMER_WIDTH: f32 = 1.5;

/// Linear interpolation between two u8 values
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t) as u8
}

/// Splash screen with animated pipe flow
pub struct SplashScreen {
    /// Animation position along the forward path (0.0 to 1.0)
    /// 0.00 → 0.25: Forward through pipes (agents → junction → entry)
    /// 0.25 → 0.50: Split around title box (top/bottom meet on right)
    /// 0.50 → 1.00: Pause before repeating
    animation_position: f32,
    /// Whether to show first-time user mode (Add Project button only)
    pub first_time_mode: bool,
}

impl SplashScreen {
    pub fn new() -> Self {
        Self {
            animation_position: 0.0,
            first_time_mode: false,
        }
    }

    /// Advance the animation
    pub fn tick(&mut self) {
        self.animation_position += 0.008; // Slower, smoother animation
        if self.animation_position > 1.0 {
            self.animation_position = 0.0;
        }
    }

    /// Get wave position for forward path through pipes/junction (agents → entry)
    fn forward_wave_pos(&self) -> f32 {
        if self.animation_position <= 0.25 {
            // Map 0.0-0.25 to path positions 0.0-0.5
            self.animation_position / 0.25 * 0.5
        } else {
            -1.0
        }
    }

    /// Get wave position for title box paths (forward only)
    /// Returns 0.0 at entry, 0.5 at meet point on right
    /// Returns -1.0 if not in title box phase
    fn title_wave_pos(&self) -> f32 {
        if self.animation_position >= 0.25 && self.animation_position <= 0.50 {
            // Forward: entry (0.0) to meet point (0.5)
            let progress = (self.animation_position - 0.25) / 0.25;
            progress * 0.5
        } else {
            -1.0
        }
    }

    /// Calculate shimmer brightness for a position along a pipe's path
    /// `path_pos`: normalized position along the path (0.0 = start, 1.0 = end)
    /// `wave_pos`: current wave position (0.0 to 1.0)
    fn shimmer_brightness(&self, path_pos: f32, wave_pos: f32) -> f32 {
        // Distance from wave center (no wrap-around - wave travels one direction)
        let dist = path_pos - wave_pos;

        // Only show shimmer if wave has reached this position and is nearby
        if dist < -0.15 || dist > 0.15 {
            return 0.2; // Ambient level
        }

        // Gaussian-like falloff centered on wave position
        let wave_dist = dist / (SHIMMER_WIDTH / 10.0);
        let highlight = (-wave_dist * wave_dist).exp();

        (0.2 + highlight * 0.8).clamp(0.0, 1.0)
    }

    /// Get shimmer color for pipe/junction segments (path_pos 0.0-0.5)
    fn pipe_junction_color(&self, path_pos: f32, base_color: Color) -> Color {
        let mut brightness: f32 = 0.2; // Ambient

        // Check forward path
        let forward_wave = self.forward_wave_pos();
        if forward_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, forward_wave));
        }

        self.apply_brightness(base_color, brightness)
    }

    /// Get shimmer color for top pipe at given path position
    fn top_pipe_color(&self, path_pos: f32, base_color: Color) -> Color {
        self.pipe_junction_color(path_pos, base_color)
    }

    /// Get shimmer color for bottom pipe at given path position (synchronized with top)
    fn bottom_pipe_color(&self, path_pos: f32, base_color: Color) -> Color {
        self.pipe_junction_color(path_pos, base_color)
    }

    /// Get shimmer color for junction (shared path between pipes and title box)
    fn junction_color(&self, path_pos: f32, base_color: Color) -> Color {
        let mut brightness: f32 = 0.2; // Ambient

        // Check forward path
        let forward_wave = self.forward_wave_pos();
        if forward_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, forward_wave));
        }

        // Check title box path (for segments at 0.5, which is both entry and part of title box)
        let title_wave = self.title_wave_pos();
        if title_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, title_wave));
        }

        self.apply_brightness(base_color, brightness)
    }

    /// Get shimmer color for title box TOP path segments
    /// Top path: entry → up left side → across top → down right side → meet point
    /// path_pos: 0.0 = entry, 0.5 = meet point on right
    fn title_top_color(&self, path_pos: f32, base_color: Color) -> Color {
        let mut brightness: f32 = 0.2;

        let title_wave = self.title_wave_pos();
        if title_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, title_wave));
        }

        self.apply_brightness(base_color, brightness)
    }

    /// Get shimmer color for title box BOTTOM path segments
    /// Bottom path: entry → down left side → across bottom → up right side → meet point
    /// path_pos: 0.0 = entry, 0.5 = meet point on right
    fn title_bottom_color(&self, path_pos: f32, base_color: Color) -> Color {
        let mut brightness: f32 = 0.2;

        let title_wave = self.title_wave_pos();
        if title_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, title_wave));
        }

        self.apply_brightness(base_color, brightness)
    }

    /// Get shimmer color for shared title box segments (entry point, meet point)
    /// These show shimmer from BOTH top and bottom paths
    fn title_shared_color(&self, path_pos: f32, base_color: Color) -> Color {
        let mut brightness: f32 = 0.2;

        let title_wave = self.title_wave_pos();
        if title_wave >= 0.0 {
            brightness = brightness.max(self.shimmer_brightness(path_pos, title_wave));
        }

        self.apply_brightness(base_color, brightness)
    }

    /// Apply brightness to a base color
    fn apply_brightness(&self, base_color: Color, brightness: f32) -> Color {
        if let Color::Rgb(r, g, b) = base_color {
            // Interpolate between dim and bright versions
            let dim_r = (r as f32 * 0.3) as u8;
            let dim_g = (g as f32 * 0.3) as u8;
            let dim_b = (b as f32 * 0.3) as u8;

            Color::Rgb(
                lerp(dim_r, r, brightness),
                lerp(dim_g, g, brightness),
                lerp(dim_b, b, brightness),
            )
        } else {
            base_color
        }
    }

    /// Render the splash screen
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        // Clear the area
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ');
            }
        }

        // Calculate content area for blue background
        let content_width = 60;
        let content_height = 22;
        let start_x = area.x + (area.width.saturating_sub(content_width)) / 2;
        let start_y = area.y + (area.height.saturating_sub(content_height)) / 2;

        // Fill interior of the frame with blue background
        let interior_bg = Color::Rgb(15, 25, 45); // Dark blue background
        for y in (start_y + 1)..(start_y + content_height - 1) {
            for x in (start_x + 1)..(start_x + content_width - 1) {
                buf[(x, y)].set_bg(interior_bg);
            }
        }

        // Colors - unified cyan for all pipes and boxes
        let border_color = Color::Rgb(60, 80, 100);
        let text_color = Color::Rgb(180, 180, 180);
        let title_color = Color::Rgb(255, 255, 255);
        let hint_color = Color::Rgb(100, 100, 100);
        let pipe_color = Color::Rgb(100, 180, 220); // Unified cyan for all pipes
        let claude_color = pipe_color;
        let codex_color = pipe_color;
        let junction_color = pipe_color;

        // Draw the frame
        self.draw_frame(buf, start_x, start_y, content_width, content_height, border_color);

        // Layout:
        // Row 2:       ┌──┐
        // Row 3:    ───┤  ├───┐    ╭─────────────────────────────╮
        // Row 4:       └──┘   │    │                             │
        // Row 5:              ├────┤  C O N D U I T              │
        // Row 6:       ┌──┐   │    │                             │
        // Row 7:    ───┤  ├───┘    │  Multi-Agent Terminal       │
        // Row 8:       └──┘        ╰─────────────────────────────╯

        let pipe_start_x = start_x + 4;
        let junction_x = start_x + 17;
        let title_x = junction_x + 5;
        let title_y = start_y + 3;

        // Draw the CONDUIT title box with left connector
        self.draw_title_box_with_connector(buf, title_x, title_y, title_color, border_color, junction_color);

        // Draw top pipe (Claude) - connector box at row 2-4, main pipe at row 3
        self.draw_pipe_with_connector(
            buf,
            pipe_start_x,
            start_y + 2,  // connector box top
            junction_x,   // where it turns down
            claude_color,
            true,         // is top pipe
        );

        // Draw bottom pipe (Codex) - connector box at row 6-8, main pipe at row 7
        self.draw_pipe_with_connector(
            buf,
            pipe_start_x,
            start_y + 6,  // connector box top
            junction_x,   // where it turns up
            codex_color,
            false,        // is bottom pipe
        );

        // Draw vertical segments connecting pipes to junction
        // Path position: horizontal takes ~0.0-0.325, vertical ~0.325-0.375
        // Top vertical: │ at y+4
        let vert_path_pos = 0.34;
        let top_vert_color = self.top_pipe_color(vert_path_pos, claude_color);
        buf[(junction_x, start_y + 4)].set_char('│').set_style(Style::default().fg(top_vert_color));

        // Bottom vertical: │ at y+6
        let bottom_vert_color = self.bottom_pipe_color(vert_path_pos, codex_color);
        buf[(junction_x, start_y + 6)].set_char('│').set_style(Style::default().fg(bottom_vert_color));

        // Draw the junction and connection to title box
        // Junction ╠ is at same x as the pipe corners, connects to title box
        self.draw_junction(buf, junction_x, start_y + 5, title_x, junction_color);

        // Draw different content based on mode
        if self.first_time_mode {
            // First-time mode: Show Add Project button only
            self.render_add_project_button(buf, start_x, start_y, content_width, content_height);
        } else {
            // Normal mode: Show agent selection boxes
            let agents_y = start_y + 12;

            // Claude box
            self.draw_agent_box(buf, start_x + 4, agents_y, "1", "Claude Code", "Anthropic", claude_color, text_color);

            // Codex box
            self.draw_agent_box(buf, start_x + 32, agents_y, "2", "Codex CLI", "OpenAI", codex_color, text_color);

            // Draw hint text
            let hint = "[1] Claude · [2] Codex · [r] Add Repository · [q] Quit";
            let hint_x = start_x + (content_width.saturating_sub(hint.len() as u16)) / 2;
            let hint_y = start_y + content_height - 3;
            self.draw_text(buf, hint_x, hint_y, hint, hint_color);
        }

        // Version
        let version = "v0.1.0";
        let version_x = start_x + content_width - version.len() as u16 - 2;
        let version_y = start_y + content_height - 2;
        self.draw_text(buf, version_x, version_y, version, hint_color);
    }

    /// Render the Add Project button for first-time users
    fn render_add_project_button(&self, buf: &mut Buffer, start_x: u16, start_y: u16, content_width: u16, content_height: u16) {
        let button_text = "  + Add Project  ";
        let button_width = button_text.len() as u16;

        // Center the button
        let button_x = start_x + (content_width.saturating_sub(button_width)) / 2;
        let button_y = start_y + 13;

        // Draw highlighted button (cyan background, black text)
        let button_style = Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(100, 180, 220))
            .add_modifier(Modifier::BOLD);

        for (i, c) in button_text.chars().enumerate() {
            buf[(button_x + i as u16, button_y)].set_char(c).set_style(button_style);
        }

        // Draw quit hint below
        let quit_hint = "<Esc> Quit";
        let quit_x = start_x + (content_width.saturating_sub(quit_hint.len() as u16)) / 2;
        let quit_y = start_y + content_height - 3;
        self.draw_text(buf, quit_x, quit_y, quit_hint, Color::Rgb(100, 100, 100));
    }

    fn draw_frame(&self, buf: &mut Buffer, x: u16, y: u16, width: u16, height: u16, color: Color) {
        let style = Style::default().fg(color);

        // Top border
        buf[(x, y)].set_char('╔').set_style(style);
        for i in 1..width - 1 {
            buf[(x + i, y)].set_char('═').set_style(style);
        }
        buf[(x + width - 1, y)].set_char('╗').set_style(style);

        // Sides
        for i in 1..height - 1 {
            buf[(x, y + i)].set_char('║').set_style(style);
            buf[(x + width - 1, y + i)].set_char('║').set_style(style);
        }

        // Middle separator
        let sep_y = y + 10;
        buf[(x, sep_y)].set_char('╠').set_style(style);
        for i in 1..width - 1 {
            buf[(x + i, sep_y)].set_char('═').set_style(style);
        }
        buf[(x + width - 1, sep_y)].set_char('╣').set_style(style);

        // Bottom border
        buf[(x, y + height - 1)].set_char('╚').set_style(style);
        for i in 1..width - 1 {
            buf[(x + i, y + height - 1)].set_char('═').set_style(style);
        }
        buf[(x + width - 1, y + height - 1)].set_char('╝').set_style(style);
    }

    /// Draw title box with split shimmer paths
    /// TOP path: entry → up left → across top → down right to middle (meet point)
    /// BOTTOM path: entry → down left → across bottom → up right to middle (meet point)
    /// Both paths use positions 0.0 (entry) → 0.5 (meet point on right)
    fn draw_title_box_with_connector(&self, buf: &mut Buffer, x: u16, y: u16, title_color: Color, _border_color: Color, connector_color: Color) {
        let title = " C O N D U I T ";
        let subtitle = "Multi-Agent Terminal";
        let box_width: u16 = 30;
        let box_height: u16 = 6;
        // Entry is at row 2, meet point is at right edge around row 2-3

        // === ENTRY POINT (shared by both paths) ===
        // Entry connector (┤) at path position 0.0
        let entry_color = self.title_shared_color(0.0, connector_color);
        buf[(x, y + 2)].set_char('┤').set_style(Style::default().fg(entry_color));

        // === TOP PATH: entry → up → across top → down to meet ===

        // Left side going UP from entry (rows 1, 0)
        // Path positions: 0.0 → 0.12
        buf[(x, y + 1)].set_char('│').set_style(Style::default().fg(self.title_top_color(0.06, connector_color)));

        // Top-left corner (╭)
        buf[(x, y)].set_char('╭').set_style(Style::default().fg(self.title_top_color(0.12, connector_color)));

        // Top edge going right (─)
        // Path positions: 0.12 → 0.35
        for i in 1..box_width - 1 {
            let path_pos = 0.12 + (i as f32 / (box_width - 1) as f32) * 0.23;
            let color = self.title_top_color(path_pos, connector_color);
            buf[(x + i, y)].set_char('─').set_style(Style::default().fg(color));
        }

        // Top-right corner (╮)
        buf[(x + box_width - 1, y)].set_char('╮').set_style(Style::default().fg(self.title_top_color(0.35, connector_color)));

        // Right side going down from top to meet point (rows 1, 2)
        // Path positions: 0.35 → 0.5
        buf[(x + box_width - 1, y + 1)].set_char('│').set_style(Style::default().fg(self.title_top_color(0.42, connector_color)));

        // === BOTTOM PATH: entry → down → across bottom → up to meet ===

        // Left side going DOWN from entry (rows 3, 4)
        // Path positions: 0.0 → 0.12
        buf[(x, y + 3)].set_char('│').set_style(Style::default().fg(self.title_bottom_color(0.04, connector_color)));
        buf[(x, y + 4)].set_char('│').set_style(Style::default().fg(self.title_bottom_color(0.08, connector_color)));

        // Bottom-left corner (╰)
        buf[(x, y + box_height - 1)].set_char('╰').set_style(Style::default().fg(self.title_bottom_color(0.12, connector_color)));

        // Bottom edge going right (─)
        // Path positions: 0.12 → 0.35
        for i in 1..box_width - 1 {
            let path_pos = 0.12 + (i as f32 / (box_width - 1) as f32) * 0.23;
            let color = self.title_bottom_color(path_pos, connector_color);
            buf[(x + i, y + box_height - 1)].set_char('─').set_style(Style::default().fg(color));
        }

        // Bottom-right corner (╯)
        buf[(x + box_width - 1, y + box_height - 1)].set_char('╯').set_style(Style::default().fg(self.title_bottom_color(0.35, connector_color)));

        // Right side going up from bottom to meet point (rows 4, 3)
        // Path positions: 0.35 → 0.5
        buf[(x + box_width - 1, y + 4)].set_char('│').set_style(Style::default().fg(self.title_bottom_color(0.40, connector_color)));
        buf[(x + box_width - 1, y + 3)].set_char('│').set_style(Style::default().fg(self.title_bottom_color(0.45, connector_color)));

        // === MEET POINT (shared by both paths) ===
        // Right side at row 2 - both paths meet here
        buf[(x + box_width - 1, y + 2)].set_char('│').set_style(Style::default().fg(self.title_shared_color(0.5, connector_color)));

        // Title (centered)
        let title_start = x + (box_width - title.len() as u16) / 2;
        self.draw_text(buf, title_start, y + 2, title, title_color);

        // Subtitle (centered)
        let sub_start = x + (box_width - subtitle.len() as u16) / 2;
        self.draw_text(buf, sub_start, y + 3, subtitle, Color::Rgb(120, 120, 120));
    }

    /// Draw a pipe with bracket-style connector box
    /// Layout:    ┌──┐
    ///         ───┤  ├───┐ (or ┘ if is_top is false)
    ///            └──┘
    fn draw_pipe_with_connector(
        &self,
        buf: &mut Buffer,
        start_x: u16,
        box_top_y: u16,  // top of the connector box
        turn_x: u16,     // x position where pipe turns
        base_color: Color,
        is_top: bool,    // true = top pipe (╗), false = bottom pipe (╝)
    ) {
        let pipe_y = box_top_y + 1; // main pipe row is middle of connector box

        // Total horizontal path length for normalization
        let total_h_length = (turn_x - start_x) as f32;

        // Leading horizontal segment (before connector box)
        // Path: 0.0-0.325 for horizontal pipe (half of original since title box uses 0.5-1.0)
        let lead_length = 3u16;
        for i in 0..lead_length {
            let path_pos = i as f32 / total_h_length * 0.325; // 0-0.325 for horizontal
            let color = if is_top {
                self.top_pipe_color(path_pos, base_color)
            } else {
                self.bottom_pipe_color(path_pos, base_color)
            };
            buf[(start_x + i, pipe_y)].set_char('─').set_style(Style::default().fg(color));
        }

        // Connector box: ┌──┐
        //                ┤  ├
        //                └──┘
        let box_x = start_x + lead_length;

        // Use dimmed ambient color for static box borders (not part of shimmer path)
        let dim_color = self.apply_brightness(base_color, 0.2);
        let dim_style = Style::default().fg(dim_color);

        buf[(box_x, box_top_y)].set_char('┌').set_style(dim_style);
        buf[(box_x + 1, box_top_y)].set_char('─').set_style(dim_style);
        buf[(box_x + 2, box_top_y)].set_char('─').set_style(dim_style);
        buf[(box_x + 3, box_top_y)].set_char('┐').set_style(dim_style);

        // Connector brackets with shimmer
        let left_bracket_pos = lead_length as f32 / total_h_length * 0.325;
        let right_bracket_pos = (lead_length + 3) as f32 / total_h_length * 0.325;
        let left_color = if is_top {
            self.top_pipe_color(left_bracket_pos, base_color)
        } else {
            self.bottom_pipe_color(left_bracket_pos, base_color)
        };
        let right_color = if is_top {
            self.top_pipe_color(right_bracket_pos, base_color)
        } else {
            self.bottom_pipe_color(right_bracket_pos, base_color)
        };
        buf[(box_x, pipe_y)].set_char('┤').set_style(Style::default().fg(left_color));
        buf[(box_x + 3, pipe_y)].set_char('├').set_style(Style::default().fg(right_color));

        buf[(box_x, box_top_y + 2)].set_char('└').set_style(dim_style);
        buf[(box_x + 1, box_top_y + 2)].set_char('─').set_style(dim_style);
        buf[(box_x + 2, box_top_y + 2)].set_char('─').set_style(dim_style);
        buf[(box_x + 3, box_top_y + 2)].set_char('┘').set_style(dim_style);

        // Trailing horizontal segment (after connector box to turn point)
        let trail_start = box_x + 4;
        let trail_length = turn_x - trail_start;

        for i in 0..trail_length {
            let char_pos = (lead_length + 4 + i as u16) as f32;
            let path_pos = char_pos / total_h_length * 0.325;
            let color = if is_top {
                self.top_pipe_color(path_pos, base_color)
            } else {
                self.bottom_pipe_color(path_pos, base_color)
            };
            buf[(trail_start + i, pipe_y)].set_char('─').set_style(Style::default().fg(color));
        }

        // Corner where pipe turns (end of horizontal, start of vertical)
        let corner_char = if is_top { '┐' } else { '┘' };
        let corner_pos = 0.325; // Corner is at 32.5% of path
        let corner_color = if is_top {
            self.top_pipe_color(corner_pos, base_color)
        } else {
            self.bottom_pipe_color(corner_pos, base_color)
        };
        buf[(turn_x, pipe_y)].set_char(corner_char).set_style(Style::default().fg(corner_color));
    }

    /// Draw the junction where both pipes meet and connect to title box
    /// Layout: ├──── (connects to title box's ┤)
    fn draw_junction(&self, buf: &mut Buffer, x: u16, y: u16, title_x: u16, color: Color) {
        // Junction point where both pipes meet - path position ~0.375
        let junction_pos = 0.375;
        let junction_color = self.junction_color(junction_pos, color);
        buf[(x, y)].set_char('├').set_style(Style::default().fg(junction_color));

        // Horizontal segment to title box - path position 0.375 to 0.50
        let length = title_x - x - 1;
        for i in 1..=length {
            let path_pos = 0.375 + (i as f32 / (length as f32 + 1.0)) * 0.125;
            let shimmer_color = self.junction_color(path_pos, color);
            buf[(x + i, y)].set_char('─').set_style(Style::default().fg(shimmer_color));
        }
    }

    fn draw_agent_box(&self, buf: &mut Buffer, x: u16, y: u16, key: &str, name: &str, provider: &str, accent_color: Color, text_color: Color) {
        let border_style = Style::default().fg(Color::Rgb(60, 80, 100));
        let width = 24u16;

        // Top border
        buf[(x, y)].set_char('┌').set_style(border_style);
        for i in 1..width - 1 {
            buf[(x + i, y)].set_char('─').set_style(border_style);
        }
        buf[(x + width - 1, y)].set_char('┐').set_style(border_style);

        // Content line 1: key and name
        buf[(x, y + 1)].set_char('│').set_style(border_style);
        buf[(x + 2, y + 1)].set_char('[').set_style(Style::default().fg(Color::Rgb(80, 80, 80)));
        self.draw_text(buf, x + 3, y + 1, key, accent_color);
        buf[(x + 4, y + 1)].set_char(']').set_style(Style::default().fg(Color::Rgb(80, 80, 80)));
        self.draw_text(buf, x + 6, y + 1, name, text_color);
        buf[(x + width - 1, y + 1)].set_char('│').set_style(border_style);

        // Content line 2: provider
        buf[(x, y + 2)].set_char('│').set_style(border_style);
        self.draw_text(buf, x + 6, y + 2, provider, Color::Rgb(100, 100, 100));
        buf[(x + width - 1, y + 2)].set_char('│').set_style(border_style);

        // Bottom border
        buf[(x, y + 3)].set_char('└').set_style(border_style);
        for i in 1..width - 1 {
            buf[(x + i, y + 3)].set_char('─').set_style(border_style);
        }
        buf[(x + width - 1, y + 3)].set_char('┘').set_style(border_style);
    }

    fn draw_text(&self, buf: &mut Buffer, x: u16, y: u16, text: &str, color: Color) {
        let style = Style::default().fg(color);
        for (i, c) in text.chars().enumerate() {
            buf[(x + i as u16, y)].set_char(c).set_style(style);
        }
    }
}

impl Default for SplashScreen {
    fn default() -> Self {
        Self::new()
    }
}
