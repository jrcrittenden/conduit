use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::agent::error::AgentError;
use crate::agent::events::{
    AgentEvent, AssistantMessageEvent, CommandOutputEvent, ErrorEvent, SessionInitEvent,
    TokenUsage, TurnCompletedEvent, TurnFailedEvent,
};
use crate::agent::runner::{AgentHandle, AgentRunner, AgentStartConfig, AgentType};
use crate::agent::session::SessionId;
use crate::agent::stream::{CodexRawEvent, JsonlStreamParser};

pub struct CodexCliRunner {
    binary_path: PathBuf,
}

impl CodexCliRunner {
    pub fn new() -> Self {
        Self {
            binary_path: Self::find_binary().unwrap_or_else(|| PathBuf::from("codex")),
        }
    }

    fn find_binary() -> Option<PathBuf> {
        which::which("codex").ok()
    }

    fn build_command(&self, config: &AgentStartConfig) -> Command {
        let mut cmd = Command::new(&self.binary_path);

        // Use exec subcommand for headless mode
        if let Some(session_id) = &config.resume_session {
            // Resume existing session
            cmd.arg("exec").arg("resume").arg(session_id.as_str());
            if !config.prompt.is_empty() {
                cmd.arg(&config.prompt);
            }
        } else {
            // New session
            cmd.arg("exec");
            cmd.arg(&config.prompt);
        }

        // JSON output mode
        cmd.arg("--json");

        // Full auto mode (no approval prompts)
        cmd.arg("--full-auto");

        // Model selection
        if let Some(model) = &config.model {
            cmd.arg("-m").arg(model);
        }

        // Working directory
        cmd.current_dir(&config.working_dir);

        // Additional args
        for arg in &config.additional_args {
            cmd.arg(arg);
        }

        // Stdio setup
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        cmd
    }

    /// Convert Codex-specific event to unified AgentEvent
    fn convert_event(raw: CodexRawEvent) -> Option<AgentEvent> {
        match raw {
            CodexRawEvent::ThreadStarted { thread_id } => {
                Some(AgentEvent::SessionInit(SessionInitEvent {
                    session_id: SessionId::from_string(thread_id),
                    model: None,
                }))
            }
            CodexRawEvent::TurnStarted => Some(AgentEvent::TurnStarted),
            CodexRawEvent::TurnCompleted { usage } => {
                Some(AgentEvent::TurnCompleted(TurnCompletedEvent {
                    usage: TokenUsage {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        cached_tokens: usage.cached_input_tokens,
                        total_tokens: usage.input_tokens + usage.output_tokens,
                    },
                }))
            }
            CodexRawEvent::TurnFailed { error } => {
                Some(AgentEvent::TurnFailed(TurnFailedEvent {
                    error: error.message,
                }))
            }
            CodexRawEvent::ItemCompleted { item } => Self::convert_item_event(&item),
            CodexRawEvent::ItemUpdated { item } => {
                // For streaming updates, check if it's a message
                Self::convert_item_event(&item)
            }
            CodexRawEvent::Error { message } => Some(AgentEvent::Error(ErrorEvent {
                message,
                is_fatal: true,
            })),
            CodexRawEvent::ItemStarted { .. } | CodexRawEvent::Unknown => None,
        }
    }

    fn convert_item_event(item: &crate::agent::stream::CodexThreadItem) -> Option<AgentEvent> {
        let item_type = item.item_type.as_deref()?;

        match item_type {
            "agent_message" | "message" => {
                let text = item
                    .details
                    .get("text")
                    .or_else(|| item.details.get("content"))
                    .and_then(|v| v.as_str())?;
                Some(AgentEvent::AssistantMessage(AssistantMessageEvent {
                    text: text.to_string(),
                    is_final: true,
                }))
            }
            "command_execution" | "local_shell_call" => {
                let command = item
                    .details
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let output = item
                    .details
                    .get("aggregated_output")
                    .or_else(|| item.details.get("output"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let exit_code = item
                    .details
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                Some(AgentEvent::CommandOutput(CommandOutputEvent {
                    command: command.to_string(),
                    output: output.to_string(),
                    exit_code,
                    is_streaming: false,
                }))
            }
            _ => None,
        }
    }
}

impl Default for CodexCliRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRunner for CodexCliRunner {
    fn agent_type(&self) -> AgentType {
        AgentType::Codex
    }

    async fn start(&self, config: AgentStartConfig) -> Result<AgentHandle, AgentError> {
        let mut cmd = self.build_command(&config);
        let mut child = cmd.spawn()?;

        let pid = child.id().ok_or(AgentError::ProcessSpawnFailed)?;
        let stdout = child.stdout.take().ok_or(AgentError::StdoutCaptureFailed)?;

        let (tx, rx) = mpsc::channel::<AgentEvent>(256);

        // Spawn JSONL parser task
        tokio::spawn(async move {
            let (raw_tx, mut raw_rx) = mpsc::channel::<CodexRawEvent>(256);

            let parse_handle = tokio::spawn(async move {
                let _ = JsonlStreamParser::parse_stream(stdout, raw_tx).await;
            });

            while let Some(raw_event) = raw_rx.recv().await {
                if let Some(event) = Self::convert_event(raw_event) {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            }

            let _ = parse_handle.await;
        });

        // Monitor process
        tokio::spawn(async move {
            let _ = child.wait().await;
        });

        Ok(AgentHandle::new(rx, pid))
    }

    async fn send_input(&self, _handle: &AgentHandle, _input: &str) -> Result<(), AgentError> {
        Err(AgentError::NotSupported(
            "Codex exec mode doesn't support interactive input".into(),
        ))
    }

    async fn stop(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(handle.pid as i32, libc::SIGTERM);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = handle;
            return Err(AgentError::NotSupported(
                "Stop not implemented on this platform".into(),
            ));
        }
        Ok(())
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(handle.pid as i32, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = handle;
            return Err(AgentError::NotSupported(
                "Kill not implemented on this platform".into(),
            ));
        }
        Ok(())
    }

    fn is_available(&self) -> bool {
        self.binary_path.exists() || Self::find_binary().is_some()
    }

    fn binary_path(&self) -> Option<PathBuf> {
        if self.binary_path.exists() {
            Some(self.binary_path.clone())
        } else {
            Self::find_binary()
        }
    }
}
