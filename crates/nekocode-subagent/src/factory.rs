use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};

/// Builds an isolated child middleware instance from a spec. Implemented by
/// the API crate (the only layer that can see the shell/file/mcp/skills
/// constructors). `subagent_id` is the AtomicU64-allocated ID (not a DB id);
/// `extensions` is the child's fresh DashMap so middleware like shell gets
/// its own session map.
#[async_trait::async_trait]
pub trait SubagentMiddlewareFactory: Send + Sync {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware>;
}
