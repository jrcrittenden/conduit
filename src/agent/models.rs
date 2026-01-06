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
    /// Whether this is a new model (shows badge)
    pub is_new: bool,
    /// Agent type this model belongs to
    pub agent_type: AgentType,
}

impl ModelInfo {
    pub fn new(
        agent_type: AgentType,
        id: &str,
        display_name: &str,
        alias: &str,
        description: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            display_name: display_name.to_string(),
            alias: alias.to_string(),
            description: description.to_string(),
            is_new: false,
            agent_type,
        }
    }

    pub fn with_new_badge(mut self) -> Self {
        self.is_new = true;
        self
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
                AgentType::Claude,
                "opus",
                "Opus 4.5",
                "opus",
                "Most powerful, best for complex reasoning",
            ),
            ModelInfo::new(
                AgentType::Claude,
                "sonnet",
                "Sonnet 4.5",
                "sonnet",
                "Fast and capable, best for most tasks",
            ),
            ModelInfo::new(
                AgentType::Claude,
                "haiku",
                "Haiku 4.5",
                "haiku",
                "Fastest, great for simple tasks",
            ),
        ]
    }

    /// Get available models for Codex CLI
    pub fn codex_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo::new(
                AgentType::Codex,
                "gpt-5.2-codex",
                "GPT-5.2-Codex",
                "gpt-5.2-codex",
                "Latest Codex model",
            )
            .with_new_badge(),
            ModelInfo::new(
                AgentType::Codex,
                "gpt-5.2",
                "GPT-5.2",
                "gpt-5.2",
                "Fast and efficient",
            ),
            ModelInfo::new(
                AgentType::Codex,
                "gpt-5.1-codex-max",
                "GPT-5.1-Codex-Max",
                "gpt-5.1-codex-max",
                "Maximum capability",
            ),
        ]
    }

    /// Get all models grouped by agent type
    pub fn all_models() -> Vec<ModelInfo> {
        let mut models = Self::claude_models();
        models.extend(Self::codex_models());
        models
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
            AgentType::Claude => "opus".to_string(),
            AgentType::Codex => "gpt-5.2-codex".to_string(),
        }
    }

    /// Find a model by ID or alias
    pub fn find_model(agent_type: AgentType, id_or_alias: &str) -> Option<ModelInfo> {
        Self::models_for(agent_type)
            .into_iter()
            .find(|m| m.id == id_or_alias || m.alias == id_or_alias)
    }

    /// Get the icon for an agent type
    pub fn agent_icon(agent_type: AgentType) -> &'static str {
        match agent_type {
            AgentType::Claude => "✻",
            AgentType::Codex => "◎",
        }
    }

    /// Get the section title for an agent type
    pub fn agent_section_title(agent_type: AgentType) -> &'static str {
        match agent_type {
            AgentType::Claude => "Claude Code",
            AgentType::Codex => "Codex",
        }
    }
}
