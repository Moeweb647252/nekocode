use nekocode_types::generate::Usage;

use super::error::AgentError;

/// Abstraction over message persistence for the agent run loop.
///
/// The run loop has two implementations: one backed by a database
/// (`DbMessageStore`, used by `Agent`) and one backed by an in-memory
/// `Vec` (`InMemoryMessageStore`, used by `SubAgent`). The trait
/// isolates the persistence concern so the loop logic can be shared.
#[async_trait::async_trait]
pub trait MessageStore: Send + Sync {
    /// Append a user message to the conversation and return the new
    /// message count.
    async fn push_user_message(
        &self,
        content: nekocode_types::generate::MessageContent,
    ) -> Result<usize, AgentError>;

    /// Append an assistant message (with usage) and return the new
    /// message count.
    async fn push_assistant_message(
        &self,
        message: nekocode_types::generate::AssistantMessage,
        usage: Usage,
    ) -> Result<usize, AgentError>;

    /// Append a tool-call result and return the new message count.
    async fn push_tool_result(
        &self,
        result: nekocode_types::tool::ToolCallResult,
    ) -> Result<usize, AgentError>;

    /// Append a middleware-injected message and return the new count.
    async fn push_middleware_message(
        &self,
        content: nekocode_types::generate::MessageContent,
    ) -> Result<usize, AgentError>;

    /// Return the current full message history as provider-compatible
    /// content. This is used to build the `GenerateRequest::messages`
    /// field before each provider call.
    async fn current_messages(
        &self,
    ) -> Result<Vec<nekocode_types::generate::Message>, AgentError>;

    /// Finalize the run — e.g. mark the turn as finished and persist
    /// accumulated usage.
    async fn finalize(&self, usage: &Usage) -> Result<(), AgentError>;
}
