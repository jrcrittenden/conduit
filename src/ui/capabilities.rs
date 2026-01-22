use crate::agent::AgentType;

#[derive(Debug, Clone, Copy)]
pub struct AgentCapabilities {
    pub supports_plan_mode: bool,
    pub supports_interactive_input: bool,
    pub supports_steer: bool,
    pub supports_follow_up: bool,
}

impl AgentCapabilities {
    pub fn for_agent(agent_type: AgentType) -> Self {
        Self {
            supports_plan_mode: agent_type.supports_plan_mode(),
            supports_interactive_input: false,
            supports_steer: false,
            supports_follow_up: false,
        }
    }
}
