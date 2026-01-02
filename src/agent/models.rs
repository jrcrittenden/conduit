//! Model configuration and registry

use crate::agent::AgentType;

/// Information about a model
#[derive(Debug, Clone)]
pub struct ModelInfo {
    /// Internal model ID (passed to CLI)
    pub id: String,
    /// Display name for UI
    pub display_name: String,
    /// Short alias for quick selection
    pub alias: String,
    /// Description of model capabilities
    pub description: String,
}

impl ModelInfo {
    pub fn new(id: &str, display_name: &str, alias: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            alias: alias.to_string(),
            description: description.to_string(),
        }
    }
}

/// Registry of available models for each agent type
#[derive(Debug, Default)]
pub struct ModelRegistry;

impl ModelRegistry {
    /// Get available models for Claude Code
    pub fn claude_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo::new(
                "sonnet",
                "Claude Sonnet 4",
                "sonnet",
                "Fast and capable, best for most tasks",
            ),
            ModelInfo::new(
                "opus",
                "Claude Opus 4",
                "opus",
                "Most powerful, best for complex reasoning",
            ),
            ModelInfo::new(
                "haiku",
                "Claude Haiku 3.5",
                "haiku",
                "Fastest, great for simple tasks",
            ),
        ]
    }

    /// Get available models for Codex CLI
    pub fn codex_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo::new("o4-mini", "o4-mini", "o4-mini", "Fast and efficient"),
            ModelInfo::new("o3", "o3", "o3", "Most capable reasoning"),
            ModelInfo::new("gpt-4.1", "GPT-4.1", "gpt-4.1", "Balanced performance"),
        ]
    }

    /// Get models for a specific agent type
    pub fn models_for(agent_type: AgentType) -> Vec<ModelInfo> {
        match agent_type {
            AgentType::Claude => Self::claude_models(),
            AgentType::Codex => Self::codex_models(),
        }
    }

    /// Get the default model for an agent type
    pub fn default_model(agent_type: AgentType) -> String {
        match agent_type {
            AgentType::Claude => "sonnet".to_string(),
            AgentType::Codex => "o4-mini".to_string(),
        }
    }

    /// Find a model by ID or alias
    pub fn find_model(agent_type: AgentType, id_or_alias: &str) -> Option<ModelInfo> {
        Self::models_for(agent_type)
            .into_iter()
            .find(|m| m.id == id_or_alias || m.alias == id_or_alias)
    }
}
