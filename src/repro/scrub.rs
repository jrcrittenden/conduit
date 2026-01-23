use regex::Regex;

use crate::repro::tape::{RecordedInput, ReproTape, ReproTapeEntry};

#[derive(Debug, Clone)]
pub struct ScrubConfig {
    patterns: Vec<Regex>,
}

impl ScrubConfig {
    pub fn new(patterns: Vec<Regex>) -> Self {
        Self { patterns }
    }

    pub fn default_patterns() -> Vec<Regex> {
        // Keep patterns simple: the Rust `regex` crate doesn't support look-behind.
        let raw = [
            r"sk-[A-Za-z0-9]{10,}",
            r"Bearer\s+[A-Za-z0-9._-]{10,}",
            r"(?i)anthropic[_-]?api[_-]?key\s*=\s*[A-Za-z0-9._-]{10,}",
            r"(?i)openai[_-]?api[_-]?key\s*=\s*[A-Za-z0-9._-]{10,}",
        ];
        raw.into_iter().filter_map(|p| Regex::new(p).ok()).collect()
    }

    pub fn default_shareable() -> Self {
        Self::new(Self::default_patterns())
    }

    pub fn scrub_string(&self, input: &str) -> String {
        let mut out = input.to_string();
        for re in &self.patterns {
            out = re.replace_all(&out, "[REDACTED]").into_owned();
        }
        out
    }

    pub fn scrub_tape(&self, tape: &mut ReproTape) {
        for entry in &mut tape.entries {
            self.scrub_entry(entry);
        }
    }

    fn scrub_entry(&self, entry: &mut ReproTapeEntry) {
        match entry {
            ReproTapeEntry::AgentEvent { event, .. } => {
                // Best-effort scrubbing: only touch known text fields.
                // AgentEvent already contains structured data; we avoid heavy-handed stringification.
                if let crate::agent::events::AgentEvent::AssistantMessage(msg) = event {
                    msg.text = self.scrub_string(&msg.text);
                }
                if let crate::agent::events::AgentEvent::AssistantReasoning(reason) = event {
                    reason.text = self.scrub_string(&reason.text);
                }
                if let crate::agent::events::AgentEvent::TurnFailed(failed) = event {
                    failed.error = self.scrub_string(&failed.error);
                }
                if let crate::agent::events::AgentEvent::Error(err) = event {
                    err.message = self.scrub_string(&err.message);
                }
                if let crate::agent::events::AgentEvent::CommandOutput(out) = event {
                    out.output = self.scrub_string(&out.output);
                    out.command = self.scrub_string(&out.command);
                }
            }
            ReproTapeEntry::AgentInput { input, .. } => match input {
                RecordedInput::ClaudeJsonl { jsonl } => {
                    *jsonl = self.scrub_string(jsonl);
                }
                RecordedInput::CodexPrompt { text, .. } => {
                    *text = self.scrub_string(text);
                }
                RecordedInput::OpencodeQuestion { .. } => {
                    // Usually safe: answers are options/labels. Leave as-is for now.
                }
            },
            ReproTapeEntry::Note { message, .. } => {
                *message = self.scrub_string(message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_string_redacts_key() {
        let cfg = ScrubConfig::default_shareable();
        let out = cfg.scrub_string("token=sk-abc1234567890XYZ");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("sk-abc1234567890XYZ"));
    }
}
