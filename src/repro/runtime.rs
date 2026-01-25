use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use std::sync::OnceLock;
use tokio::sync::Notify;

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

const REPLAY_NO_LIMIT: u64 = u64::MAX;

#[derive(Debug, Clone, Default)]
pub struct ReplayStateSnapshot {
    pub paused: bool,
    pub current_seq: u64,
    pub max_seq: Option<u64>,
    pub total_events: u64,
}

#[derive(Debug, Clone)]
pub struct ReplayController {
    state: Arc<ReplayControllerState>,
}

#[derive(Debug)]
struct ReplayControllerState {
    paused: AtomicBool,
    max_seq: AtomicU64,
    current_seq: AtomicU64,
    total_events: AtomicU64,
    notify: Notify,
}

impl ReplayController {
    pub fn new(pause_at: Option<u64>, total_events: u64) -> Self {
        let max_seq = pause_at.unwrap_or(REPLAY_NO_LIMIT);
        let state = ReplayControllerState {
            paused: AtomicBool::new(false),
            max_seq: AtomicU64::new(max_seq),
            current_seq: AtomicU64::new(0),
            total_events: AtomicU64::new(total_events),
            notify: Notify::new(),
        };
        Self {
            state: Arc::new(state),
        }
    }

    pub fn pause(&self) {
        self.state.paused.store(true, Ordering::SeqCst);
        self.state.notify.notify_waiters();
    }

    pub fn resume(&self) {
        self.state.paused.store(false, Ordering::SeqCst);
        self.state.max_seq.store(REPLAY_NO_LIMIT, Ordering::SeqCst);
        self.state.notify.notify_waiters();
    }

    pub fn step(&self) {
        let next_seq = self
            .state
            .current_seq
            .load(Ordering::SeqCst)
            .saturating_add(1);
        self.state.max_seq.store(next_seq, Ordering::SeqCst);
        self.state.paused.store(false, Ordering::SeqCst);
        self.state.notify.notify_waiters();
    }

    pub fn seek_forward(&self, seq: u64) {
        self.state.max_seq.store(seq, Ordering::SeqCst);
        self.state.paused.store(false, Ordering::SeqCst);
        self.state.notify.notify_waiters();
    }

    pub async fn wait_for(&self, seq: u64) {
        loop {
            let paused = self.state.paused.load(Ordering::SeqCst);
            let max_seq = self.state.max_seq.load(Ordering::SeqCst);
            if !paused && seq <= max_seq {
                break;
            }

            if !paused && seq > max_seq {
                self.state.paused.store(true, Ordering::SeqCst);
            }

            self.state.notify.notified().await;
        }
    }

    pub fn mark_emitted(&self, seq: u64) {
        self.state.current_seq.store(seq, Ordering::SeqCst);
    }

    pub fn state_snapshot(&self) -> ReplayStateSnapshot {
        let max_seq = self.state.max_seq.load(Ordering::SeqCst);
        ReplayStateSnapshot {
            paused: self.state.paused.load(Ordering::SeqCst),
            current_seq: self.state.current_seq.load(Ordering::SeqCst),
            max_seq: if max_seq == REPLAY_NO_LIMIT {
                None
            } else {
                Some(max_seq)
            },
            total_events: self.state.total_events.load(Ordering::SeqCst),
        }
    }
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

fn replay_controller_cell() -> &'static Mutex<Option<ReplayController>> {
    static CELL: OnceLock<Mutex<Option<ReplayController>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn init_replay_controller(pause_at: Option<u64>, total_events: u64) {
    let controller = ReplayController::new(pause_at, total_events);
    *replay_controller_cell().lock() = Some(controller);
}

pub fn replay_controller() -> Option<ReplayController> {
    let mut guard = replay_controller_cell().lock();
    if guard.is_none() && is_replay() {
        if let Ok(Some(tape)) = replay_tape() {
            let total_events = tape
                .entries
                .iter()
                .map(|entry| match entry {
                    crate::repro::tape::ReproTapeEntry::AgentEvent { seq, .. }
                    | crate::repro::tape::ReproTapeEntry::AgentInput { seq, .. }
                    | crate::repro::tape::ReproTapeEntry::Note { seq, .. } => *seq,
                })
                .max()
                .unwrap_or_default();
            *guard = Some(ReplayController::new(None, total_events));
        }
    }
    guard.clone()
}

pub fn replay_state_snapshot() -> Option<ReplayStateSnapshot> {
    replay_controller().map(|controller| controller.state_snapshot())
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
