//! Battle state tracking

use std::time::Duration;

use crate::agent::AgentType;

/// Current state of a battle
#[derive(Debug, Clone, Default)]
pub enum BattleState {
    /// Waiting for user to submit prompt
    #[default]
    Idle,

    /// Battle countdown (3, 2, 1...)
    Countdown { remaining: u8 },

    /// Both agents are racing to complete
    Racing,

    /// One agent completed, waiting for the other
    OneComplete {
        /// Which agent finished first
        first: AgentType,
        /// Time it took
        time: Duration,
    },

    /// Both agents completed successfully
    Completed {
        /// The winning agent
        winner: AgentType,
        /// Time advantage of winner
        margin: Duration,
    },

    /// One agent errored out
    Error {
        /// Which agent failed
        failed: AgentType,
        /// Error message
        error: String,
        /// Whether the other agent is still running
        other_running: bool,
    },

    /// Both agents errored
    BothErrored {
        left_error: String,
        right_error: String,
    },

    /// User is viewing the results screen
    ViewingResults,
}

impl BattleState {
    /// Check if the battle is still in progress
    pub fn is_racing(&self) -> bool {
        matches!(self, BattleState::Racing | BattleState::OneComplete { .. })
    }

    /// Check if the battle has concluded
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            BattleState::Completed { .. }
                | BattleState::Error {
                    other_running: false,
                    ..
                }
                | BattleState::BothErrored { .. }
        )
    }

    /// Check if we're showing results
    pub fn is_viewing_results(&self) -> bool {
        matches!(self, BattleState::ViewingResults)
    }

    /// Get the winner if battle is complete
    pub fn winner(&self) -> Option<AgentType> {
        match self {
            BattleState::Completed { winner, .. } => Some(*winner),
            BattleState::Error {
                failed,
                other_running: false,
                ..
            } => {
                // The one that didn't fail wins by default
                Some(match failed {
                    AgentType::Claude => AgentType::Codex,
                    AgentType::Codex => AgentType::Claude,
                })
            }
            _ => None,
        }
    }

    /// Get winning margin if available
    pub fn margin(&self) -> Option<Duration> {
        match self {
            BattleState::Completed { margin, .. } => Some(*margin),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_racing_states() {
        assert!(BattleState::Racing.is_racing());
        assert!(BattleState::OneComplete {
            first: AgentType::Claude,
            time: Duration::from_secs(10)
        }
        .is_racing());
        assert!(!BattleState::Idle.is_racing());
        assert!(!BattleState::Completed {
            winner: AgentType::Claude,
            margin: Duration::from_secs(5)
        }
        .is_racing());
    }

    #[test]
    fn test_winner_detection() {
        let completed = BattleState::Completed {
            winner: AgentType::Codex,
            margin: Duration::from_secs(3),
        };
        assert_eq!(completed.winner(), Some(AgentType::Codex));

        // When one fails, other wins
        let error = BattleState::Error {
            failed: AgentType::Claude,
            error: "test".into(),
            other_running: false,
        };
        assert_eq!(error.winner(), Some(AgentType::Codex));
    }
}
