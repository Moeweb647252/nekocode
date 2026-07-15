//! nekocode-core — the agent orchestration layer.
//!
//! Defines the three abstractions the rest of the workspace builds on:
//! - the [`provider::Provider`] trait ([`provider`]) abstracting an LLM backend
//!   and its streamed events,
//! - the [`middleware::Middleware`] trait ([`middleware`]) hooking
//!   `before_generate` / `after_generate` / `on_turn_end` into the run loop,
//! - the [`agent::Agent`] type and its run loop ([`agent`]) that wires
//!   providers, middleware, and tools together.
//!
//! Shared request/response types live in [`types`], and a type-keyed
//! [`extensions::Extensions`] map carries per-thread state (registries,
//! controllers) for middleware to find.

pub mod agent;
pub mod extensions;
pub mod middleware;
pub mod provider;
pub mod types;
