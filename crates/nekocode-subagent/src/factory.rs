use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};

/// The seam between this crate and the API crate: the only way this crate can
/// build an isolated child middleware instance from a `MiddlewareSpec`. It is
/// implemented in the API crate (the sole layer with access to the
/// shell/file/mcp/skills constructors) and called by `spawn_subagent`, which
/// passes the AtomicU64-allocated `subagent_id` and the child's fresh
/// `Extensions` so that session-scoped middleware (e.g. shell) get their own
/// per-child state map.
#[async_trait::async_trait]
pub trait SubagentMiddlewareFactory: Send + Sync {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Extensions,
    ) -> Box<dyn Middleware>;
}
