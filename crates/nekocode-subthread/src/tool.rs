use std::sync::Arc;

use nekocode_entities::middleware::Middleware;
use nekocode_entities::thread::Thread;
use nekocode_types::tool::{Tool, ToolError};
use toasty::Json;

use crate::{
    config::SubthreadConfig,
    path::ensure_within,
    registry::SubthreadRegistry,
};

/// Shared context carried by every subthread tool. One instance lives behind
/// an `Arc` inside `SubthreadMiddleware` and is cloned (cheaply — all fields
/// are `Arc`/`Db`) into each tool.
#[derive(Clone)]
pub struct SubthreadContext {
    pub db: toasty::Db,
    pub parent_thread_id: u64,
    pub parent_working_directory: String,
    pub registry: Arc<SubthreadRegistry>,
    pub config: Arc<SubthreadConfig>,
    /// Used by `set_subthread_settings` to evict a cached agent after a
    /// config change, and by `start_subthread` to activate/run the subthread.
    /// `None` in unit-test contexts.
    pub controller: Option<Arc<dyn crate::controller::ThreadController>>,
}

impl SubthreadContext {
    /// Validate that `subthread_id` exists, is owned by this parent, and
    /// return its row. Shared guard used by most tools.
    pub(crate) async fn require_owned_subthread(
        &self,
        subthread_id: u64,
    ) -> Result<Thread, ToolError> {
        let mut db = self.db.clone();
        let thread = toasty::query!(Thread FILTER .id == #subthread_id)
            .first()
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query subthread: {e}"))
            })?
            .ok_or_else(|| {
                ToolError::InvalidParameters(format!(
                    "No subthread with id {}",
                    subthread_id
                ))
            })?;
        if thread.own_by_id != Some(self.parent_thread_id) {
            return Err(ToolError::InvalidParameters(format!(
                "Thread {} is not a subthread of the current thread",
                subthread_id
            )));
        }
        Ok(thread)
    }

    /// Read whether a subthread has the `subthread` middleware enabled (i.e.
    /// `allow_subthread`). Shared by `list`/`inspect`.
    pub(crate) async fn allow_subthread(&self, subthread_id: u64) -> Result<bool, ToolError> {
        let mut db = self.db.clone();
        let rows = toasty::query!(Middleware FILTER .thread_id == #subthread_id)
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query middlewares: {e}"))
            })?;
        Ok(rows
            .into_iter()
            .any(|m| m.name == "subthread" && m.enabled))
    }
}

/// Create a subthread entity + seed default middlewares (shell, tool, and
/// optionally subthread). Does NOT activate the subthread.
pub struct SpawnSubthreadTool {
    ctx: SubthreadContext,
}

impl SpawnSubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for SpawnSubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "spawn_subthread".to_string(),
            description: "Spawn a child thread (subthread) of the current thread. The subthread's working_directory must be the current thread's working_directory or a subdirectory of it. Returns the new subthread's id. The subthread is created in an idle state; call start_subthread to run it.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "working_directory": {
                        "type": "string",
                        "description": "Working directory for the subthread. Must be within the current thread's working directory tree (the directory must already exist)."
                    },
                    "allow_subthread": {
                        "type": "boolean",
                        "description": "Whether this subthread may spawn its own sub-subthreads. Default false."
                    }
                },
                "required": ["working_directory"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let working_directory = params
            .get("working_directory")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing 'working_directory' parameter".into())
            })?;
        let allow_subthread = params
            .get("allow_subthread")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Validate containment against the parent's working directory.
        let parent = std::path::Path::new(&self.ctx.parent_working_directory);
        let canon = ensure_within(parent, working_directory).map_err(ToolError::InvalidParameters)?;
        let working_directory = canon.to_string_lossy().to_string();

        let mut db = self.ctx.db.clone();

        // Inherit the parent's workspace (find_or_create is idempotent on the
        // directory, and the parent already has one for this tree).
        let workspace = nekocode_entities::workspace::find_or_create(
            &mut db,
            &working_directory,
        )
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("Failed to find/create workspace: {e}"))
        })?;

        // Resolve the parent's model so the subthread uses the same provider.
        let parent_thread = toasty::query!(Thread FILTER .id == #(self.ctx.parent_thread_id))
            .first()
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query parent thread: {e}"))
            })?
            .ok_or_else(|| {
                ToolError::ExecutionError(format!(
                    "Parent thread {} not found",
                    self.ctx.parent_thread_id
                ))
            })?;

        let subthread = toasty::create!(Thread {
            working_directory: working_directory.clone(),
            model: parent_thread.model.clone(),
            workspace_id: Some(workspace.id),
            own_by_id: Some(self.ctx.parent_thread_id),
        })
        .exec(&mut db)
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("Failed to create subthread: {e}"))
        })?;

        // Seed default middlewares: shell + tool scoped to the subthread's wd.
        let shell_cfg = nekocode_shell_config_value(&working_directory);
        toasty::create!(Middleware {
            thread_id: subthread.id,
            name: "shell".to_string(),
            config: Json(shell_cfg),
        })
        .exec(&mut db)
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("Failed to seed shell middleware: {e}"))
        })?;

        let tool_cfg = nekocode_file_config_value(&working_directory);
        toasty::create!(Middleware {
            thread_id: subthread.id,
            name: "tool".to_string(),
            config: Json(tool_cfg),
        })
        .exec(&mut db)
        .await
        .map_err(|e| {
            ToolError::ExecutionError(format!("Failed to seed tool middleware: {e}"))
        })?;

        // Optionally seed the subthread middleware so this subthread can spawn
        // its own sub-subthreads. allow_subthread defaults to false inside the
        // nested config to bound recursion depth.
        if allow_subthread {
            let sub_cfg = SubthreadConfig { allow_subthread: false }.to_value();
            toasty::create!(Middleware {
                thread_id: subthread.id,
                name: "subthread".to_string(),
                config: Json(sub_cfg),
            })
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!(
                    "Failed to seed subthread middleware: {e}"
                ))
            })?;
        }

        // Track in-memory as Idle. The registry is per-parent (lives in this
        // parent's `Agent.extensions`), so the parent association is implicit
        // — no need to pass it.
        self.ctx.registry.insert_idle(subthread.id);

        Ok(serde_json::json!({
            "subthread_id": subthread.id,
            "working_directory": working_directory,
            "allow_subthread": allow_subthread,
        }))
    }
}

/// Build a `nekocode_shell::ShellConfig` JSON value for a subthread. Defined
/// here (rather than depending on the shell crate) because the subthread crate
/// must not depend on `nekocode-shell` — it only needs the JSON shape, which
/// is `{ "workingDirectory": <wd> }`. Keeping the literal here avoids a
/// cross-crate coupling that the spec doesn't require.
fn nekocode_shell_config_value(working_directory: &str) -> serde_json::Value {
    serde_json::json!({ "workingDirectory": working_directory })
}

/// Build a `nekocode_file::FileConfig` JSON value for a subthread. Same
/// rationale as above.
fn nekocode_file_config_value(working_directory: &str) -> serde_json::Value {
    serde_json::json!({ "workingDirectory": working_directory })
}

/// List all subthreads of the current thread, enriched with in-memory run
/// state and the `allow_subthread` flag.
pub struct ListSubthreadsTool {
    ctx: SubthreadContext,
}

impl ListSubthreadsTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for ListSubthreadsTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "list_subthreads".to_string(),
            description: "List all subthreads of the current thread, including each subthread's run state (idle/running/finished/error) and whether it may spawn sub-subthreads.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(
        &self,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let mut db = self.ctx.db.clone();
        let rows = toasty::query!(Thread FILTER .own_by_id == #(self.ctx.parent_thread_id) ORDER BY .id ASC)
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query subthreads: {e}"))
            })?;

        let mut out = Vec::with_capacity(rows.len());
        for t in rows {
            let run_state = self.ctx.registry.run_state(t.id);
            let allow = self.ctx.allow_subthread(t.id).await?;
            out.push(serde_json::json!({
                "subthread_id": t.id,
                "working_directory": t.working_directory,
                "run_state": run_state_name(&run_state),
                "allow_subthread": allow,
            }));
        }
        Ok(serde_json::json!({ "subthreads": out }))
    }
}

/// Inspect a single subthread's current state.
pub struct InspectSubthreadTool {
    ctx: SubthreadContext,
}

impl InspectSubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for InspectSubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "inspect_subthread".to_string(),
            description: "Inspect a subthread's current state: run state (idle/running/finished/error), working directory, and whether it may spawn sub-subthreads.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    }
                },
                "required": ["subthread_id"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        let thread = self.ctx.require_owned_subthread(subthread_id).await?;
        let run_state = self.ctx.registry.run_state(subthread_id);
        let allow = self.ctx.allow_subthread(subthread_id).await?;
        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "run_state": run_state_name(&run_state),
            "working_directory": thread.working_directory,
            "allow_subthread": allow,
        }))
    }
}

/// Map a `SubthreadRunState` to a stable lowercase name for JSON output.
pub(crate) fn run_state_name(state: &crate::registry::SubthreadRunState) -> &'static str {
    use crate::registry::SubthreadRunState::*;
    match state {
        Idle => "idle",
        Running => "running",
        Finished => "finished",
        Error(_) => "error",
    }
}

/// Parse the `subthread_id` integer parameter shared by most tools.
pub(crate) fn parse_subthread_id(params: &serde_json::Value) -> Result<u64, ToolError> {
    params
        .get("subthread_id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing or invalid 'subthread_id' parameter".into())
        })
}

use nekocode_entities::turn::Turn;

/// Read a subthread's message history from the database. Returns turns (each
/// with its messages) in chronological order. Supports pagination via
/// `start_turn` (0-based) and `limit` (default 10).
///
/// When `text_only` is true (the default), the response strips non-prose
/// content: reasoning blocks within assistant messages are dropped, tool-call
/// blocks are dropped, and tool-call-result messages are filtered out
/// entirely. The caller gets the conversation's actual prose only — useful
/// when the model needs to recall what was said without re-reading the
/// reasoning/tool plumbing. Pass `text_only: false` to retrieve the full
/// structured message stream (including tool calls and their results).
pub struct ReadSubthreadTool {
    ctx: SubthreadContext,
}

impl ReadSubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for ReadSubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "read_subthread".to_string(),
            description: "Read a subthread's message history from the database. Returns turns (each with its messages) in chronological order. Supports pagination via start_turn (0-based) and limit (default 10). By default (text_only=true) the response contains only the conversation's prose — reasoning blocks, tool calls, and tool results are stripped. Pass text_only=false to retrieve the full structured stream.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    },
                    "start_turn": {
                        "type": "integer",
                        "description": "0-based turn index to start from. Default 0."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of turns to return. Default 10."
                    },
                    "text_only": {
                        "type": "boolean",
                        "description": "When true (default), strip non-prose content from the response: drop reasoning blocks and tool-call blocks inside assistant messages, and filter out tool-call-result messages entirely. When false, return the full structured message stream (including tool calls and their results)."
                    }
                },
                "required": ["subthread_id"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        // Validate ownership even though we only read.
        self.ctx.require_owned_subthread(subthread_id).await?;

        let start_turn = params
            .get("start_turn")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10);
        if limit == 0 {
            return Err(ToolError::InvalidParameters(
                "'limit' must be >= 1".into(),
            ));
        }
        // Default true: the tool is intended to give the model back its own
        // conversation prose, not the implementation plumbing around it.
        let text_only = params
            .get("text_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut db = self.ctx.db.clone();
        // Load all turns for the subthread in order, then paginate in-memory.
        // toasty's query DSL lacks a clean OFFSET; the turn counts are small
        // enough that this is acceptable. If a subthread grows large, switch
        // to a turn_index range filter.
        let turns = toasty::query!(Turn FILTER .thread_id == #subthread_id ORDER BY .turn_index ASC)
            .include(Turn::fields().messages())
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query turns: {e}"))
            })?;

        let mut out = Vec::new();
        for turn in turns
            .into_iter()
            .skip(usize::try_from(start_turn).unwrap_or(usize::MAX))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
        {
            let messages: Vec<serde_json::Value> = turn
                .messages
                .get()
                .iter()
                .filter_map(|m| {
                    let raw = serde_json::to_value(&m.content.0).ok()?;
                    if text_only {
                        strip_non_prose(raw)
                    } else {
                        Some(raw)
                    }
                })
                .collect();
            out.push(serde_json::json!({
                "turn_index": turn.turn_index,
                "finished": turn.finished,
                "messages": messages,
            }));
        }
        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "turns": out,
        }))
    }
}

/// Filter a serialized `ChatMessage` down to its prose for `text_only` mode.
///
/// - `user` / `middlewareMessage`: kept as-is (they are pure text).
/// - `toolCallResult`: dropped entirely (not prose).
/// - `assistant`: only `text` blocks are kept; `reasoningContent` is
///   dropped, `toolCall` blocks are dropped. If the assistant message has no
///   remaining text blocks, the message itself is dropped (returns `None` so
///   the caller can `filter_map` it out).
fn strip_non_prose(mut message: serde_json::Value) -> Option<serde_json::Value> {
    let obj = message.as_object_mut()?;
    let msg_type = obj.get("type")?.as_str()?;
    match msg_type {
        "user" | "middlewareMessage" => Some(message),
        "toolCallResult" => None,
        "assistant" => {
            // The assistant message body is wrapped in `data`; only its
            // `blocks` array carries prose vs plumbing.
            let data = obj.get_mut("data")?.as_object_mut()?;
            if let Some(blocks) = data.get_mut("blocks").and_then(|b| b.as_array_mut()) {
                blocks.retain(|block| {
                    block.get("type").and_then(|t| t.as_str()) == Some("text")
                });
            }
            // Drop the message entirely if no text blocks survived, so the
            // caller doesn't see an empty assistant turn.
            let has_text = data
                .get("blocks")
                .and_then(|b| b.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);
            if has_text { Some(message) } else { None }
        }
        _ => Some(message),
    }
}

/// View a subthread's middleware settings. Same shape as the API's
/// `list_middlewares` endpoint.
pub struct SubthreadSettingsTool {
    ctx: SubthreadContext,
}

impl SubthreadSettingsTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for SubthreadSettingsTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "subthread_settings".to_string(),
            description: "View a subthread's middleware settings (id, name, config, enabled for each middleware row).".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    }
                },
                "required": ["subthread_id"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        self.ctx.require_owned_subthread(subthread_id).await?;
        let mut db = self.ctx.db.clone();
        let rows = toasty::query!(Middleware FILTER .thread_id == #subthread_id)
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query middlewares: {e}"))
            })?;
        let middlewares: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "name": m.name,
                    "config": m.config.0,
                    "enabled": m.enabled,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "middlewares": middlewares,
        }))
    }
}

/// Modify a subthread's middleware settings (config and/or enabled). Refuses
/// while the subthread is running.
pub struct SetSubthreadSettingsTool {
    ctx: SubthreadContext,
}

impl SetSubthreadSettingsTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for SetSubthreadSettingsTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "set_subthread_settings".to_string(),
            description: "Modify a subthread's middleware settings. Update a middleware row's config and/or enabled flag. Refuses while the subthread is running (the parent must wait for completion). After updating, the subthread's cached agent is evicted so the next start_subthread picks up the change.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    },
                    "middleware_id": {
                        "type": "integer",
                        "description": "The middleware row id to update."
                    },
                    "config": {
                        "type": "object",
                        "description": "New config JSON for the middleware. Optional."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "New enabled flag. Optional."
                    }
                },
                "required": ["subthread_id", "middleware_id"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        let middleware_id = params
            .get("middleware_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "Missing or invalid 'middleware_id' parameter".into(),
                )
            })?;
        self.ctx.require_owned_subthread(subthread_id).await?;

        // Refuse while running — the cached agent would diverge from the DB.
        if matches!(
            self.ctx.registry.run_state(subthread_id),
            crate::registry::SubthreadRunState::Running
        ) {
            return Err(ToolError::ExecutionError(
                "subthread is currently running; wait for it to finish before changing settings".into(),
            ));
        }

        let config = params.get("config").cloned();
        let enabled = params.get("enabled").and_then(|v| v.as_bool());

        let mut db = self.ctx.db.clone();
        // Verify the middleware row belongs to this subthread.
        let mw = toasty::query!(Middleware FILTER .id == #middleware_id)
            .first()
            .exec(&mut db)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to query middleware: {e}"))
            })?
            .ok_or_else(|| {
                ToolError::InvalidParameters(format!(
                    "No middleware with id {}",
                    middleware_id
                ))
            })?;
        if mw.thread_id != subthread_id {
            return Err(ToolError::InvalidParameters(format!(
                "Middleware {} does not belong to subthread {}",
                middleware_id, subthread_id
            )));
        }

        let mut update = toasty::query!(Middleware FILTER .id == #middleware_id).update();
        if let Some(config) = config {
            update.set_config(Json(config));
        }
        if let Some(enabled) = enabled {
            update.set_enabled(enabled);
        }
        update.exec(&mut db).await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to update middleware: {e}"))
        })?;

        // Evict any cached agent so the next start_subthread rebuilds it.
        if let Some(controller) = &self.ctx.controller {
            controller.deactivate(subthread_id).await;
        }

        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "middleware_id": middleware_id,
            "updated": true,
        }))
    }
}

/// Activate a subthread and run it with a prompt in the background. Returns
/// immediately with `status: "started"`; the parent polls via
/// `inspect_subthread` / `wait_*_subthread` and reads results via
/// `read_subthread`. Refuses if the subthread is already running.
pub struct StartSubthreadTool {
    ctx: SubthreadContext,
}

impl StartSubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for StartSubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "start_subthread".to_string(),
            description: "Start a subthread running with a given prompt. Activates the subthread (builds its agent from its middleware settings) and runs it in a background task. Returns immediately with status 'started'. Poll completion via inspect_subthread, wait_any_subthread, or wait_all_subthreads; read results via read_subthread. Refuses if the subthread is already running.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The user message to send as the subthread's first turn."
                    }
                },
                "required": ["subthread_id", "prompt"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing 'prompt' parameter".into())
            })?
            .to_string();
        self.ctx.require_owned_subthread(subthread_id).await?;

        // Reject concurrent runs.
        if matches!(
            self.ctx.registry.run_state(subthread_id),
            crate::registry::SubthreadRunState::Running
        ) {
            return Err(ToolError::ExecutionError(format!(
                "subthread {} is already running",
                subthread_id
            )));
        }

        let controller = self.ctx.controller.clone().ok_or_else(|| {
            ToolError::ExecutionError(
                "no thread controller configured; cannot start subthread".into(),
            )
        })?;

        // Activate (build the agent, insert into active_threads) — or reuse the
        // already-activated agent. `start_subthread`'s semantics are "start the
        // agent run loop", so an already-activated (but not running) subthread
        // is a valid reuse target, not an error. The only refusal happens
        // earlier, when `run_state == Running`.
        let agent = controller
            .activate(subthread_id)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to activate subthread: {e}")))?;
        let agent = match agent {
            crate::controller::ActivationOutcome::Activated(a)
            | crate::controller::ActivationOutcome::AlreadyActivated(a) => a,
        };

        // Spawn the background run_loop. Events are discarded; results land in
        // the DB and are read via read_subthread.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let registry = self.ctx.registry.clone();
        let controller_for_task = controller.clone();
        let handle = tokio::spawn(async move {
            // Drain the event channel so the agent's send() calls never block
            // when no one is consuming. Aborted once the run completes.
            let drain = tokio::spawn(async move {
                while rx.recv().await.is_some() {}
            });

            let result = controller_for_task
                .run(agent, prompt, nekocode_core::agent::AgentEventSink::new(tx))
                .await;
            drain.abort();

            match result {
                Ok(()) => registry.set_finished(subthread_id),
                Err(e) => registry.set_error(subthread_id, e.to_string()),
            }
            controller_for_task.deactivate(subthread_id).await;
        });

        self.ctx.registry.set_running(subthread_id, handle);

        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "status": "started",
        }))
    }
}


use std::time::Duration;

/// Wait until any one of the specified subthreads becomes ready (Finished or
/// Error), or until the timeout elapses. On timeout the subthreads keep
/// running; the parent may call again or proceed.
pub struct WaitAnySubthreadTool {
    ctx: SubthreadContext,
}

impl WaitAnySubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAnySubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "wait_any_subthread".to_string(),
            description: "Wait until any one of the specified subthreads becomes ready (finished or errored), or until the timeout elapses. Returns the first ready subthread on success, or the list of still-pending subthreads on timeout. Does NOT kill or affect running subthreads on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The subthread ids to wait on."
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Maximum seconds to wait. Must be positive."
                    }
                },
                "required": ["subthread_ids", "timeout"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let ids = parse_subthread_ids(&params)?;
        let timeout_secs = parse_timeout(&params)?;
        // Validate ownership of every id.
        for id in &ids {
            self.ctx.require_owned_subthread(*id).await?;
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            // Check for an already-ready subthread first.
            for id in &ids {
                let state = self.ctx.registry.run_state(*id);
                if state.is_ready() {
                    return Ok(serde_json::json!({
                        "status": "ready",
                        "subthread_id": id,
                        "run_state": run_state_name(&state),
                    }));
                }
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "pending": ids,
                }));
            }

            // Collect Notify handles to await. Re-collect each iteration in case
            // entries were added/removed.
            let notifies: Vec<_> = ids
                .iter()
                .filter_map(|id| self.ctx.registry.notify(*id))
                .collect();

            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifies.is_empty() {
                // Nothing to wait on (no registry entries); just sleep to deadline.
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(&notifies) => {}
                }
            }
            // Loop and re-check.
        }
    }
}

/// Wait until all specified subthreads are ready, or until the timeout
/// elapses. With no `subthread_ids`, defaults to all of the parent's
/// currently-Running subthreads (excludes Idle so it doesn't block forever).
pub struct WaitAllSubthreadsTool {
    ctx: SubthreadContext,
}

impl WaitAllSubthreadsTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAllSubthreadsTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "wait_all_subthreads".to_string(),
            description: "Wait until all specified subthreads are ready (finished or errored), or until the timeout elapses. With no subthread_ids, defaults to all of the current thread's subthreads that are currently running. On timeout, returns the ready and pending lists separately. Does NOT kill or affect running subthreads on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The subthread ids to wait on. If omitted, waits on all of the current thread's currently-running subthreads."
                    },
                    "timeout": {
                        "type": "number",
                        "description": "Maximum seconds to wait. Must be positive."
                    }
                },
                "required": ["timeout"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let timeout_secs = parse_timeout(&params)?;

        // Resolve the target set.
        let ids: Vec<u64> = if params.get("subthread_ids").and_then(|v| v.as_array()).is_some() {
            let ids = parse_subthread_ids(&params)?;
            for id in &ids {
                self.ctx.require_owned_subthread(*id).await?;
            }
            ids
        } else {
            // Default: all of the parent's currently-running subthreads.
            self.ctx
                .registry
                .all_thread_ids()
                .into_iter()
                .filter(|id| {
                    matches!(
                        self.ctx.registry.run_state(*id),
                        crate::registry::SubthreadRunState::Running
                    )
                })
                .collect()
        };

        if ids.is_empty() {
            return Ok(serde_json::json!({
                "status": "ready",
                "results": [],
            }));
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            let (mut ready, mut pending) = (Vec::new(), Vec::new());
            for id in &ids {
                let state = self.ctx.registry.run_state(*id);
                if state.is_ready() {
                    ready.push(serde_json::json!({
                        "subthread_id": id,
                        "run_state": run_state_name(&state),
                    }));
                } else {
                    pending.push(*id);
                }
            }
            if pending.is_empty() {
                return Ok(serde_json::json!({
                    "status": "ready",
                    "results": ready,
                }));
            }

            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "ready": ready,
                    "pending": pending,
                }));
            }

            let notifies: Vec<_> = ids
                .iter()
                .filter_map(|id| self.ctx.registry.notify(*id))
                .collect();
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifies.is_empty() {
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(&notifies) => {}
                }
            }
        }
    }
}

/// Parse the `subthread_ids` array parameter.
pub(crate) fn parse_subthread_ids(params: &serde_json::Value) -> Result<Vec<u64>, ToolError> {
    let arr = params
        .get("subthread_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing 'subthread_ids' array parameter".into())
        })?;
    if arr.is_empty() {
        return Err(ToolError::InvalidParameters(
            "'subthread_ids' must be a non-empty array".into(),
        ));
    }
    arr.iter()
        .map(|v| {
            v.as_u64().ok_or_else(|| {
                ToolError::InvalidParameters(
                    "'subthread_ids' must contain integers".into(),
                )
            })
        })
        .collect()
}

/// Parse and validate the `timeout` (positive seconds) parameter.
pub(crate) fn parse_timeout(params: &serde_json::Value) -> Result<f64, ToolError> {
    let secs = params
        .get("timeout")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| {
            ToolError::InvalidParameters("Missing 'timeout' parameter".into())
        })?;
    if !secs.is_finite() || secs <= 0.0 {
        return Err(ToolError::InvalidParameters(format!(
            "'timeout' must be a positive number of seconds, got {secs}"
        )));
    }
    Ok(secs)
}

/// Wait for any of the given `Notify` handles to fire. Each handle is awaited
/// inside its own pinned async block so the `Notified` future (which borrows
/// its `Notify`) is owned by the block; `select_all` races them.
async fn notify_any(notifies: &[std::sync::Arc<tokio::sync::Notify>]) {
    use futures_util::future::select_all;
    use std::future::Future;
    use std::pin::Pin;
    if notifies.is_empty() {
        std::future::pending::<()>().await;
        return;
    }
    let futures: Vec<Pin<Box<dyn Future<Output = ()> + Send>>> = notifies
        .iter()
        .map(|n| {
            let n = n.clone();
            Box::pin(async move { n.notified().await })
                as Pin<Box<dyn Future<Output = ()> + Send>>
        })
        .collect();
    let _ = select_all(futures).await;
}

/// Delete a subthread and all of its descendants recursively. Aborts any
/// in-flight background tasks they own, evicts them from the active-thread
/// caches, and deletes their messages → turns → middlewares → thread rows in
/// one transaction. Refuses if the subthread (or any descendant) is
/// mid-generation. After deletion the subthread is gone from both the DB and
/// the in-memory registry.
pub struct DeleteSubthreadTool {
    ctx: SubthreadContext,
}

impl DeleteSubthreadTool {
    pub fn new(ctx: SubthreadContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for DeleteSubthreadTool {
    fn spec(&self) -> nekocode_types::tool::ToolSpec {
        nekocode_types::tool::ToolSpec {
            name: "delete_subthread".to_string(),
            description: "Delete a subthread and all of its descendants recursively. Aborts any in-flight subthread background tasks, evicts them from the active-thread caches, and deletes their messages, turns, middlewares, and thread rows in one transaction. Refuses if the subthread (or any descendant) is currently mid-generation. Use this to clean up subthreads you no longer need.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subthread_id": {
                        "type": "integer",
                        "description": "The subthread id returned by spawn_subthread."
                    }
                },
                "required": ["subthread_id"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let subthread_id = parse_subthread_id(&params)?;
        // Validate ownership before delegating to the controller's cascade
        // delete — the controller operates on arbitrary thread ids, so we gate
        // it here to ensure only the owning parent can delete a subthread.
        self.ctx.require_owned_subthread(subthread_id).await?;

        let controller = self.ctx.controller.clone().ok_or_else(|| {
            ToolError::ExecutionError(
                "no thread controller configured; cannot delete subthread".into(),
            )
        })?;

        // The controller's delete_subthread handles the full cascade: abort
        // in-flight tasks (via each descendant's per-parent registry in
        // Agent.extensions), evict from active_threads/generate_states, and
        // delete DB rows in one transaction.
        controller
            .delete_subthread(subthread_id)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to delete subthread: {e}"))
            })?;

        // Also drop the entry from this parent's own in-memory registry so
        // list_subthreads / inspect_subthread stop reporting it immediately.
        // (The controller's cascade already aborted any task this parent's
        // registry held for the subthread via abort_all_and_clear, but the
        // registry entry itself may still be present if the subthread was
        // idle rather than running.)
        self.ctx.registry.remove(subthread_id);

        Ok(serde_json::json!({
            "subthread_id": subthread_id,
            "deleted": true,
        }))
    }
}
