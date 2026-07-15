use serde::{Deserialize, Serialize};

/// A model-authored request to invoke a tool, exactly as parsed from the
/// provider's stream. `args` is the raw JSON the model produced for the call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// The outcome of executing a [`ToolCall`], keyed back to the originating
/// call by `id` so the conversation can pair request and result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    pub id: String,
    pub result: ToolCallResultInner,
}

/// The success or error payload of a tool execution. Uses struct variants
/// (not newtypes) so the `#[serde(tag = "type")]` tag always has a JSON
/// object to attach to — see the inline comment below for the failure mode
/// the newtype form used to cause.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum ToolCallResultInner {
    #[serde(rename = "success")]
    Success {
        // serde's internally-tagged representation (`tag = "type"`) requires
        // each variant to serialize as a JSON object so the tag field has
        // somewhere to live. A newtype variant wrapping a bare `Value`/`String`
        // fails that — e.g. `Error(String)` serializes to a plain JSON string
        // with no map to attach the tag to, panicking at serialize time. Use
        // struct variants with named fields instead.
        value: serde_json::Value,
    },
    #[serde(rename = "error")]
    Error { error: String },
}

impl From<Result<serde_json::Value, ToolError>> for ToolCallResultInner {
    fn from(value: Result<serde_json::Value, ToolError>) -> Self {
        match value {
            Ok(result) => ToolCallResultInner::Success { value: result },
            Err(err) => ToolCallResultInner::Error { error: err.to_string() },
        }
    }
}

/// A tool's declaration as advertised to the model: a name, a natural-language
/// description, and a JSON Schema describing the accepted parameters. Sent to
/// the provider as part of a generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameter_schema: serde_json::Value,
}

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

/// Errors a tool can produce, either before running (`InvalidParameters`) or
/// during execution (`ExecutionError`). Stringy by design — tool results are
/// surfaced to the model and the user, not matched programmatically.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),
    #[error("Error during tool execution: {0}")]
    ExecutionError(String),
}

/// The interface every concrete tool implements: advertise its [`ToolSpec`]
/// and execute a call. Implementations must be `Send + Sync` so they can live
/// behind an `Arc<dyn Tool>` in the [`ToolRegistry`].
#[async_trait]
pub trait Tool {
    fn spec(&self) -> ToolSpec;

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}

/// The agent's name-keyed registry of available tools, built per thread by
/// middleware during `before_generate` and consumed when the agent executes
/// the tool calls a generation produced.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool + Send + Sync>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool + Send + Sync>) {
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool + Send + Sync>> {
        self.tools.get(name).cloned()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::ToolCallResultInner;

    /// The historical representation that crashed serialization: an internally
    /// tagged enum (`#[serde(tag = "type")]`) using *newtype* variants that wrap
    /// bare scalars (`String`) / `Value`. Reconstructed here so we can prove the
    /// failure mode and guard against regressing back to it.
    #[derive(Serialize)]
    #[serde(tag = "type")]
    enum OldToolCallResultInner {
        #[serde(rename = "success")]
        Success(serde_json::Value),
        #[serde(rename = "error")]
        Error(String),
    }

    /// Serialize `value` under the OLD newtype form, catching any panic so the
    /// test reports the failure mode (panic vs. returned Err) instead of
    /// aborting the whole test binary.
    fn serialize_old(
        value: OldToolCallResultInner,
    ) -> Result<Result<String, serde_json::Error>, String> {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            serde_json::to_string(&value)
        }));
        match result {
            Ok(inner) => Ok(inner),
            Err(_payload) => Err("serializer panicked".to_string()),
        }
    }

    /// Returns true when the old form FAILED to serialize (panic or Err).
    fn old_failed(value: OldToolCallResultInner) -> bool {
        match serialize_old(value) {
            Ok(Ok(_)) => false,
            Ok(Err(_)) => true,
            Err(_) => true,
        }
    }

    /// ROOT CAUSE PROOF: the old newtype form fails for every result an actual
    /// tool call can produce — non-object success payloads and ALL errors. An
    /// internally-tagged enum has nowhere to attach the `type` tag when the
    /// variant content is a scalar/non-map, so serde aborts the call. This is
    /// exactly what took the backend down when the shell tool returned a result.
    #[test]
    fn old_newtype_form_fails_for_realistic_results() {
        // Success with a string scalar (e.g. a tool that returned a bare string).
        assert!(
            old_failed(OldToolCallResultInner::Success(json!("hello"))),
            "string-valued success should fail under the old form"
        );
        // Success with a number scalar.
        assert!(
            old_failed(OldToolCallResultInner::Success(json!(42))),
            "number-valued success should fail under the old form"
        );
        // Error: always a String scalar — this is what `shell` emits on a failed
        // spawn / timeout / missing param, and what the agent emits for a tool
        // it couldn't find.
        assert!(
            old_failed(OldToolCallResultInner::Error("boom".into())),
            "error result should fail under the old form"
        );
    }

    /// Pins down the *mode* of the old-form failure. On the current
    /// serde/serde_json it is a panic (caught here via catch_unwind), not a
    /// returned Err — which is why an unwinding panic propagated through the
    /// tokio task that serializes the streamed tool result.
    /// The exact failure mode: the old newtype form makes serde *return an
    /// error* (it does not panic on its own). That matters because Toasty
    /// persists every message with `serde_json::to_string(..).expect("failed
    /// to serialize")` (toasty/src/stmt/json.rs), so a returned serialization
    /// error becomes a **panic** inside the agent's `run_loop` when it persists
    /// the tool result — which is what took the backend down whenever a shell
    /// tool call produced an error result (timeout, failed spawn, "Tool not
    /// found", invalid shell_id, …) or a non-object success payload.
    #[test]
    fn old_newtype_error_form_returns_serde_error() {
        // Error result: serde refuses to serialize the tagged newtype.
        let err_mode = serialize_old(OldToolCallResultInner::Error("boom".into()));
        match err_mode {
            Ok(Err(e)) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("cannot serialize tagged newtype variant"),
                    "unexpected error message: {msg}"
                );
            }
            other => panic!("expected a returned serde error, got {other:?}"),
        }

        // Scalar success is rejected the same way.
        let ok_mode = serialize_old(OldToolCallResultInner::Success(json!("x")));
        assert!(matches!(ok_mode, Ok(Err(_))), "scalar success should be rejected, got {ok_mode:?}");

        // Object success, by contrast, merges its fields with the tag and
        // serializes fine — which is why a *successful* `shell` call (object
        // payload) did not trigger this, only error/scalar results did.
        let obj_mode = serialize_old(OldToolCallResultInner::Success(json!({"a": 1})));
        assert!(matches!(obj_mode, Ok(Ok(_))), "object success should serialize, got {obj_mode:?}");
    }

    /// The fix: struct variants give the tag a map to live in, so every result
    /// serializes cleanly.
    #[test]
    fn current_struct_form_serializes_every_result() {
        let ok = ToolCallResultInner::Success {
            value: json!({"stdout": "hi", "stderr": "", "exit_code": 0}),
        };
        let s = serde_json::to_string(&ok).unwrap();
        assert!(s.contains(r#""type":"success""#), "missing success tag in {s}");
        assert!(s.contains(r#""value""#), "missing value field in {s}");

        // Scalars must also work now (the old form died here).
        let s_scalar =
            serde_json::to_string(&ToolCallResultInner::Success { value: json!("x") }).unwrap();
        assert!(s_scalar.contains(r#""type":"success""#));

        let err = ToolCallResultInner::Error {
            error: "boom".into(),
        };
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains(r#""type":"error""#), "missing error tag in {s}");
        assert!(s.contains(r#""boom""#), "missing error payload in {s}");
    }

    /// Regression guard for the exact path that crashed the backend: the agent
    /// persists a `Message::ToolCallResult` carrying an *error* result (what a
    /// shell tool emits on timeout / failed spawn / "Tool not found"). Toasty
    /// serializes this with `serde_json::to_string(..).expect(..)`, so this
    /// must never return an error again.
    #[test]
    fn message_with_error_tool_result_serializes() {
        use crate::generate::MessageType;

        let msg = MessageType::ToolCallResult(super::ToolCallResult {
            id: "call_1".into(),
            result: ToolCallResultInner::Error {
                error: "command timed out after 5s".into(),
            },
        });
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains(r#""type":"toolCallResult""#), "outer tag missing: {s}");
        assert!(s.contains(r#""type":"error""#), "inner error tag missing: {s}");
        assert!(s.contains("command timed out"), "error payload missing: {s}");

        // Round-trips too (the DB reads it back this way).
        let back: MessageType = serde_json::from_str(&s).unwrap();
        match back {
            MessageType::ToolCallResult(r) => match r.result {
                ToolCallResultInner::Error { error } => {
                    assert_eq!(error, "command timed out after 5s");
                }
                other => panic!("expected error variant, got {other:?}"),
            },
            other => panic!("expected ToolCallResult, got {other:?}"),
        }
    }
}
