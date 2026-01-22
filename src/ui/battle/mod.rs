//! Battle Mode - Head-to-head AI agent competition
//!
//! This module implements Battle Mode, where two AI agents (Claude Code and Codex CLI)
//! compete to complete the same prompt. Features:
//! - Split-pane view showing both agents working simultaneously
//! - Real-time race metrics (time, tokens, cost)
//! - Winner detection and shareable results

mod agent;
mod results;
mod session;
mod state;

pub use agent::BattleAgent;
pub use results::{BattleResults, ShareableResults};
pub use session::BattleSession;
pub use state::BattleState;
