# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

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
| `nekocode-shell` | Shell middleware + tool definitions (`shell`, `spawn_shell`, `cancel_shell`, `send_shell_input`, `fetch_shell_output`, `wait_shell_done`). |
| `nekocode-file` | File tools middleware: `read_file`, `write_file`, `edit_file`, `set_title`. Includes path sandbox enforcement when `working_directory` is configured. |
| `nekocode-mcp` | MCP middleware: connects to MCP servers (stdio and Streamable HTTP transports), discovers tools, and registers them into the agent's `ToolRegistry`. |
| `nekocode-skills` | Skills middleware implementing the [agentskills.io](https://agentskills.io/specification) spec with progressive disclosure (catalog → instructions → resources). |
| `nekocode-subthread` | Subthread spawning, control, and synchronization middleware. Models long-lived, DB-persisted background threads with `SubthreadRegistry` for in-memory run state. |

**Frontend** (`webui/`): Vue 3 + Vite + TypeScript with PrimeVue (component library), Pinia (state), Vue Router, UnoCSS. Single route at `/` loading `modules/main/View.vue`.

## Key patterns

- **Agent run loop** (`crates/nekocode-core/src/agent/mod.rs`): Takes user input, fetches thread messages from DB, then loops: runs `before_generate` middleware → calls provider `stream_generate` → converts events → runs `after_generate` middleware. Middleware can set `AgentControlFlow::GenerateWith(MessageContent)` to re-invoke the provider with an injected middleware message. DB → provider-message conversion is done inline in `run_loop` (there is no `collect_db_messages` function).

- **`Provider` trait** (`crates/nekocode-core/src/provider.rs`): `stream_generate` takes a `GenerateRequest` + unbounded sender, streams `ProviderEvent` variants (content deltas, reasoning, tool calls).

- **Generate API** (`crates/nekocode/src/api/generate/`): WebSocket-based. Client sends `{threadId, userInput}`, server spawns the agent run loop and streams `WebSocketEvent::Delta(AgentEvent)` then `WebSocketEvent::Stop(StopReason)`. Uses `dashmap` for concurrent generate state tracking and `CancellationToken` for interruption.

- **Auth** (`crates/nekocode/src/api/auth/`): `auth_middleware` is mounted on the router and checks `Token` header against the `tokens` table (or skips if config is `AuthenticationConfig::None`). The `/api/auth` route is public (not behind the middleware); all other `/api` routes are protected. Password auth creates a UUID token with 30-day expiry. Password comparison uses constant-time equality.

- **File tools sandbox** (`crates/nekocode-file/src/config.rs`): When `working_directory` is configured, `resolve_and_check` canonicalizes paths and verifies they stay within the sandbox. Paths outside the working directory are rejected. Without a configured `working_directory`, no sandbox is enforced.

- **DB queries use Toasty macros** (`toasty::query!`, `toasty::create!`) with `Model` derive — see entities for the schema. Use `toasty::query!(Thread FILTER .id == #(id))` syntax, not raw SQL.
