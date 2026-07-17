//! Lightweight, in-memory, single-turn subagent machinery.
//!
//! A subagent is spawned with a prompt, runs `Agent::run_loop` once, stores
//! the captured `Turn` in memory, and is done. It is purely in-memory (no DB
//! rows, no `SubthreadController`), lighter than `nekocode-subthread` (which is
//! DB-persisted and multi-turn). Per-parent state lives in
//! `Agent.extensions` as an `Arc<SubagentRegistry>` (stored under
//! `TypeId::of::<Arc<SubagentRegistry>>()`).
//!
//! Middleware instances for a child are built in isolation at spawn time via
//! the `SubagentMiddlewareFactory` trait (implemented by the API crate, the
//! only layer that can see the shell/file/mcp/skills constructors).

pub mod config;
pub mod factory;
pub mod middleware;
pub mod profile;
pub mod registry;
pub mod runner;
pub mod tool;

pub use config::SubagentConfig;
pub use factory::SubagentMiddlewareFactory;
pub use middleware::{SubagentContext, SubagentMiddleware};
pub use profile::{ProfileCatalog, SubagentProfile};
pub use registry::{
    SubagentRegistry, SubagentRunOutcome, SubagentRunResult, SubagentSnapshot, SubagentTask,
    WaitAllOutcome, WaitAnyOutcome,
};
