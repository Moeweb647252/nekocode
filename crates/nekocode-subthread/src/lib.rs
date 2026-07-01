//! nekocode-subthread — subthread spawning, control, and synchronization
//! middleware.
//!
//! A parent thread with the `subthread` middleware enabled can spawn child
//! threads (`spawn_subthread`), run them in the background (`start_subthread`),
//! inspect/read their state, and synchronize on completion
//! (`wait_any_subthread` / `wait_all_subthreads`). The parent-child
//! relationship is persisted via `Thread.own_by_id`; in-memory run state lives
//! in a shared `SubthreadRegistry`.

pub mod controller;
pub mod config;
pub mod middleware;
pub mod path;
pub mod registry;
pub mod tool;

pub use config::SubthreadConfig;
pub use middleware::SubthreadMiddleware;
pub use registry::{SubthreadRegistry, SubthreadRunState, SubthreadState};
