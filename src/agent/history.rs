//! Agent session history reader
//!
//! Reads chat history from agent files for session restoration.
//! - Claude Code: ~/.claude/projects/{project-path}/{session-id}.jsonl
//! - Codex CLI: ~/.codex/sessions/YYYY/MM/DD/rollout-*-{session-id}.jsonl

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use serde_json::Value;

use crate::ui::components::{ChatMessage, MessageRole};

/// Error type for history loading
#[derive(Debug)]
pub enum HistoryError {
    HomeNotFound,
    SessionNotFound(String),
    IoError(std::io::Error),
    ParseError(String),
}

impl From<std::io::Error> for HistoryError {
    fn from(e: std::io::Error) -> Self {
        HistoryError::IoError(e)
    }
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::HomeNotFound => write!(f, "Home directory not found"),
            HistoryError::SessionNotFound(id) => write!(f, "Session not found: {}", id),
            HistoryError::IoError(e) => write!(f, "IO error: {}", e),
            HistoryError::ParseError(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl std::error::Error for HistoryError {}

/// Load Claude Code history for a session
///
/// Claude stores sessions as `~/.claude/projects/{project-path}/{session-id}.jsonl`
pub fn load_claude_history(session_id: &str) -> Result<Vec<ChatMessage>, HistoryError> {
    let home = dirs::home_dir().ok_or(HistoryError::HomeNotFound)?;
    let projects_dir = home.join(".claude/projects");

    if !projects_dir.exists() {
        return Err(HistoryError::SessionNotFound(session_id.to_string()));
    }

    // Search all project directories for this session
    let session_file = find_claude_session_file(&projects_dir, session_id)?;
    parse_claude_history_file(&session_file)
}

/// Find Claude session file by searching project directories
fn find_claude_session_file(projects_dir: &PathBuf, session_id: &str) -> Result<PathBuf, HistoryError> {
    let filename = format!("{}.jsonl", session_id);

    // Iterate through project directories
    if let Ok(entries) = fs::read_dir(projects_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let session_path = path.join(&filename);
                if session_path.exists() {
                    return Ok(session_path);
                }
            }
        }
    }

    Err(HistoryError::SessionNotFound(session_id.to_string()))
}

/// Parse a Claude history JSONL file
fn parse_claude_history_file(path: &PathBuf) -> Result<Vec<ChatMessage>, HistoryError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<Value>(&line) {
            if let Some(msg) = convert_claude_entry(&entry) {
                messages.push(msg);
            }
        }
    }

    Ok(messages)
}

/// Convert a Claude JSONL entry to ChatMessage
fn convert_claude_entry(entry: &Value) -> Option<ChatMessage> {
    let entry_type = entry.get("type")?.as_str()?;

    match entry_type {
        "user" => {
            // User message: {"type":"user","message":{"role":"user","content":"..."}}
            let message = entry.get("message")?;
            let content = message.get("content")?.as_str()?;
            Some(ChatMessage {
                role: MessageRole::User,
                content: content.to_string(),
                tool_name: None,
                tool_args: None,
                is_streaming: false,
                summary: None,
            })
        }
        "assistant" => {
            // Assistant message: {"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}
            let message = entry.get("message")?;
            let content = message.get("content")?;

            // Content can be a string or array of content blocks
            let text = if let Some(text) = content.as_str() {
                text.to_string()
            } else if let Some(blocks) = content.as_array() {
                // Extract text from content blocks
                blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        match block_type {
                            "text" => block.get("text")?.as_str().map(|s| s.to_string()),
                            "tool_use" => {
                                // Summarize tool use
                                let name = block.get("name")?.as_str()?;
                                Some(format!("[Tool: {}]", name))
                            }
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                return None;
            };

            if text.is_empty() {
                return None;
            }

            Some(ChatMessage {
                role: MessageRole::Assistant,
                content: text,
                tool_name: None,
                tool_args: None,
                is_streaming: false,
                summary: None,
            })
        }
        _ => None, // Skip queue-operation and other types
    }
}

/// Load Codex CLI history for a session
///
/// Codex stores sessions as `~/.codex/sessions/YYYY/MM/DD/rollout-{timestamp}-{uuid}.jsonl`
pub fn load_codex_history(session_id: &str) -> Result<Vec<ChatMessage>, HistoryError> {
    let home = dirs::home_dir().ok_or(HistoryError::HomeNotFound)?;
    let sessions_dir = home.join(".codex/sessions");

    if !sessions_dir.exists() {
        return Err(HistoryError::SessionNotFound(session_id.to_string()));
    }

    // Search for session file containing the session ID
    let session_file = find_codex_session_file(&sessions_dir, session_id)?;
    parse_codex_history_file(&session_file)
}

/// Find Codex session file by searching recursively
fn find_codex_session_file(sessions_dir: &PathBuf, session_id: &str) -> Result<PathBuf, HistoryError> {
    // Walk through year/month/day directories
    for year_entry in fs::read_dir(sessions_dir)?.flatten() {
        let year_path = year_entry.path();
        if !year_path.is_dir() {
            continue;
        }

        for month_entry in fs::read_dir(&year_path).into_iter().flatten().flatten() {
            let month_path = month_entry.path();
            if !month_path.is_dir() {
                continue;
            }

            for day_entry in fs::read_dir(&month_path).into_iter().flatten().flatten() {
                let day_path = day_entry.path();
                if !day_path.is_dir() {
                    continue;
                }

                // Look for files containing the session ID
                for file_entry in fs::read_dir(&day_path).into_iter().flatten().flatten() {
                    let file_path = file_entry.path();
                    if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
                        if name.contains(session_id) && name.ends_with(".jsonl") {
                            return Ok(file_path);
                        }
                    }
                }
            }
        }
    }

    Err(HistoryError::SessionNotFound(session_id.to_string()))
}

/// Parse a Codex history JSONL file
fn parse_codex_history_file(path: &PathBuf) -> Result<Vec<ChatMessage>, HistoryError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<Value>(&line) {
            if let Some(msg) = convert_codex_entry(&entry) {
                messages.push(msg);
            }
        }
    }

    Ok(messages)
}

/// Convert a Codex JSONL entry to ChatMessage
fn convert_codex_entry(entry: &Value) -> Option<ChatMessage> {
    let entry_type = entry.get("type")?.as_str()?;

    if entry_type != "response_item" {
        return None;
    }

    let payload = entry.get("payload")?;
    let role = payload.get("role")?.as_str()?;
    let content = payload.get("content")?;

    // Extract text from content blocks
    let text = if let Some(blocks) = content.as_array() {
        blocks
            .iter()
            .filter_map(|block| {
                let block_type = block.get("type")?.as_str()?;
                match block_type {
                    "input_text" | "output_text" | "text" => {
                        block.get("text")?.as_str().map(|s| s.to_string())
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        return None;
    };

    // Skip environment context messages
    if text.contains("<environment_context>") {
        return None;
    }

    if text.is_empty() {
        return None;
    }

    let message_role = match role {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        _ => return None,
    };

    Some(ChatMessage {
        role: message_role,
        content: text,
        tool_name: None,
        tool_args: None,
        is_streaming: false,
        summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_claude_user_entry() {
        let entry = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": "Hello, Claude!"
            }
        });

        let msg = convert_claude_entry(&entry).unwrap();
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "Hello, Claude!");
    }

    #[test]
    fn test_convert_claude_assistant_entry() {
        let entry = serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Hello!"},
                    {"type": "text", "text": "How can I help?"}
                ]
            }
        });

        let msg = convert_claude_entry(&entry).unwrap();
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.content, "Hello!\nHow can I help?");
    }

    #[test]
    fn test_skip_queue_operation() {
        let entry = serde_json::json!({
            "type": "queue-operation",
            "operation": "dequeue"
        });

        assert!(convert_claude_entry(&entry).is_none());
    }

    #[test]
    fn test_convert_codex_user_entry() {
        let entry = serde_json::json!({
            "type": "response_item",
            "payload": {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What is the answer?"}
                ]
            }
        });

        let msg = convert_codex_entry(&entry).unwrap();
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content, "What is the answer?");
    }

    #[test]
    fn test_skip_environment_context() {
        let entry = serde_json::json!({
            "type": "response_item",
            "payload": {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>"}
                ]
            }
        });

        assert!(convert_codex_entry(&entry).is_none());
    }
}
