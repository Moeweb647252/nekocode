# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build / Test / Run

```bash
# Build the whole workspace
cargo build

# Run the server (requires ~/.config/nekocode/config.toml)
cargo run

# Run all Rust tests
cargo test

# Run tests for a specific crate
cargo test -p nekocode-core

# Frontend (cd webui first)
cd webui && pnpm install
cd webui && pnpm dev        # dev server
cd webui && pnpm build      # production build
cd webui && pnpm lint       # lint (oxlint + eslint)
cd webui && pnpm format     # format (oxfmt)
cd webui && pnpm type-check # vue-tsc type checking
```

## Architecture

**Rust workspace** (root `Cargo.toml`): A chat AI server using Axum + Toasty ORM (SQLite).

| Crate | Purpose |
|-------|---------|
| `nekocode` (bin) | Axum HTTP server: REST API + WebSocket for streaming. Reads config from `~/.config/nekocode/config.toml`. |
| `nekocode-core` | Core orchestration: `Agent::run_loop` (middleware pipeline → provider calls), `Provider` trait (LLM abstraction), `Middleware` trait, `ToolRegistry`. |
| `nekocode-entities` | Database models via Toasty ORM: `Thread`, `Message`, `Turn`, `Token`, `Middleware`. `prepare_db()` opens SQLite. |
| `nekocode-types` | Shared types: `Config` (TOML deser), `StreamEvent`/`StreamEventData` (SSE/WS events), `ToolCall`/`ToolCallResult`/`ToolSpec`, `Tool`/`ToolRegistry`. |
| `nekocode-provider` | LLM backend implementations (DeepSeek, Anthropic, OpenAI-compatible) + SSE client (`EventSource` trait on `reqwest::Response`). |
| `nekocode-shell` | Shell middleware + tool definitions (`shell`, `spawn_shell`, `cancel_shell`, `send_shell_input`, `fetch_shell_output`). This is where tools actually live. |
| `nekocode-tool` | Tool definitions (stub — boilerplate only, no real tools). |
| `nekocode-mcp` | MCP support (stub — boilerplate only). |
| `nekocode-skills` | Skills support (stub — boilerplate only). |
| `nekocode-file` | File support (stub — boilerplate only). |

**Frontend** (`webui/`): Vue 3 + Vite + TypeScript with PrimeVue (component library), Pinia (state), Vue Router, UnoCSS. Single route at `/` loading `modules/main/View.vue`.

## Key patterns

- **Agent run loop** (`crates/nekocode-core/src/agent/mod.rs`): Takes user input, fetches thread messages from DB, then loops: runs `before_generate` middleware → calls provider `stream_generate` → converts events → runs `after_generate` middleware. Middleware can set `AgentControlFlow::GenerateWith(MessageContent)` to re-invoke the provider with an injected middleware message. DB → provider-message conversion is done inline in `run_loop` (there is no `collect_db_messages` function).

- **`Provider` trait** (`crates/nekocode-core/src/provider.rs`): `stream_generate` takes a `GenerateRequest` + unbounded sender, streams `ProviderEvent` variants (content deltas, reasoning, tool calls). Note: `generate()` is a non-streaming convenience method but is currently unused.

- **Generate API** (`crates/nekocode/src/api/generate/`): WebSocket-based. Client sends `{threadId, userInput}`, server spawns the agent run loop and streams `WebSocketEvent::Delta(AgentEvent)` then `WebSocketEvent::Stop(StopReason)`. Uses `dashmap` for concurrent generate state tracking and `CancellationToken` for interruption.

- **Auth** (`crates/nekocode/src/api/mod.rs`): `auth_middleware` checks `Token` header against the `tokens` table (or skips if config is `AuthenticationConfig::None`). Password auth creates a UUID token with 30-day expiry. Note: the middleware is implemented but not yet mounted on the router.

- **DB queries use Toasty macros** (`toasty::query!`, `toasty::create!`) with `Model` derive — see entities for the schema. Use `toasty::query!(Thread FILTER .id == #(id))` syntax, not raw SQL.
