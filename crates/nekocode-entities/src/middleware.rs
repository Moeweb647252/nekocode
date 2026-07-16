use serde::Serialize;
use toasty::Json;

/// A persisted middleware configuration row, child of a
/// [`Thread`](crate::thread::Thread).
///
/// `name` identifies the middleware kind (shell, file, mcp, …) and `config`
/// holds its serialized config as JSON. `enabled` toggles whether the
/// middleware is built for its thread; for the singleton-per-thread kinds
/// (Shell/Tool) it toggles the whole middleware, for Mcp it toggles that
/// specific server row.
#[derive(toasty::Model, Clone, Debug, Serialize)]
pub struct Middleware {
    #[key]
    #[auto]
    pub id: u64,
    #[index]
    pub thread_id: u64,
    /// Explicit composition order. Ties are resolved by id for rows created
    /// before this field existed, so activation is deterministic after schema
    /// migration as well.
    #[default(0)]
    pub order_index: u64,
    pub name: String,
    pub config: Json<serde_json::Value>,
    /// Whether this middleware is active for its thread. Shell and Tool are
    /// singleton per thread and can be toggled; Mcp is zero-or-many and each
    /// row's enabled flag controls whether that specific MCP server is active.
    #[default(true)]
    pub enabled: bool,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=thread_id, references=id)]
    pub thread: toasty::Deferred<crate::thread::Thread>,
}
