use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::error::AgentError;
use crate::agent::events::AgentEvent;
use crate::agent::runner::{AgentHandle, AgentInput, AgentRunner, AgentStartConfig, AgentType};
use crate::repro::tape::{RecordedInput, ReproTapeEntry, ReproTapeWriter};

#[derive(Clone)]
pub struct RecordingAgentRunner {
    session_id: Uuid,
    inner: Arc<dyn AgentRunner>,
    writer: Arc<ReproTapeWriter>,
}

impl RecordingAgentRunner {
    pub fn new(
        session_id: Uuid,
        inner: Arc<dyn AgentRunner>,
        writer: Arc<ReproTapeWriter>,
    ) -> Self {
        Self {
            session_id,
            inner,
            writer,
        }
    }

    fn record_event(&self, event: &AgentEvent) {
        let entry = ReproTapeEntry::AgentEvent {
            seq: self.writer.next_seq(),
            ts_ms: now_ms(),
            session_id: self.session_id.to_string(),
            event: event.clone(),
        };
        if let Err(err) = self.writer.append(entry) {
            tracing::debug!(error = %err, "failed to append agent event to repro tape");
        }
    }

    fn record_input(&self, input: &AgentInput) {
        let entry = ReproTapeEntry::AgentInput {
            seq: self.writer.next_seq(),
            ts_ms: now_ms(),
            session_id: self.session_id.to_string(),
            input: to_recorded_input(input),
        };
        if let Err(err) = self.writer.append(entry) {
            tracing::debug!(error = %err, "failed to append agent input to repro tape");
        }
    }
}

#[async_trait]
impl AgentRunner for RecordingAgentRunner {
    fn agent_type(&self) -> AgentType {
        self.inner.agent_type()
    }

    async fn start(&self, config: AgentStartConfig) -> Result<AgentHandle, AgentError> {
        let inner_handle = self.inner.start(config).await?;

        let AgentHandle {
            mut events,
            session_id,
            pid,
            mut input_tx,
        } = inner_handle;

        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
        let recorder = self.clone();

        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                recorder.record_event(&event);
                if event_tx.send(event).await.is_err() {
                    break;
                }
            }
        });

        let wrapped_input_tx = input_tx.take().map(|inner_tx| {
            let (tx, mut rx) = mpsc::channel::<AgentInput>(32);
            let recorder = self.clone();
            tokio::spawn(async move {
                while let Some(input) = rx.recv().await {
                    recorder.record_input(&input);
                    if inner_tx.send(input).await.is_err() {
                        break;
                    }
                }
            });
            tx
        });

        let mut handle = AgentHandle::new(event_rx, pid, wrapped_input_tx);
        handle.session_id = session_id;
        Ok(handle)
    }

    async fn send_input(&self, handle: &AgentHandle, input: AgentInput) -> Result<(), AgentError> {
        // If we already wrapped an input channel, it will record + forward.
        if handle.input_tx.is_some() {
            return self.inner.send_input(handle, input).await;
        }

        // Otherwise, record here and delegate.
        self.record_input(&input);
        self.inner.send_input(handle, input).await
    }

    async fn stop(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        self.inner.stop(handle).await
    }

    async fn kill(&self, handle: &AgentHandle) -> Result<(), AgentError> {
        self.inner.kill(handle).await
    }

    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn binary_path(&self) -> Option<PathBuf> {
        self.inner.binary_path()
    }
}

fn to_recorded_input(input: &AgentInput) -> RecordedInput {
    match input {
        AgentInput::ClaudeJsonl(jsonl) => RecordedInput::ClaudeJsonl {
            jsonl: jsonl.clone(),
        },
        AgentInput::CodexPrompt { text, images, .. } => RecordedInput::CodexPrompt {
            text: text.clone(),
            images: images.iter().map(|p| p.display().to_string()).collect(),
        },
        AgentInput::OpencodeQuestion {
            request_id,
            answers,
        } => RecordedInput::OpencodeQuestion {
            request_id: request_id.clone(),
            answers: answers.clone(),
        },
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::events::{AgentEvent, AssistantMessageEvent, SessionInitEvent};
    use crate::agent::session::SessionId;
    use crate::repro::tape::ReproTape;
    use tempfile::tempdir;

    struct TestRunner {
        agent_type: AgentType,
        event_sequence: Vec<AgentEvent>,
        captured_inputs: Arc<parking_lot::Mutex<Vec<AgentInput>>>,
    }

    impl TestRunner {
        fn new(agent_type: AgentType, event_sequence: Vec<AgentEvent>) -> Self {
            Self {
                agent_type,
                event_sequence,
                captured_inputs: Arc::new(parking_lot::Mutex::new(Vec::new())),
            }
        }

        fn captured_inputs(&self) -> Vec<AgentInput> {
            self.captured_inputs.lock().clone()
        }
    }

    #[async_trait]
    impl AgentRunner for TestRunner {
        fn agent_type(&self) -> AgentType {
            self.agent_type
        }

        async fn start(&self, _config: AgentStartConfig) -> Result<AgentHandle, AgentError> {
            let (tx, rx) = mpsc::channel(32);
            let events = self.event_sequence.clone();
            tokio::spawn(async move {
                for ev in events {
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            });

            let (input_tx, mut input_rx) = mpsc::channel::<AgentInput>(32);
            let captured = self.captured_inputs.clone();
            tokio::spawn(async move {
                while let Some(input) = input_rx.recv().await {
                    captured.lock().push(input);
                }
            });

            Ok(AgentHandle::new(rx, 4242, Some(input_tx)))
        }

        async fn send_input(
            &self,
            handle: &AgentHandle,
            input: AgentInput,
        ) -> Result<(), AgentError> {
            let Some(ref tx) = handle.input_tx else {
                return Err(AgentError::ChannelClosed);
            };
            tx.send(input).await.map_err(|_| AgentError::ChannelClosed)
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

    #[tokio::test]
    async fn recording_runner_writes_events_and_inputs() {
        let dir = tempdir().unwrap();
        let tape_path = dir.path().join("tape.jsonl");
        let writer = Arc::new(ReproTapeWriter::create(&tape_path).unwrap());

        let session_uuid = Uuid::new_v4();
        let inner = Arc::new(TestRunner::new(
            AgentType::Codex,
            vec![
                AgentEvent::SessionInit(SessionInitEvent {
                    session_id: SessionId::from_string("sess-1"),
                    model: None,
                }),
                AgentEvent::AssistantMessage(AssistantMessageEvent {
                    text: "hi".to_string(),
                    is_final: true,
                }),
            ],
        ));

        let wrapped: Arc<dyn AgentRunner> = Arc::new(RecordingAgentRunner::new(
            session_uuid,
            inner.clone(),
            writer,
        ));

        let config = AgentStartConfig::new("prompt", PathBuf::from("/tmp"));
        let mut handle = wrapped.start(config).await.unwrap();

        // Drain events.
        while handle.events.recv().await.is_some() {}

        // Send an input via the wrapped channel.
        let input_tx = handle.input_tx.take().unwrap();
        input_tx
            .send(AgentInput::CodexPrompt {
                text: "hello".to_string(),
                images: Vec::new(),
                model: None,
            })
            .await
            .unwrap();

        // Give the forwarding task a moment to write.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(inner.captured_inputs().len(), 1);

        let tape = ReproTape::read_jsonl_from_path(&tape_path).unwrap();
        assert!(
            tape.entries
                .iter()
                .any(|e| matches!(e, ReproTapeEntry::AgentEvent { .. })),
            "expected at least one AgentEvent entry"
        );
        assert!(
            tape.entries
                .iter()
                .any(|e| matches!(e, ReproTapeEntry::AgentInput { .. })),
            "expected at least one AgentInput entry"
        );
    }
}
