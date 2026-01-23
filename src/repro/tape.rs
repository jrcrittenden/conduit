use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::agent::events::AgentEvent;

pub const REPRO_TAPE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordedInput {
    ClaudeJsonl {
        jsonl: String,
    },
    CodexPrompt {
        text: String,
        images: Vec<String>,
    },
    OpencodeQuestion {
        request_id: String,
        answers: Option<Vec<Vec<String>>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReproTapeEntry {
    AgentEvent {
        seq: u64,
        ts_ms: u64,
        session_id: String,
        event: AgentEvent,
    },
    AgentInput {
        seq: u64,
        ts_ms: u64,
        session_id: String,
        input: RecordedInput,
    },
    Note {
        seq: u64,
        ts_ms: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReproTapeJsonlLine {
    Header {
        schema_version: u32,
        created_at_ms: u64,
    },
    Entry {
        entry: ReproTapeEntry,
    },
}

#[derive(Debug, Clone)]
pub struct ReproTape {
    pub schema_version: u32,
    pub created_at_ms: u64,
    pub entries: Vec<ReproTapeEntry>,
}

impl ReproTape {
    pub fn new() -> Self {
        Self {
            schema_version: REPRO_TAPE_SCHEMA_VERSION,
            created_at_ms: now_ms(),
            entries: Vec::new(),
        }
    }

    pub fn write_jsonl_to_path(&self, path: &Path) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        let header = ReproTapeJsonlLine::Header {
            schema_version: self.schema_version,
            created_at_ms: self.created_at_ms,
        };
        writeln!(
            writer,
            "{}",
            serde_json::to_string(&header).map_err(io::Error::other)?
        )?;
        for entry in &self.entries {
            let line = ReproTapeJsonlLine::Entry {
                entry: entry.clone(),
            };
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&line).map_err(io::Error::other)?
            )?;
        }
        writer.flush()?;
        Ok(())
    }

    pub fn read_jsonl_from_path(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut schema_version: Option<u32> = None;
        let mut created_at_ms: Option<u64> = None;
        let mut entries = Vec::new();

        for (idx, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let parsed: ReproTapeJsonlLine =
                serde_json::from_str(&line).map_err(|e| io::Error::other(format!("{e}")))?;
            match parsed {
                ReproTapeJsonlLine::Header {
                    schema_version: v,
                    created_at_ms: t,
                } => {
                    if idx != 0 {
                        return Err(io::Error::other("tape header must be the first JSONL line"));
                    }
                    schema_version = Some(v);
                    created_at_ms = Some(t);
                }
                ReproTapeJsonlLine::Entry { entry } => {
                    entries.push(entry);
                }
            }
        }

        let schema_version =
            schema_version.ok_or_else(|| io::Error::other("missing tape header"))?;
        let created_at_ms =
            created_at_ms.ok_or_else(|| io::Error::other("missing tape header timestamp"))?;

        Ok(Self {
            schema_version,
            created_at_ms,
            entries,
        })
    }
}

impl Default for ReproTape {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReproTapeWriter {
    schema_version: u32,
    created_at_ms: u64,
    seq: AtomicU64,
    writer: Mutex<BufWriter<File>>,
}

impl ReproTapeWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        let created_at_ms = now_ms();
        let header = ReproTapeJsonlLine::Header {
            schema_version: REPRO_TAPE_SCHEMA_VERSION,
            created_at_ms,
        };
        writeln!(
            writer,
            "{}",
            serde_json::to_string(&header).map_err(io::Error::other)?
        )?;
        writer.flush()?;
        Ok(Self {
            schema_version: REPRO_TAPE_SCHEMA_VERSION,
            created_at_ms,
            seq: AtomicU64::new(1),
            writer: Mutex::new(writer),
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    pub fn append(&self, entry: ReproTapeEntry) -> io::Result<()> {
        let line = ReproTapeJsonlLine::Entry { entry };
        let json = serde_json::to_string(&line).map_err(io::Error::other)?;
        let mut writer = self.writer.lock();
        writeln!(writer, "{json}")?;
        writer.flush()?;
        Ok(())
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::{AgentEvent, AssistantMessageEvent};
    use tempfile::tempdir;

    #[test]
    fn tape_jsonl_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tape.jsonl");

        let mut tape = ReproTape::new();
        tape.entries.push(ReproTapeEntry::AgentEvent {
            seq: 1,
            ts_ms: 123,
            session_id: "session-1".to_string(),
            event: AgentEvent::AssistantMessage(AssistantMessageEvent {
                text: "hi".to_string(),
                is_final: true,
            }),
        });
        tape.entries.push(ReproTapeEntry::AgentInput {
            seq: 2,
            ts_ms: 124,
            session_id: "session-1".to_string(),
            input: RecordedInput::ClaudeJsonl {
                jsonl: "{\"foo\":1}\n".to_string(),
            },
        });

        tape.write_jsonl_to_path(&path).unwrap();
        let read = ReproTape::read_jsonl_from_path(&path).unwrap();

        assert_eq!(read.schema_version, REPRO_TAPE_SCHEMA_VERSION);
        assert_eq!(read.entries.len(), 2);
    }

    #[test]
    fn tape_writer_writes_header_and_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tape.jsonl");

        let writer = ReproTapeWriter::create(&path).unwrap();
        let seq = writer.next_seq();
        writer
            .append(ReproTapeEntry::Note {
                seq,
                ts_ms: 1,
                message: "hello".to_string(),
            })
            .unwrap();

        let read = ReproTape::read_jsonl_from_path(&path).unwrap();
        assert_eq!(read.schema_version, REPRO_TAPE_SCHEMA_VERSION);
        assert_eq!(read.entries.len(), 1);
    }
}
