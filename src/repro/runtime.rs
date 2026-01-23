use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use std::sync::OnceLock;

use crate::repro::tape::{ReproTape, ReproTapeWriter};

const ENV_REPRO_MODE: &str = "CONDUIT_REPRO_MODE";
const ENV_REPRO_CONTINUE_LIVE: &str = "CONDUIT_REPRO_CONTINUE_LIVE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReproMode {
    Off,
    Record,
    /// Replay the tape on startup. If `continue_live` is true, replay runs once and then
    /// the app switches back to live mode for subsequent interactions.
    Replay {
        continue_live: bool,
    },
}

fn mode_cell() -> &'static Mutex<ReproMode> {
    static CELL: OnceLock<Mutex<ReproMode>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(ReproMode::Off))
}

pub fn mode() -> ReproMode {
    mode_cell().lock().clone()
}

pub fn set_mode(new_mode: ReproMode) {
    *mode_cell().lock() = new_mode;
}

pub fn is_recording() -> bool {
    matches!(mode(), ReproMode::Record)
}

pub fn is_replay() -> bool {
    matches!(mode(), ReproMode::Replay { .. })
}

pub fn continue_live_after_replay() -> bool {
    match mode() {
        ReproMode::Replay { continue_live } => continue_live,
        _ => false,
    }
}

pub fn init_from_env() {
    let raw = std::env::var(ENV_REPRO_MODE).unwrap_or_default();
    let mode = match raw.trim().to_ascii_lowercase().as_str() {
        "record" => ReproMode::Record,
        "replay" => {
            let continue_live = std::env::var(ENV_REPRO_CONTINUE_LIVE)
                .ok()
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
            ReproMode::Replay { continue_live }
        }
        _ => ReproMode::Off,
    };
    set_mode(mode);
}

pub fn tape_path() -> PathBuf {
    crate::util::data_dir()
        .join(crate::repro::bundle::REPRO_DIRNAME)
        .join("tape.jsonl")
}

fn writer_cell() -> &'static Mutex<Option<Arc<ReproTapeWriter>>> {
    static CELL: OnceLock<Mutex<Option<Arc<ReproTapeWriter>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn recording_writer() -> anyhow::Result<Option<Arc<ReproTapeWriter>>> {
    if !is_recording() {
        return Ok(None);
    }

    let mut guard = writer_cell().lock();
    if let Some(existing) = guard.as_ref() {
        return Ok(Some(existing.clone()));
    }

    let repro_dir = crate::util::data_dir().join(crate::repro::bundle::REPRO_DIRNAME);
    std::fs::create_dir_all(&repro_dir)?;
    let path = tape_path();
    let writer = Arc::new(ReproTapeWriter::create(&path)?);
    *guard = Some(writer.clone());
    Ok(Some(writer))
}

fn tape_cell() -> &'static Mutex<Option<Arc<ReproTape>>> {
    static CELL: OnceLock<Mutex<Option<Arc<ReproTape>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn replay_tape() -> anyhow::Result<Option<Arc<ReproTape>>> {
    if !is_replay() {
        return Ok(None);
    }

    let mut guard = tape_cell().lock();
    if let Some(existing) = guard.as_ref() {
        return Ok(Some(existing.clone()));
    }

    let path = tape_path();
    if !path.exists() {
        return Ok(None);
    }

    let tape = Arc::new(ReproTape::read_jsonl_from_path(&path)?);
    *guard = Some(tape.clone());
    Ok(Some(tape))
}
