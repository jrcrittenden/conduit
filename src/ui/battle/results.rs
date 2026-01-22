//! Battle results generation and sharing

use std::time::Duration;

use crate::agent::AgentType;

use super::BattleSession;

/// Aggregated battle results
#[derive(Debug, Clone)]
pub struct BattleResults {
    /// The prompt that was battled
    pub prompt: String,

    /// Winner (if any)
    pub winner: Option<AgentType>,

    /// Winning margin
    pub margin: Option<Duration>,

    /// Left agent (Claude) stats
    pub left: AgentStats,

    /// Right agent (Codex) stats
    pub right: AgentStats,
}

/// Stats for one agent
#[derive(Debug, Clone)]
pub struct AgentStats {
    /// Agent type
    pub agent_type: AgentType,

    /// Model used
    pub model: Option<String>,

    /// Completion time
    pub time: Option<Duration>,

    /// Input tokens
    pub input_tokens: i64,

    /// Output tokens
    pub output_tokens: i64,

    /// Estimated cost
    pub cost: f64,

    /// Files modified
    pub files_modified: usize,

    /// Tool calls made
    pub tool_calls: usize,

    /// Error message if failed
    pub error: Option<String>,
}

impl BattleResults {
    /// Generate results from a battle session
    pub fn from_session(session: &BattleSession) -> Self {
        Self {
            prompt: session.prompt.clone(),
            winner: session.state.winner(),
            margin: session.state.margin(),
            left: AgentStats {
                agent_type: AgentType::Claude,
                model: session.left.model.clone(),
                time: session.left.completion_time(),
                input_tokens: session.left.usage.input_tokens,
                output_tokens: session.left.usage.output_tokens,
                cost: session.left.estimated_cost(),
                files_modified: session.left.files_modified.len(),
                tool_calls: session.left.tool_calls,
                error: session.left.error.clone(),
            },
            right: AgentStats {
                agent_type: AgentType::Codex,
                model: session.right.model.clone(),
                time: session.right.completion_time(),
                input_tokens: session.right.usage.input_tokens,
                output_tokens: session.right.usage.output_tokens,
                cost: session.right.estimated_cost(),
                files_modified: session.right.files_modified.len(),
                tool_calls: session.right.tool_calls,
                error: session.right.error.clone(),
            },
        }
    }

    /// Get stats for winner
    pub fn winner_stats(&self) -> Option<&AgentStats> {
        self.winner.map(|w| match w {
            AgentType::Claude => &self.left,
            AgentType::Codex => &self.right,
        })
    }

    /// Get stats for loser
    pub fn loser_stats(&self) -> Option<&AgentStats> {
        self.winner.map(|w| match w {
            AgentType::Claude => &self.right,
            AgentType::Codex => &self.left,
        })
    }
}

impl AgentStats {
    /// Format time for display
    pub fn time_display(&self) -> String {
        match self.time {
            Some(d) => format!("{:.1}s", d.as_secs_f64()),
            None => "DNF".to_string(),
        }
    }

    /// Format cost for display
    pub fn cost_display(&self) -> String {
        format!("${:.3}", self.cost)
    }

    /// Format tokens for display
    pub fn tokens_display(&self) -> String {
        format!("{}in/{}out", self.input_tokens, self.output_tokens)
    }

    /// Get display name
    pub fn display_name(&self) -> &'static str {
        match self.agent_type {
            AgentType::Claude => "Claude Code",
            AgentType::Codex => "Codex CLI",
        }
    }

    /// Get short name for sharing
    pub fn short_name(&self) -> &'static str {
        match self.agent_type {
            AgentType::Claude => "Claude",
            AgentType::Codex => "Codex",
        }
    }
}

/// Shareable text format for results
pub struct ShareableResults {
    /// Plain text version
    pub text: String,
    /// Markdown version
    pub markdown: String,
}

impl ShareableResults {
    /// Generate shareable results from battle results
    pub fn from_results(results: &BattleResults) -> Self {
        let text = Self::generate_text(results);
        let markdown = Self::generate_markdown(results);
        Self { text, markdown }
    }

    fn generate_text(results: &BattleResults) -> String {
        let mut lines = vec!["⚔️ AI BATTLE RESULTS ⚔️".to_string(), String::new()];

        // Truncate prompt for display
        let prompt_display = if results.prompt.len() > 50 {
            format!("{}...", &results.prompt[..47])
        } else {
            results.prompt.clone()
        };
        lines.push(format!("Prompt: \"{}\"", prompt_display));
        lines.push(String::new());

        // Winner/Loser stats
        if let (Some(winner), Some(loser)) = (results.winner_stats(), results.loser_stats()) {
            let model_suffix = winner
                .model
                .as_ref()
                .map(|m| format!(" ({})", Self::short_model(m)))
                .unwrap_or_default();

            lines.push(format!("🥇 {}{}", winner.display_name(), model_suffix));
            lines.push(format!(
                "   ⏱️ {} | 💰 {} | 📁 {} files",
                winner.time_display(),
                winner.cost_display(),
                winner.files_modified
            ));
            lines.push(String::new());

            let model_suffix = loser
                .model
                .as_ref()
                .map(|m| format!(" ({})", Self::short_model(m)))
                .unwrap_or_default();

            lines.push(format!("🥈 {}{}", loser.display_name(), model_suffix));
            lines.push(format!(
                "   ⏱️ {} | 💰 {} | 📁 {} files",
                loser.time_display(),
                loser.cost_display(),
                loser.files_modified
            ));
            lines.push(String::new());

            // Winner summary
            if let Some(margin) = results.margin {
                if margin.as_secs() > 0 {
                    lines.push(format!(
                        "Winner: {} by {:.1}s ⚡",
                        winner.short_name(),
                        margin.as_secs_f64()
                    ));
                } else {
                    lines.push(format!("Winner: {} ⚡", winner.short_name()));
                }
            }
        } else {
            // No clear winner (both failed?)
            lines.push("No winner - both agents encountered errors".to_string());
        }

        lines.push(String::new());
        lines.push("Built with Conduit ⚡".to_string());
        lines.push("#AIBattle #ClaudeVsGPT #DevTools".to_string());

        lines.join("\n")
    }

    fn generate_markdown(results: &BattleResults) -> String {
        let mut lines = vec!["## ⚔️ AI Battle Results".to_string(), String::new()];

        // Prompt
        lines.push(format!("**Prompt:** \"{}\"", results.prompt));
        lines.push(String::new());

        // Table header
        lines.push("| Agent | Time | Cost | Files | Tools |".to_string());
        lines.push("|-------|------|------|-------|-------|".to_string());

        // Left agent row
        let left_medal = if results.winner == Some(AgentType::Claude) {
            "🥇 "
        } else {
            "🥈 "
        };
        lines.push(format!(
            "| {}{} | {} | {} | {} | {} |",
            left_medal,
            results.left.display_name(),
            results.left.time_display(),
            results.left.cost_display(),
            results.left.files_modified,
            results.left.tool_calls
        ));

        // Right agent row
        let right_medal = if results.winner == Some(AgentType::Codex) {
            "🥇 "
        } else {
            "🥈 "
        };
        lines.push(format!(
            "| {}{} | {} | {} | {} | {} |",
            right_medal,
            results.right.display_name(),
            results.right.time_display(),
            results.right.cost_display(),
            results.right.files_modified,
            results.right.tool_calls
        ));

        lines.push(String::new());

        // Winner summary
        if let Some(winner) = results.winner_stats() {
            if let Some(margin) = results.margin {
                lines.push(format!(
                    "**Winner:** {} by {:.1}s",
                    winner.display_name(),
                    margin.as_secs_f64()
                ));
            } else {
                lines.push(format!("**Winner:** {}", winner.display_name()));
            }
        }

        lines.join("\n")
    }

    /// Shorten model name for display
    fn short_model(model: &str) -> &str {
        // Extract just the model variant
        if model.contains("opus") {
            "Opus"
        } else if model.contains("sonnet") {
            "Sonnet"
        } else if model.contains("haiku") {
            "Haiku"
        } else if model.contains("gpt-5") {
            "GPT-5"
        } else if model.contains("gpt-4") {
            "GPT-4"
        } else {
            model
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shareable_text_generation() {
        let results = BattleResults {
            prompt: "Add a login form".to_string(),
            winner: Some(AgentType::Claude),
            margin: Some(Duration::from_secs(5)),
            left: AgentStats {
                agent_type: AgentType::Claude,
                model: Some("claude-sonnet-4".to_string()),
                time: Some(Duration::from_secs(23)),
                input_tokens: 1000,
                output_tokens: 500,
                cost: 0.023,
                files_modified: 3,
                tool_calls: 8,
                error: None,
            },
            right: AgentStats {
                agent_type: AgentType::Codex,
                model: Some("gpt-5.2-codex".to_string()),
                time: Some(Duration::from_secs(28)),
                input_tokens: 1200,
                output_tokens: 600,
                cost: 0.034,
                files_modified: 2,
                tool_calls: 10,
                error: None,
            },
        };

        let shareable = ShareableResults::from_results(&results);

        assert!(shareable.text.contains("AI BATTLE RESULTS"));
        assert!(shareable.text.contains("Claude Code"));
        assert!(shareable.text.contains("🥇"));
        assert!(shareable.text.contains("Winner: Claude"));
    }
}
