use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::error::AgentError;
use crate::agent::events::AgentEvent;
use crate::agent::runner::{AgentHandle, AgentInput, AgentRunner, AgentStartConfig, AgentType};
use crate::repro::runtime;
use crate::repro::tape::{ReproTape, ReproTapeEntry};

pub struct ReplayAgentRunner {
    session_id: String,
    agent_type: AgentType,
    tape: Arc<ReproTape>,
}

impl ReplayAgentRunner {
    pub fn new(session_id: Uuid, agent_type: AgentType, tape: Arc<ReproTape>) -> Self {
        Self {
            session_id: session_id.to_string(),
            agent_type,
            tape,
        }
    }

    fn events_for_session(&self) -> Vec<(u64, AgentEvent)> {
        self.tape
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ReproTapeEntry::AgentEvent {
                    seq,
                    session_id,
                    event,
                    ..
                } if session_id == &self.session_id => Some((*seq, event.clone())),
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl AgentRunner for ReplayAgentRunner {
    fn agent_type(&self) -> AgentType {
        self.agent_type
    }

    async fn start(&self, _config: AgentStartConfig) -> Result<AgentHandle, AgentError> {
        let (tx, rx) = mpsc::channel::<AgentEvent>(256);
        let events = self.events_for_session();
        let controller = runtime::replay_controller();

        tokio::spawn(async move {
            for (seq, ev) in events {
                if let Some(ctrl) = controller.as_ref() {
                    ctrl.wait_for(seq).await;
                    ctrl.mark_emitted(seq);
                }
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });

        Ok(AgentHandle::new(rx, 0, None))
    }

    async fn send_input(
        &self,
        _handle: &AgentHandle,
        _input: AgentInput,
    ) -> Result<(), AgentError> {
        Err(AgentError::NotSupported(
            "repro replay runner is read-only".into(),
        ))
    }

    async fn stop(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
        Ok(())
    }

    async fn kill(&self, _handle: &AgentHandle) -> Result<(), AgentError> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }

    fn binary_path(&self) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::{AgentEvent, AssistantMessageEvent};
    use crate::repro::tape::ReproTape;

    #[tokio::test]
    async fn replay_runner_filters_by_session() {
        let session_a = Uuid::new_v4();
        let session_b = Uuid::new_v4();

        let mut tape = ReproTape::new();
        tape.entries.push(ReproTapeEntry::AgentEvent {
            seq: 1,
            ts_ms: 1,
            session_id: session_a.to_string(),
            event: AgentEvent::AssistantMessage(AssistantMessageEvent {
                text: "a".to_string(),
                is_final: true,
            }),
        });
        tape.entries.push(ReproTapeEntry::AgentEvent {
            seq: 2,
            ts_ms: 2,
            session_id: session_b.to_string(),
            event: AgentEvent::AssistantMessage(AssistantMessageEvent {
                text: "b".to_string(),
                is_final: true,
            }),
        });

        let runner = ReplayAgentRunner::new(session_a, AgentType::Claude, Arc::new(tape));
        let handle = runner
            .start(AgentStartConfig::new("x", PathBuf::from("/tmp")))
            .await
            .unwrap();

        let mut events = Vec::new();
        let mut rx = handle.events;
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }

        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::AssistantMessage(msg) => assert_eq!(msg.text, "a"),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
