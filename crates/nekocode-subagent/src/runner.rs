use nekocode_core::agent::Agent;
use nekocode_types::generate::MessageContent;

use crate::registry::{SubagentRunOutcome, SubagentRunResult};

/// Run a child agent's `run_loop` once with the given prompt and return its
/// terminal outcome to the registry-owned task. The `sink` carries the child's own
/// `AgentEvent` stream (relayed to the parent as `MiddlewareEvent`s by the
/// spawn tool). Two cancellation signals race the run:
/// - `cancel`: this child's per-state token (fired by `abort_subagent` on it
///   directly, or by `abort_all_and_clear` from its own registry).
/// - `run_cancel`: the flag shared by the WHOLE spawn tree; the root's
///   `on_turn_end` cancels it once and every descendant `run_subagent`
///   observes it concurrently — this is what makes cross-depth cascade
///   reliable instead of best-effort.
///
/// Both signals are combined into the cancellation token passed through the
/// Agent run loop, so provider tasks and middleware cleanup complete before the
/// runner returns. `old_turns` is always empty (single-turn).
pub async fn run_subagent(
    child: Agent,
    prompt: String,
    sink: nekocode_core::agent::AgentEventSink,
    cancel: tokio_util::sync::CancellationToken,
    run_cancel: tokio_util::sync::CancellationToken,
) -> SubagentRunOutcome {
    let combined = tokio_util::sync::CancellationToken::new();
    let combined_for_relay = combined.clone();
    let cancel_relay = tokio::spawn(async move {
        tokio::select! {
            _ = cancel.cancelled() => {}
            _ = run_cancel.cancelled() => {}
        }
        combined_for_relay.cancel();
    });
    let result = child
        .run_loop_with_cancellation(
            vec![MessageContent::Text { content: prompt }],
            Vec::new(),
            sink,
            combined,
        )
        .await;
    cancel_relay.abort();
    let _ = cancel_relay.await;
    match result {
        Ok(turn) => SubagentRunOutcome::Finished(SubagentRunResult::from_turn(turn)),
        Err(partial) => SubagentRunOutcome::Error {
            error: "subagent run_loop failed".into(),
            partial: SubagentRunResult::from_turn(partial),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use nekocode_core::extensions::Extensions;
    use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
    use nekocode_types::generate::{AssistantContentBlock, AssistantMessage, StopReason, Usage};
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
            Self {
                responses: Mutex::new(r),
            }
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
                    sender
                        .send(ProviderEvent::Content(content.clone()))
                        .unwrap();
                }
            }
            sender
                .send(ProviderEvent::MessageEnd(StopReason::Stop))
                .unwrap();
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
        let db = nekocode_entities::prepare_db(path)
            .await
            .expect("prepare_db");
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(Vec::new()),
            provider,
            extensions: Extensions::new(),
        }
    }

    #[tokio::test]
    async fn run_subagent_returns_finished_result() {
        let child = make_child(Arc::new(MockProvider::new(vec![text_msg("result")]))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let outcome = run_subagent(
            child,
            "do thing".into(),
            nekocode_core::agent::AgentEventSink::new(tx),
            tokio_util::sync::CancellationToken::new(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let SubagentRunOutcome::Finished(result) = outcome else {
            panic!("expected finished outcome");
        };
        assert!(result.finished);
        // The captured turn has user + assistant messages.
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn run_subagent_error_returns_partial_result() {
        // Empty responses → first stream_generate errors ("mock exhausted").
        let child = make_child(Arc::new(MockProvider::new(Vec::new()))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let outcome = run_subagent(
            child,
            "do thing".into(),
            nekocode_core::agent::AgentEventSink::new(tx),
            tokio_util::sync::CancellationToken::new(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let SubagentRunOutcome::Error { partial, .. } = outcome else {
            panic!("expected error outcome");
        };
        assert!(!partial.finished);
        assert_eq!(partial.messages.len(), 1, "partial keeps the user message");
    }

    /// A provider that never resolves — lets us observe the `run_cancel`
    /// branch of run_subagent's select! in isolation.
    struct PendingProvider;
    #[async_trait::async_trait]
    impl Provider for PendingProvider {
        async fn stream_generate(
            &self,
            _: nekocode_core::types::GenerateRequest,
            _: mpsc::UnboundedSender<ProviderEvent>,
        ) -> Result<ProviderResponse, ProviderError> {
            std::future::pending().await
        }
    }

    /// Two run_subagent runs share one `run_cancel` (the tree-wide flag).
    /// Cancelling it once must bail BOTH runs concurrently — this is the
    /// cross-depth cascade guarantee: a parent and its grandchild subscribe
    /// to the same flag and both end on a single cancel, no reliance on the
    /// runtime re-poling one layer before the next.
    #[tokio::test]
    async fn shared_run_cancel_aborts_multiple_concurrent_runs() {
        let run_cancel = tokio_util::sync::CancellationToken::new();

        let (ta, _ra) = mpsc::unbounded_channel();
        let run_a = run_subagent(
            make_child(Arc::new(PendingProvider)).await,
            "a".into(),
            nekocode_core::agent::AgentEventSink::new(ta),
            tokio_util::sync::CancellationToken::new(),
            run_cancel.clone(),
        );
        let (tb, _rb) = mpsc::unbounded_channel();
        let run_b = run_subagent(
            make_child(Arc::new(PendingProvider)).await,
            "b".into(),
            nekocode_core::agent::AgentEventSink::new(tb),
            tokio_util::sync::CancellationToken::new(),
            run_cancel.clone(),
        );

        // Both runs are pending (PendingProvider never resolves).
        let mut a = tokio::spawn(run_a);
        let mut b = tokio::spawn(run_b);
        // Yield so both runs actually start and park on the select!.
        tokio::task::yield_now().await;

        // One cancel fires the whole tree.
        run_cancel.cancel();

        // Both must complete (the run_cancel branch returned) within a beat.
        let (outcome_a, outcome_b) =
            tokio::time::timeout(std::time::Duration::from_millis(200), async {
                ((&mut a).await.unwrap(), (&mut b).await.unwrap())
            })
            .await
            .expect("both runs ended promptly after the shared run_cancel fired");

        assert!(matches!(outcome_a, SubagentRunOutcome::Error { .. }));
        assert!(matches!(outcome_b, SubagentRunOutcome::Error { .. }));
    }
}
