use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_types::generate::MessageContent;

use crate::registry::{SubagentRegistry, SubagentRunResult};

/// Run a child agent's `run_loop` once with the given prompt and capture the
/// resulting `Turn` into the registry. The `sink` carries the child's own
/// `AgentEvent` stream (relayed to the parent as `MiddlewareEvent`s by the
/// spawn tool). `cancel` is the child's `CancellationToken`; if the parent
/// turn ends, `on_turn_end` → `abort_all_and_clear` cancels it so this run
/// bails promptly and records an error for waiters. `old_turns` is always
/// empty (single-turn).
pub async fn run_subagent(
    agent_id: u64,
    child: Agent,
    prompt: String,
    registry: Arc<SubagentRegistry>,
    sink: nekocode_core::agent::AgentEventSink,
    cancel: tokio_util::sync::CancellationToken,
) {
    let run = child.run_loop(
        vec![MessageContent::Text { content: prompt }],
        Vec::new(),
        sink,
    );
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            // Parent aborted: the child run_loop future is dropped here, so
            // its OWN end-of-turn on_turn_end dispatch (which would cascade-
            // abort the child's spawned grandchildren) never runs. Drive that
            // cascade explicitly so descendants don't leak as detached tasks.
            for mw in child.middlewares.iter() {
                let _ = mw.on_turn_end().await;
            }
            registry.set_error(agent_id, "subagent cancelled by parent turn end".into());
            return;
        }
        r = run => r,
    };
    match result {
        Ok(turn) => registry.set_finished(agent_id, SubagentRunResult {
            usage: turn.usage, messages: turn.messages, finished: turn.finished,
        }),
        Err(_partial) => registry.set_error(agent_id, "subagent run_loop failed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
    use nekocode_types::generate::{
        AssistantContentBlock, AssistantMessage, StopReason, Usage,
    };
    use tokio::sync::mpsc;

    /// A local mock provider returning a scripted sequence of assistant
    /// messages (FIFO). Exhausting the list yields an error — mirrors
    /// nekocode-core's MockProvider shape without crossing crate visibility.
    struct MockProvider {
        responses: Mutex<Vec<AssistantMessage>>,
    }

    impl MockProvider {
        fn new(responses: Vec<AssistantMessage>) -> Self {
            let mut r = responses;
            r.reverse(); // pop() is LIFO; reverse once for FIFO
            Self { responses: Mutex::new(r) }
        }
    }

    fn text_msg(s: &str) -> AssistantMessage {
        AssistantMessage {
            blocks: vec![AssistantContentBlock::Text {
                content: s.to_string(),
                reasoning_content: None,
            }],
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn stream_generate(
            &self,
            _request: nekocode_core::types::GenerateRequest,
            sender: mpsc::UnboundedSender<ProviderEvent>,
        ) -> Result<ProviderResponse, ProviderError> {
            let msg = self
                .responses
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| ProviderError::Other(anyhow::anyhow!("mock exhausted")))?;
            for block in &msg.blocks {
                if let AssistantContentBlock::Text { content, .. } = block {
                    sender.send(ProviderEvent::Content(content.clone())).unwrap();
                }
            }
            sender.send(ProviderEvent::MessageEnd(StopReason::Stop)).unwrap();
            Ok(ProviderResponse {
                message: msg,
                usage: Usage {
                    total_input: 10,
                    total_output: 5,
                    cache_hit: false,
                    cache_miss: 10,
                },
            })
        }
    }

    async fn make_child(provider: Arc<dyn Provider>) -> Agent {
        // Process-wide monotonic counter so parallel tests get distinct db
        // file paths (a fresh `AtomicU64::new(0)` per call would always emit 0
        // and collide on `..._{pid}_0.db`, racing to "database is locked").
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nekocode_subagent_runner_{}_{}.db",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let db = nekocode_entities::prepare_db(path).await.expect("prepare_db");
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(Vec::new()),
            provider,
            extensions: Arc::new(dashmap::DashMap::new()),
        }
    }

    #[tokio::test]
    async fn run_subagent_success_stores_result() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry.allocate_running();
        let child = make_child(Arc::new(MockProvider::new(vec![text_msg("result")]))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        run_subagent(
            id,
            child,
            "do thing".into(),
            registry.clone(),
            nekocode_core::agent::AgentEventSink::new(tx),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        assert!(matches!(registry.run_state(id), crate::registry::SubagentRunState::Finished));
        let result = registry.result(id).expect("result stored");
        assert!(result.finished);
        // The captured turn has user + assistant messages.
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn run_subagent_error_marks_error_state() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry.allocate_running();
        // Empty responses → first stream_generate errors ("mock exhausted").
        let child = make_child(Arc::new(MockProvider::new(Vec::new()))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        run_subagent(
            id,
            child,
            "do thing".into(),
            registry.clone(),
            nekocode_core::agent::AgentEventSink::new(tx),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        assert!(matches!(
            registry.run_state(id),
            crate::registry::SubagentRunState::Error(_)
        ));
        assert!(registry.run_state(id).is_ready());
    }
}
