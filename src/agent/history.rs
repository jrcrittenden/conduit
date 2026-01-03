//! Agent session history reader
//!
//! Reads chat history from agent files for session restoration.
//! - Claude Code: ~/.claude/projects/{project-path}/{session-id}.jsonl
//! - Codex CLI: ~/.codex/sessions/YYYY/MM/DD/rollout-*-{session-id}.jsonl

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use serde_json::Value;

use crate::ui::components::{ChatMessage, MessageRole};

/// Info extracted from a function_call entry for later lookup
struct FunctionCallInfo {
    name: String,
    command: String,
}

/// Debug entry for history loading - shows what happened to each JSONL line
#[derive(Debug, Clone)]
pub struct HistoryDebugEntry {
    /// Line number in the file (0-indexed)
    pub line_number: usize,
    /// Entry type from JSON (e.g., "response_item", "session_meta")
    pub entry_type: String,
    /// Status of this entry (INCLUDE, SKIP, ERROR)
    pub status: String,
    /// Reason for status (e.g., "role=user", "filtered: environment_context")
    pub reason: String,
    /// Raw JSON value
    pub raw_json: Value,
}

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

/// Parse a Codex history JSONL file with debug information
pub fn parse_codex_history_file_with_debug(path: &PathBuf) -> Result<(Vec<ChatMessage>, Vec<HistoryDebugEntry>), HistoryError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

    // First pass: collect function_call entries by call_id
    let mut function_calls: HashMap<String, FunctionCallInfo> = HashMap::new();
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            if let Some((call_id, info)) = extract_function_call_info(&entry) {
                function_calls.insert(call_id, info);
            }
        }
    }

    // Second pass: process all entries with function_call lookup
    let mut messages = Vec::new();
    let mut debug_entries = Vec::new();

    for (line_num, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(line) {
            Ok(entry) => {
                let (msg, status, reason) = convert_codex_entry_with_debug(&entry, &function_calls);
                let entry_type = entry.get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                debug_entries.push(HistoryDebugEntry {
                    line_number: line_num,
                    entry_type,
                    status: status.to_string(),
                    reason,
                    raw_json: entry,
                });

                if let Some(m) = msg {
                    messages.push(m);
                }
            }
            Err(e) => {
                debug_entries.push(HistoryDebugEntry {
                    line_number: line_num,
                    entry_type: "parse_error".to_string(),
                    status: "ERROR".to_string(),
                    reason: e.to_string(),
                    raw_json: Value::String(line.clone()),
                });
            }
        }
    }

    Ok((messages, debug_entries))
}

/// Extract function_call info from a response_item entry
fn extract_function_call_info(entry: &Value) -> Option<(String, FunctionCallInfo)> {
    let entry_type = entry.get("type")?.as_str()?;
    if entry_type != "response_item" {
        return None;
    }

    let payload = entry.get("payload")?;
    let payload_type = payload.get("type")?.as_str()?;
    if payload_type != "function_call" {
        return None;
    }

    let call_id = payload.get("call_id")?.as_str()?.to_string();
    let name = payload.get("name")?.as_str()?.to_string();

    // Parse arguments JSON to get command
    let args_str = payload.get("arguments")?.as_str()?;
    let args: Value = serde_json::from_str(args_str).ok()?;
    let command = args.get("cmd")?.as_str()?.to_string();

    Some((call_id, FunctionCallInfo { name, command }))
}

/// Load Codex CLI history with debug information
pub fn load_codex_history_with_debug(session_id: &str) -> Result<(Vec<ChatMessage>, Vec<HistoryDebugEntry>, PathBuf), HistoryError> {
    let home = dirs::home_dir().ok_or(HistoryError::HomeNotFound)?;
    let sessions_dir = home.join(".codex/sessions");

    if !sessions_dir.exists() {
        return Err(HistoryError::SessionNotFound(session_id.to_string()));
    }

    let session_file = find_codex_session_file(&sessions_dir, session_id)?;
    let (messages, debug_entries) = parse_codex_history_file_with_debug(&session_file)?;
    Ok((messages, debug_entries, session_file))
}

/// Create a truncated preview of text for debug output
fn truncate_preview(text: &str, max_len: usize) -> String {
    let preview: String = text.chars().take(max_len).collect();
    if text.chars().count() > max_len {
        format!("{}...", preview.replace('\n', " "))
    } else {
        preview.replace('\n', " ")
    }
}

/// Extract text content from a Codex payload's content blocks
fn extract_text_content(payload: &Value) -> String {
    let content = match payload.get("content") {
        Some(c) => c,
        None => return String::new(),
    };

    if let Some(blocks) = content.as_array() {
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
        String::new()
    }
}

/// Convert a Codex JSONL entry to ChatMessage with debug information
/// Returns (Option<ChatMessage>, status, reason)
fn convert_codex_entry_with_debug(
    entry: &Value,
    function_calls: &HashMap<String, FunctionCallInfo>,
) -> (Option<ChatMessage>, &'static str, String) {
    let entry_type = match entry.get("type").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return (None, "SKIP", "missing type field".to_string()),
    };

    let payload = match entry.get("payload") {
        Some(p) => p,
        None => return (None, "SKIP", "missing payload".to_string()),
    };

    match entry_type {
        "event_msg" => {
            // Skip event_msg entries - user messages are already in response_item
            let payload_type = payload.get("type").and_then(|t| t.as_str());
            (None, "SKIP", format!("event_msg type={:?}", payload_type))
        }

        "response_item" => {
            let payload_type = payload.get("type").and_then(|t| t.as_str());

            // Handle function_call_output (tool results)
            if payload_type == Some("function_call_output") {
                if let Some(output) = payload.get("output").and_then(|o| o.as_str()) {
                    let call_id = payload
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .unwrap_or("unknown");

                    // Look up the function call to get the command
                    let (tool_name, tool_args) = if let Some(info) = function_calls.get(call_id) {
                        (info.name.clone(), info.command.clone())
                    } else {
                        ("shell".to_string(), call_id.to_string())
                    };

                    let preview = truncate_preview(output, 60);
                    return (
                        Some(ChatMessage::tool(&tool_name, &tool_args, output)),
                        "INCLUDE",
                        format!("{}({}): \"{}\"", tool_name, truncate_preview(&tool_args, 30), preview),
                    );
                }
            }

            // Handle regular messages with role
            let role = match payload.get("role").and_then(|r| r.as_str()) {
                Some(r) => r,
                None => return (None, "SKIP", format!("role is null, type={:?}", payload_type)),
            };

            // Extract text content
            let text = extract_text_content(payload);
            if text.is_empty() {
                return (None, "SKIP", "empty text content".to_string());
            }

            // Filter system content
            if text.contains("<environment_context>") {
                return (None, "SKIP", "filtered: environment_context".to_string());
            }
            if text.starts_with("# AGENTS.md instructions") {
                return (None, "SKIP", "filtered: AGENTS.md instructions".to_string());
            }
            if text.contains("<INSTRUCTIONS>") {
                return (None, "SKIP", "filtered: INSTRUCTIONS tags".to_string());
            }

            let preview = truncate_preview(&text, 60);
            let msg = match role {
                "user" => ChatMessage::user(text),
                "assistant" => ChatMessage::assistant(text),
                _ => return (None, "SKIP", format!("unknown role: {}", role)),
            };

            (Some(msg), "INCLUDE", format!("role={}: \"{}\"", role, preview))
        }

        _ => (None, "SKIP", format!("type={}", entry_type)),
    }
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
        let function_calls = HashMap::new();

        let (msg, status, _reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "INCLUDE");
        let msg = msg.unwrap();
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
        let function_calls = HashMap::new();

        let (msg, status, _reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "SKIP");
        assert!(msg.is_none());
    }

    #[test]
    fn test_event_msg_skipped() {
        // event_msg entries are skipped - user messages come from response_item
        let entry = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": "Can you inspect the git log?"
            }
        });
        let function_calls = HashMap::new();

        let (msg, status, _reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "SKIP");
        assert!(msg.is_none());
    }

    #[test]
    fn test_function_call_output() {
        let entry = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_123",
                "output": "commit abc123\nAuthor: Test"
            }
        });
        let function_calls = HashMap::new();

        let (msg, status, _reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "INCLUDE");
        let msg = msg.unwrap();
        assert_eq!(msg.role, MessageRole::Tool);
        assert!(msg.content.contains("commit abc123"));
    }

    #[test]
    fn test_function_call_output_with_lookup() {
        // Test that function_call_output looks up the command from function_calls
        let entry = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_123",
                "output": "commit abc123\nAuthor: Test"
            }
        });

        let mut function_calls = HashMap::new();
        function_calls.insert(
            "call_123".to_string(),
            FunctionCallInfo {
                name: "exec_command".to_string(),
                command: "git log -1 --stat".to_string(),
            },
        );

        let (msg, status, reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "INCLUDE");
        let msg = msg.unwrap();
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_name, Some("exec_command".to_string()));
        assert_eq!(msg.tool_args, Some("git log -1 --stat".to_string()));
        assert!(reason.contains("exec_command"));
        assert!(reason.contains("git log"));
    }

    #[test]
    fn test_skip_agents_md() {
        let entry = serde_json::json!({
            "type": "response_item",
            "payload": {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "# AGENTS.md instructions\n\nThis is system content..."}
                ]
            }
        });
        let function_calls = HashMap::new();

        let (msg, status, reason) = convert_codex_entry_with_debug(&entry, &function_calls);
        assert_eq!(status, "SKIP");
        assert!(msg.is_none());
        assert!(reason.contains("AGENTS.md"));
    }
}
