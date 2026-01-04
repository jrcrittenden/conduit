use std::path::PathBuf;

use crate::agent::{AgentStartConfig, AgentType};
use uuid::Uuid;

/// Side effects that should be executed outside the reducer.
pub enum Effect {
    SaveSessionState,
    StartAgent {
        tab_index: usize,
        agent_type: AgentType,
        config: AgentStartConfig,
    },
    PrPreflight {
        tab_index: usize,
        working_dir: PathBuf,
    },
    OpenPrInBrowser {
        working_dir: PathBuf,
    },
    DumpDebugState,
    CreateWorkspace {
        repo_id: Uuid,
    },
    ArchiveWorkspace {
        workspace_id: Uuid,
    },
    RemoveProject {
        repo_id: Uuid,
    },
    CopyToClipboard(String),
}
