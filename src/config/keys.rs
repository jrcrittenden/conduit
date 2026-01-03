//! Keybinding configuration types and parsing
//!
//! This module provides types for representing keyboard shortcuts and
//! parsing vim-style key notation (e.g., "C-x", "M-S-w", "<CR>").

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize};

use crate::ui::action::Action;

/// A key combination (key code + modifiers)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyCombo {
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Create a KeyCombo from a crossterm KeyEvent
    pub fn from_key_event(event: &crossterm::event::KeyEvent) -> Self {
        Self {
            code: event.code,
            modifiers: event.modifiers,
        }
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();

        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("C");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("M");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("S");
        }

        let key_str = match self.code {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "<CR>".to_string(),
            KeyCode::Esc => "<Esc>".to_string(),
            KeyCode::Tab => "<Tab>".to_string(),
            KeyCode::Backspace => "<BS>".to_string(),
            KeyCode::Delete => "<Del>".to_string(),
            KeyCode::Up => "<Up>".to_string(),
            KeyCode::Down => "<Down>".to_string(),
            KeyCode::Left => "<Left>".to_string(),
            KeyCode::Right => "<Right>".to_string(),
            KeyCode::PageUp => "<PageUp>".to_string(),
            KeyCode::PageDown => "<PageDown>".to_string(),
            KeyCode::Home => "<Home>".to_string(),
            KeyCode::End => "<End>".to_string(),
            KeyCode::F(n) => format!("<F{}>", n),
            _ => format!("{:?}", self.code),
        };

        if parts.is_empty() {
            write!(f, "{}", key_str)
        } else {
            parts.push(&key_str);
            write!(f, "{}", parts.join("-"))
        }
    }
}

/// Context for keybindings (logical grouping of input modes)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyContext {
    /// Global keys that work in all modes
    Global,
    /// Chat input mode (Normal InputMode)
    Chat,
    /// Scrolling through chat history
    Scrolling,
    /// Sidebar navigation
    Sidebar,
    /// Dialog/modal contexts (confirmations, errors)
    Dialog,
    /// Project picker
    ProjectPicker,
    /// Model selector
    ModelSelector,
    /// Adding repository path
    AddRepository,
    /// Setting base directory
    BaseDir,
    /// Raw events debug view
    RawEvents,
}

impl KeyContext {
    /// Get all non-global contexts
    pub fn all_contexts() -> &'static [KeyContext] {
        &[
            KeyContext::Chat,
            KeyContext::Scrolling,
            KeyContext::Sidebar,
            KeyContext::Dialog,
            KeyContext::ProjectPicker,
            KeyContext::ModelSelector,
            KeyContext::AddRepository,
            KeyContext::BaseDir,
            KeyContext::RawEvents,
        ]
    }

    /// Convert from InputMode to KeyContext
    pub fn from_input_mode(mode: crate::ui::events::InputMode, view_mode: crate::ui::events::ViewMode) -> Self {
        use crate::ui::events::{InputMode, ViewMode};

        // RawEvents view takes precedence over input mode
        if view_mode == ViewMode::RawEvents {
            return KeyContext::RawEvents;
        }

        match mode {
            InputMode::Normal => KeyContext::Chat,
            InputMode::SelectingAgent => KeyContext::Dialog,
            InputMode::Scrolling => KeyContext::Scrolling,
            InputMode::SidebarNavigation => KeyContext::Sidebar,
            InputMode::AddingRepository => KeyContext::AddRepository,
            InputMode::SelectingModel => KeyContext::ModelSelector,
            InputMode::SettingBaseDir => KeyContext::BaseDir,
            InputMode::PickingProject => KeyContext::ProjectPicker,
            InputMode::Confirming => KeyContext::Dialog,
            InputMode::RemovingProject => KeyContext::Dialog,
            InputMode::ShowingError => KeyContext::Dialog,
        }
    }
}

/// Configuration for all keybindings
#[derive(Debug, Clone, Default)]
pub struct KeybindingConfig {
    /// Global keybindings (apply to all contexts unless overridden)
    pub global: HashMap<KeyCombo, Action>,
    /// Context-specific keybindings
    pub context: HashMap<KeyContext, HashMap<KeyCombo, Action>>,
}

impl KeybindingConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up an action for a key combo in a given context
    /// First checks context-specific bindings, then falls back to global
    pub fn get_action(&self, key: &KeyCombo, context: KeyContext) -> Option<&Action> {
        // First check context-specific bindings
        if let Some(context_bindings) = self.context.get(&context) {
            if let Some(action) = context_bindings.get(key) {
                return Some(action);
            }
        }

        // Fall back to global bindings
        self.global.get(key)
    }

    /// Merge user configuration on top of defaults
    pub fn merge(&mut self, other: KeybindingConfig) {
        // Merge global bindings
        for (key, action) in other.global {
            self.global.insert(key, action);
        }

        // Merge context-specific bindings
        for (ctx, bindings) in other.context {
            let entry = self.context.entry(ctx).or_default();
            for (key, action) in bindings {
                entry.insert(key, action);
            }
        }
    }
}

/// Parse a vim-style key notation string into a KeyCombo
///
/// Supported notation:
/// - `C-x` for Ctrl+x
/// - `M-x` for Alt+x (Meta)
/// - `S-x` for Shift+x
/// - `C-S-x` for Ctrl+Shift+x
/// - `<CR>` for Enter
/// - `<Esc>` for Escape
/// - `<Tab>` for Tab
/// - `<BS>` for Backspace
/// - `<Del>` for Delete
/// - `<Up>`, `<Down>`, `<Left>`, `<Right>` for arrow keys
/// - `<PageUp>`, `<PageDown>` for page navigation
/// - `<Home>`, `<End>` for line navigation
/// - `<Space>` for space
/// - `<F1>` through `<F12>` for function keys
pub fn parse_key_notation(s: &str) -> Result<KeyCombo, KeyParseError> {
    let s = s.trim();

    if s.is_empty() {
        return Err(KeyParseError::Empty);
    }

    // Check for special keys in angle brackets
    if s.starts_with('<') && s.ends_with('>') {
        return parse_special_key(s);
    }

    // Parse modifier-key combinations like "C-x", "M-S-w"
    let parts: Vec<&str> = s.split('-').collect();

    if parts.is_empty() {
        return Err(KeyParseError::Empty);
    }

    let mut modifiers = KeyModifiers::NONE;
    let mut key_part = None;

    for (i, part) in parts.iter().enumerate() {
        match *part {
            "C" => modifiers |= KeyModifiers::CONTROL,
            "M" => modifiers |= KeyModifiers::ALT,
            "S" => {
                // S is Shift only if there are more parts after it
                // or if it's followed by another modifier
                if i < parts.len() - 1 {
                    modifiers |= KeyModifiers::SHIFT;
                } else {
                    // 'S' is the key itself
                    key_part = Some(*part);
                }
            }
            _ => {
                // This is the key part
                key_part = Some(*part);
            }
        }
    }

    let key_str = key_part.ok_or(KeyParseError::NoKey)?;
    let code = parse_key_code(key_str)?;

    Ok(KeyCombo::new(code, modifiers))
}

/// Parse a special key notation like <CR>, <Esc>, etc.
fn parse_special_key(s: &str) -> Result<KeyCombo, KeyParseError> {
    // Remove angle brackets
    let inner = &s[1..s.len() - 1];

    // Check for modifiers in special key notation like <C-CR>
    let parts: Vec<&str> = inner.split('-').collect();

    let mut modifiers = KeyModifiers::NONE;
    let mut key_name = inner;

    if parts.len() > 1 {
        // Has modifiers
        for part in &parts[..parts.len() - 1] {
            match *part {
                "C" => modifiers |= KeyModifiers::CONTROL,
                "M" => modifiers |= KeyModifiers::ALT,
                "S" => modifiers |= KeyModifiers::SHIFT,
                _ => return Err(KeyParseError::InvalidModifier(part.to_string())),
            }
        }
        key_name = parts[parts.len() - 1];
    }

    let code = match key_name.to_uppercase().as_str() {
        "CR" | "ENTER" | "RETURN" => KeyCode::Enter,
        "ESC" | "ESCAPE" => KeyCode::Esc,
        "TAB" => KeyCode::Tab,
        "BS" | "BACKSPACE" => KeyCode::Backspace,
        "DEL" | "DELETE" => KeyCode::Delete,
        "UP" => KeyCode::Up,
        "DOWN" => KeyCode::Down,
        "LEFT" => KeyCode::Left,
        "RIGHT" => KeyCode::Right,
        "PAGEUP" | "PGUP" => KeyCode::PageUp,
        "PAGEDOWN" | "PGDN" => KeyCode::PageDown,
        "HOME" => KeyCode::Home,
        "END" => KeyCode::End,
        "SPACE" => KeyCode::Char(' '),
        s if s.starts_with('F') && s.len() > 1 => {
            let num: u8 = s[1..]
                .parse()
                .map_err(|_| KeyParseError::InvalidKey(s.to_string()))?;
            if num == 0 || num > 12 {
                return Err(KeyParseError::InvalidKey(s.to_string()));
            }
            KeyCode::F(num)
        }
        _ => return Err(KeyParseError::InvalidSpecialKey(key_name.to_string())),
    };

    Ok(KeyCombo::new(code, modifiers))
}

/// Parse a single key code (not a special key)
fn parse_key_code(s: &str) -> Result<KeyCode, KeyParseError> {
    if s.len() == 1 {
        let c = s.chars().next().unwrap();
        Ok(KeyCode::Char(c.to_ascii_lowercase()))
    } else if s.starts_with('<') && s.ends_with('>') {
        // Handle special keys without modifiers
        let key = parse_special_key(s)?;
        Ok(key.code)
    } else if s == "\\" {
        Ok(KeyCode::Char('\\'))
    } else {
        // Try parsing as a special key name without brackets
        match s.to_uppercase().as_str() {
            "SPACE" => Ok(KeyCode::Char(' ')),
            "TAB" => Ok(KeyCode::Tab),
            "ENTER" | "CR" | "RETURN" => Ok(KeyCode::Enter),
            "ESC" | "ESCAPE" => Ok(KeyCode::Esc),
            "BS" | "BACKSPACE" => Ok(KeyCode::Backspace),
            _ => Err(KeyParseError::InvalidKey(s.to_string())),
        }
    }
}

/// Error type for key parsing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyParseError {
    Empty,
    NoKey,
    InvalidKey(String),
    InvalidModifier(String),
    InvalidSpecialKey(String),
}

impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyParseError::Empty => write!(f, "empty key notation"),
            KeyParseError::NoKey => write!(f, "no key specified"),
            KeyParseError::InvalidKey(s) => write!(f, "invalid key: {}", s),
            KeyParseError::InvalidModifier(s) => write!(f, "invalid modifier: {}", s),
            KeyParseError::InvalidSpecialKey(s) => write!(f, "invalid special key: {}", s),
        }
    }
}

impl std::error::Error for KeyParseError {}

impl FromStr for KeyCombo {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_key_notation(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_key() {
        let key = parse_key_notation("a").unwrap();
        assert_eq!(key.code, KeyCode::Char('a'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_ctrl_key() {
        let key = parse_key_notation("C-x").unwrap();
        assert_eq!(key.code, KeyCode::Char('x'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_parse_alt_key() {
        let key = parse_key_notation("M-x").unwrap();
        assert_eq!(key.code, KeyCode::Char('x'));
        assert_eq!(key.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn test_parse_ctrl_shift_key() {
        let key = parse_key_notation("C-S-w").unwrap();
        assert_eq!(key.code, KeyCode::Char('w'));
        assert_eq!(
            key.modifiers,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        );
    }

    #[test]
    fn test_parse_special_keys() {
        assert_eq!(parse_key_notation("<CR>").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_key_notation("<Esc>").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_key_notation("<Tab>").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_key_notation("<BS>").unwrap().code, KeyCode::Backspace);
        assert_eq!(parse_key_notation("<Space>").unwrap().code, KeyCode::Char(' '));
    }

    #[test]
    fn test_parse_arrow_keys() {
        assert_eq!(parse_key_notation("<Up>").unwrap().code, KeyCode::Up);
        assert_eq!(parse_key_notation("<Down>").unwrap().code, KeyCode::Down);
        assert_eq!(parse_key_notation("<Left>").unwrap().code, KeyCode::Left);
        assert_eq!(parse_key_notation("<Right>").unwrap().code, KeyCode::Right);
    }

    #[test]
    fn test_parse_function_keys() {
        assert_eq!(parse_key_notation("<F1>").unwrap().code, KeyCode::F(1));
        assert_eq!(parse_key_notation("<F12>").unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn test_parse_backslash() {
        let key = parse_key_notation("C-\\").unwrap();
        assert_eq!(key.code, KeyCode::Char('\\'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_display_key_combo() {
        let key = KeyCombo::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert_eq!(key.to_string(), "C-x");

        let key = KeyCombo::new(KeyCode::Char('w'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(key.to_string(), "C-S-w");

        let key = KeyCombo::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(key.to_string(), "<CR>");
    }
}
