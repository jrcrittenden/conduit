//! Deterministic record/replay and state snapshot ("repro bundle") support.
//!
//! This module is intentionally scoped to *portable artifacts* and the minimal
//! building blocks needed to:
//! - capture app/agent state into a single file
//! - restore from that file deterministically (offline)
//!
//! Higher-level wiring (TUI/Web commands, runner wrappers, etc.) can build on
//! these primitives incrementally.

pub mod bundle;
pub mod runtime;
pub mod scrub;
pub mod tape;
pub mod ui_state;
