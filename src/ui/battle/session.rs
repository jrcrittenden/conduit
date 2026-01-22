//! Battle session - manages a head-to-head agent competition

use std::path::PathBuf;
use std::time::Instant;

use uuid::Uuid;

use super::{BattleAgent, BattleResults, BattleState};
use crate::agent::AgentType;

/// A battle session pitting two agents against each other
pub struct BattleSession {
    /// Unique identifier for this battle
    pub id: Uuid,

    /// The prompt both agents are competing on
    pub prompt: String,

    /// Left agent (Claude)
    pub left: BattleAgent,

    /// Right agent (Codex)
    pub right: BattleAgent,

    /// Working directory for both agents
    pub working_dir: PathBuf,

    /// Project name for display
    pub project_name: Option<String>,

    /// Current battle state
    pub state: BattleState,

    /// Battle start time
    pub started_at: Option<Instant>,

    /// Whether this tab needs attention (new content while unfocused)
    pub needs_attention: bool,
}

impl BattleSession {
    /// Create a new battle session
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            prompt: String::new(),
            left: BattleAgent::new(AgentType::Claude),
            right: BattleAgent::new(AgentType::Codex),
            working_dir,
            project_name: None,
            state: BattleState::Idle,
            started_at: None,
            needs_attention: false,
        }
    }

    /// Create with project name
    pub fn with_project_name(working_dir: PathBuf, project_name: String) -> Self {
        let mut session = Self::new(working_dir);
        session.project_name = Some(project_name);
        session
    }

    /// Get tab display name
    pub fn tab_name(&self) -> String {
        let base = self
            .project_name
            .clone()
            .unwrap_or_else(|| "Battle".to_string());
        format!("⚔️ {}", base)
    }

    /// Start the battle with a prompt
    pub fn start(&mut self, prompt: String) {
        self.prompt = prompt;
        self.state = BattleState::Racing;
        self.started_at = Some(Instant::now());

        // Start both agents
        self.left.start_processing();
        self.right.start_processing();
    }

    /// Get elapsed battle time
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    /// Format elapsed time as MM:SS
    pub fn elapsed_display(&self) -> String {
        let secs = self.elapsed_secs();
        let mins = secs / 60;
        let secs = secs % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    /// Get agent by type
    pub fn agent(&self, agent_type: AgentType) -> &BattleAgent {
        match agent_type {
            AgentType::Claude => &self.left,
            AgentType::Codex => &self.right,
        }
    }

    /// Get mutable agent by type
    pub fn agent_mut(&mut self, agent_type: AgentType) -> &mut BattleAgent {
        match agent_type {
            AgentType::Claude => &mut self.left,
            AgentType::Codex => &mut self.right,
        }
    }

    /// Find agent by session ID
    pub fn agent_by_session_id(&self, session_id: &str) -> Option<&BattleAgent> {
        if self
            .left
            .session_id
            .as_ref()
            .map(|s| s.as_str() == session_id)
            .unwrap_or(false)
        {
            return Some(&self.left);
        }
        if self
            .right
            .session_id
            .as_ref()
            .map(|s| s.as_str() == session_id)
            .unwrap_or(false)
        {
            return Some(&self.right);
        }
        None
    }

    /// Find mutable agent by session ID
    pub fn agent_by_session_id_mut(&mut self, session_id: &str) -> Option<&mut BattleAgent> {
        if self
            .left
            .session_id
            .as_ref()
            .map(|s| s.as_str() == session_id)
            .unwrap_or(false)
        {
            return Some(&mut self.left);
        }
        if self
            .right
            .session_id
            .as_ref()
            .map(|s| s.as_str() == session_id)
            .unwrap_or(false)
        {
            return Some(&mut self.right);
        }
        None
    }

    /// Mark an agent as complete and update battle state
    pub fn mark_complete(&mut self, agent_type: AgentType) {
        let agent = self.agent_mut(agent_type);
        agent.complete();

        let other_type = match agent_type {
            AgentType::Claude => AgentType::Codex,
            AgentType::Codex => AgentType::Claude,
        };

        // Update battle state based on other agent
        let other = self.agent(other_type);
        if other.is_complete() {
            // Both complete - determine winner
            let left_time = self.left.completion_time().unwrap();
            let right_time = self.right.completion_time().unwrap();

            let (winner, margin) = if left_time < right_time {
                (AgentType::Claude, right_time - left_time)
            } else {
                (AgentType::Codex, left_time - right_time)
            };

            self.state = BattleState::Completed { winner, margin };
        } else if other.has_error() {
            // Other already failed, we win
            self.state = BattleState::Completed {
                winner: agent_type,
                margin: std::time::Duration::ZERO,
            };
        } else {
            // We finished first, waiting for other
            self.state = BattleState::OneComplete {
                first: agent_type,
                time: self.agent(agent_type).completion_time().unwrap(),
            };
        }
    }

    /// Mark an agent as failed and update battle state
    pub fn mark_failed(&mut self, agent_type: AgentType, error: String) {
        let agent = self.agent_mut(agent_type);
        agent.fail(error.clone());

        let other_type = match agent_type {
            AgentType::Claude => AgentType::Codex,
            AgentType::Codex => AgentType::Claude,
        };

        let other = self.agent(other_type);
        if other.is_complete() {
            // Other already completed, they win
            self.state = BattleState::Completed {
                winner: other_type,
                margin: std::time::Duration::ZERO,
            };
        } else if other.has_error() {
            // Both failed
            let left_err = self.left.error.clone().unwrap_or_default();
            let right_err = self.right.error.clone().unwrap_or_default();
            self.state = BattleState::BothErrored {
                left_error: left_err,
                right_error: right_err,
            };
        } else {
            // We failed, other still running
            self.state = BattleState::Error {
                failed: agent_type,
                error,
                other_running: true,
            };
        }
    }

    /// Check if battle is still in progress
    pub fn is_racing(&self) -> bool {
        self.state.is_racing()
    }

    /// Check if battle has concluded
    pub fn is_finished(&self) -> bool {
        self.state.is_finished()
    }

    /// Generate results for display/sharing
    pub fn results(&self) -> Option<BattleResults> {
        if !self.is_finished() {
            return None;
        }
        Some(BattleResults::from_session(self))
    }

    /// Advance animations
    pub fn tick(&mut self) {
        self.left.tick();
        self.right.tick();
    }

    /// Show results view
    pub fn view_results(&mut self) {
        if self.is_finished() {
            self.state = BattleState::ViewingResults;
        }
    }

    /// Exit results view back to completed state
    pub fn dismiss_results(&mut self) {
        if let BattleState::ViewingResults = self.state {
            // Restore to Completed state based on agents
            if let (Some(left_time), Some(right_time)) =
                (self.left.completion_time(), self.right.completion_time())
            {
                let (winner, margin) = if left_time < right_time {
                    (AgentType::Claude, right_time - left_time)
                } else {
                    (AgentType::Codex, left_time - right_time)
                };
                self.state = BattleState::Completed { winner, margin };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_new_session() {
        let session = BattleSession::new(PathBuf::from("/test"));
        assert!(matches!(session.state, BattleState::Idle));
        assert_eq!(session.left.agent_type, AgentType::Claude);
        assert_eq!(session.right.agent_type, AgentType::Codex);
    }

    #[test]
    fn test_start_battle() {
        let mut session = BattleSession::new(PathBuf::from("/test"));
        session.start("test prompt".into());

        assert!(matches!(session.state, BattleState::Racing));
        assert!(session.left.is_processing);
        assert!(session.right.is_processing);
        assert_eq!(session.prompt, "test prompt");
    }

    #[test]
    fn test_claude_wins() {
        let mut session = BattleSession::new(PathBuf::from("/test"));
        session.start("test".into());

        // Claude finishes first
        session.mark_complete(AgentType::Claude);
        assert!(matches!(
            session.state,
            BattleState::OneComplete {
                first: AgentType::Claude,
                ..
            }
        ));

        // Codex finishes second
        session.mark_complete(AgentType::Codex);
        assert!(matches!(
            session.state,
            BattleState::Completed {
                winner: AgentType::Claude,
                ..
            }
        ));
    }

    #[test]
    fn test_one_fails() {
        let mut session = BattleSession::new(PathBuf::from("/test"));
        session.start("test".into());

        // Claude fails
        session.mark_failed(AgentType::Claude, "error".into());
        assert!(matches!(
            session.state,
            BattleState::Error {
                failed: AgentType::Claude,
                other_running: true,
                ..
            }
        ));

        // Codex completes, wins by default
        session.mark_complete(AgentType::Codex);
        assert!(matches!(
            session.state,
            BattleState::Completed {
                winner: AgentType::Codex,
                ..
            }
        ));
    }
}
