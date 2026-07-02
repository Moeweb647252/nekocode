//! Integration tests for nekocode-subthread tools against a temp DB.
//!
//! These cover spawn/list/inspect/read/settings and the working-directory
//! containment rule. `start_subthread` and the wait tools need a live
//! provider (the real `ThreadController`), so they are covered by a manual
//! API-layer smoke test rather than here.

use std::sync::Arc;

use nekocode_core::extensions::Extensions;
use nekocode_entities::{middleware::Middleware, prepare_db, thread::Thread};
use nekocode_subthread::{
    SubthreadConfig, SubthreadMiddleware,
    controller::{ActivationOutcome, ThreadController},
};

/// A no-op controller used only to satisfy the middleware constructor in tests
/// that don't exercise start_subthread. Its `activate` and `run` are never
/// called (the integration tests only invoke spawn/list/inspect/read/settings
/// /delete), but the trait requires a returnable `Agent` for `AlreadyActivated`,
/// so we hand back a placeholder built against a panic-on-call provider.
///
/// `delete_subthread` does real (simplified) DB cascade cleanup so the
/// `delete_subthread` tool test can verify end-to-end behavior without the
/// full API-layer cascade function (which lives in the `nekocode` crate).
struct NoopController {
    db: toasty::Db,
}

struct UnusedProvider;

#[async_trait::async_trait]
impl nekocode_core::provider::Provider for UnusedProvider {
    async fn stream_generate(
        &self,
        _request: nekocode_core::types::GenerateRequest,
        _sender: tokio::sync::mpsc::UnboundedSender<nekocode_core::provider::ProviderEvent>,
    ) -> Result<nekocode_core::provider::ProviderResponse, nekocode_core::provider::ProviderError>
    {
        panic!("UnusedProvider::stream_generate must not be called in tests")
    }
}

#[async_trait::async_trait]
impl ThreadController for NoopController {
    async fn activate(&self, _: u64) -> Result<ActivationOutcome, anyhow::Error> {
        // The variant now carries an Arc<Agent>; we never run this agent in
        // these tests, so a minimal placeholder is fine.
        Ok(ActivationOutcome::AlreadyActivated(std::sync::Arc::new(
            nekocode_core::agent::Agent {
                thread_id: 0,
                working_directory: String::new(),
                db: self.db.clone(),
                middlewares: std::sync::Arc::new(Vec::new()),
                provider: std::sync::Arc::new(UnusedProvider),
                extensions: Extensions::new(),
            },
        )))
    }
    async fn deactivate(&self, _: u64) {}
    async fn run(
        &self,
        _: Arc<nekocode_core::agent::Agent>,
        _: String,
        _: nekocode_core::agent::AgentEventSink,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
    async fn delete_subthread(&self, subthread_id: u64) -> Result<(), anyhow::Error> {
        // Simplified cascade: collect descendants via own_by_id, then delete
        // middleware + thread rows for each. The full API-layer version also
        // aborts in-flight tasks and evicts active_threads; this test path has
        // none of those, so DB-only cleanup suffices to prove the tool wires
        // through to the controller.
        let mut db = self.db.clone();
        let mut to_delete = vec![subthread_id];
        let mut frontier = vec![subthread_id];
        while let Some(parent) = frontier.pop() {
            let children =
                toasty::query!(nekocode_entities::thread::Thread FILTER .own_by_id == #parent)
                    .exec(&mut db)
                    .await?;
            for child in children {
                if !to_delete.contains(&child.id) {
                    to_delete.push(child.id);
                    frontier.push(child.id);
                }
            }
        }
        let mut tx = db.transaction().await?;
        for id in &to_delete {
            toasty::query!(Middleware FILTER .thread_id == #id)
                .delete()
                .exec(&mut tx)
                .await?;
            toasty::query!(nekocode_entities::thread::Thread FILTER .id == #id)
                .delete()
                .exec(&mut tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn test_db_path() -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nekocode_subthread_integration_{}_{}.db",
        std::process::id(),
        n
    ))
}

/// Set up a parent thread (inside a real temp working directory) plus a
/// `subthread` middleware row on it. Returns the db and parent.
async fn setup() -> (toasty::Db, Thread) {
    let mut db = prepare_db(test_db_path()).await.expect("prepare_db");
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let parent_wd = std::env::temp_dir().join(format!(
        "nekocode_subthread_parent_{}_{}",
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&parent_wd).unwrap();
    let parent_wd_str = parent_wd.to_string_lossy().to_string();
    let parent = toasty::create!(Thread {
        working_directory: parent_wd_str.clone(),
        model: "default".to_string(),
    })
    .exec(&mut db)
    .await
    .expect("create parent");
    toasty::create!(Middleware {
        thread_id: parent.id,
        name: "subthread".to_string(),
        config: toasty::Json(SubthreadConfig::default().to_value()),
    })
    .exec(&mut db)
    .await
    .expect("seed subthread middleware");
    (db, parent)
}

/// Build a middleware (which creates its own per-parent registry and publishes
/// it to `extensions`), register its tools into a fresh `ToolRegistry`, and
/// return the registry alongside the tool registry so tests can invoke tools
/// directly and inspect in-memory state.
async fn build_tools(
    db: toasty::Db,
    parent: &Thread,
) -> (
    nekocode_types::tool::ToolRegistry,
    Arc<nekocode_subthread::SubthreadRegistry>,
) {
    let extensions = Extensions::new();
    let mw = SubthreadMiddleware::new(
        extensions.clone(),
        db.clone(),
        parent.id,
        parent.working_directory.clone(),
        SubthreadConfig::default(),
        Arc::new(NoopController { db }),
    );
    let mut reg = nekocode_types::tool::ToolRegistry::new();
    let mut req = nekocode_core::types::GenerateRequest::default();
    let (mev_tx, _mev_rx) = tokio::sync::mpsc::unbounded_channel();
    <SubthreadMiddleware as nekocode_core::middleware::Middleware>::before_generate(
        &mw, &mut req, &mut reg, &mev_tx,
    )
    .await
    .expect("before_generate");
    // Pull the per-parent registry back out of extensions.
    let registry = extensions
        .get::<nekocode_subthread::SubthreadRegistry>()
        .expect("subthread registry published to extensions");
    (reg, registry)
}

#[tokio::test]
async fn spawn_then_list_then_inspect() {
    let (db, parent) = setup().await;
    let child_dir = std::path::Path::new(&parent.working_directory).join("child");
    std::fs::create_dir_all(&child_dir).unwrap();
    let (tools, _registry) = build_tools(db.clone(), &parent).await;

    let spawn = tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let sub_id = out["subthread_id"].as_u64().unwrap();

    let list = tools.get("list_subthreads").unwrap();
    let out = list.call(serde_json::json!({})).await.expect("list");
    assert_eq!(out["subthreads"].as_array().unwrap().len(), 1);
    assert_eq!(out["subthreads"][0]["subthread_id"].as_u64(), Some(sub_id));
    assert_eq!(out["subthreads"][0]["run_state"], "idle");

    let inspect = tools.get("inspect_subthread").unwrap();
    let out = inspect
        .call(serde_json::json!({ "subthread_id": sub_id }))
        .await
        .expect("inspect");
    assert_eq!(out["run_state"], "idle");
    assert_eq!(out["allow_subthread"], false);
}

#[tokio::test]
async fn spawn_rejects_outside_parent() {
    let (db, parent) = setup().await;
    let (tools, _registry) = build_tools(db.clone(), &parent).await;
    let outside =
        std::env::temp_dir().join(format!("nekocode_subthread_outside_{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let spawn = tools.get("spawn_subthread").unwrap();
    let err = spawn
        .call(serde_json::json!({
            "working_directory": outside.to_string_lossy()
        }))
        .await
        .expect_err("should reject outside wd");
    let msg = err.to_string();
    assert!(msg.contains("outside"), "got: {msg}");
}

#[tokio::test]
async fn spawn_with_allow_subthread_seeds_subthread_middleware() {
    let (db, parent) = setup().await;
    let child_dir = std::path::Path::new(&parent.working_directory).join("child2");
    std::fs::create_dir_all(&child_dir).unwrap();
    let (tools, _registry) = build_tools(db.clone(), &parent).await;
    let spawn = tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": child_dir.to_string_lossy(),
            "allow_subthread": true
        }))
        .await
        .expect("spawn");
    let sub_id = out["subthread_id"].as_u64().unwrap();

    let settings = tools.get("subthread_settings").unwrap();
    let out = settings
        .call(serde_json::json!({ "subthread_id": sub_id }))
        .await
        .expect("settings");
    let names: Vec<&str> = out["middlewares"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"tool"));
    assert!(names.contains(&"subthread"));
}

#[tokio::test]
async fn read_subthread_returns_empty_turns_when_unused() {
    let (db, parent) = setup().await;
    let child_dir = std::path::Path::new(&parent.working_directory).join("child3");
    std::fs::create_dir_all(&child_dir).unwrap();
    let (tools, _registry) = build_tools(db.clone(), &parent).await;
    let spawn = tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let sub_id = out["subthread_id"].as_u64().unwrap();

    let read = tools.get("read_subthread").unwrap();
    let out = read
        .call(serde_json::json!({ "subthread_id": sub_id }))
        .await
        .expect("read");
    assert_eq!(out["turns"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn cascade_delete_removes_subthreads() {
    let (mut db, parent) = setup().await;
    let child_dir = std::path::Path::new(&parent.working_directory).join("cascade");
    std::fs::create_dir_all(&child_dir).unwrap();
    let (tools, registry) = build_tools(db.clone(), &parent).await;
    let spawn = tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let sub_id = out["subthread_id"].as_u64().unwrap();

    let children = toasty::query!(Thread FILTER .own_by_id == #(parent.id))
        .exec(&mut db)
        .await
        .expect("query children");
    assert_eq!(children.len(), 1);

    // Emulate the API-layer cascade: abort the parent's subthread tasks via
    // its per-parent registry, then delete the rows.
    let _ = registry.abort_all_and_clear();
    let mut tx = db.transaction().await.expect("tx");
    for turn in toasty::query!(nekocode_entities::turn::Turn FILTER .thread_id == #sub_id)
        .exec(&mut tx)
        .await
        .expect("turns")
    {
        toasty::query!(nekocode_entities::message::Message FILTER .turn_id == #(turn.id))
            .delete()
            .exec(&mut tx)
            .await
            .expect("del msgs");
    }
    toasty::query!(nekocode_entities::turn::Turn FILTER .thread_id == #sub_id)
        .delete()
        .exec(&mut tx)
        .await
        .expect("del turns");
    toasty::query!(Middleware FILTER .thread_id == #sub_id)
        .delete()
        .exec(&mut tx)
        .await
        .expect("del mw");
    toasty::query!(Thread FILTER .id == #sub_id)
        .delete()
        .exec(&mut tx)
        .await
        .expect("del thread");
    tx.commit().await.expect("commit");

    let remaining = toasty::query!(Thread FILTER .own_by_id == #(parent.id))
        .exec(&mut db)
        .await
        .expect("query");
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn inspect_rejects_subthread_not_owned_by_parent() {
    let (db, parent) = setup().await;
    // Create a second parent and spawn a subthread under its tree using its
    // own tools, then try to inspect that subthread from the first parent's
    // tools — must be refused.
    let other_wd = std::env::temp_dir().join(format!(
        "nekocode_subthread_other_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&other_wd).unwrap();
    let mut db2 = db.clone();
    let other_parent = toasty::create!(Thread {
        working_directory: other_wd.to_string_lossy().to_string(),
        model: "default".to_string(),
    })
    .exec(&mut db2)
    .await
    .expect("create other parent");
    let other_child_dir = other_wd.join("oc");
    std::fs::create_dir_all(&other_child_dir).unwrap();

    let (other_tools, _other_registry) = build_tools(db.clone(), &other_parent).await;
    let spawn = other_tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": other_child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let foreign_sub_id = out["subthread_id"].as_u64().unwrap();

    // Now inspect from the FIRST parent's tools: must be refused because the
    // subthread is not owned by this parent (own_by_id mismatch).
    let (tools, _registry) = build_tools(db.clone(), &parent).await;
    let inspect = tools.get("inspect_subthread").unwrap();
    let err = inspect
        .call(serde_json::json!({ "subthread_id": foreign_sub_id }))
        .await
        .expect_err("must reject foreign subthread");
    let msg = err.to_string();
    assert!(
        msg.contains("not a subthread") || msg.contains("No subthread"),
        "got: {msg}"
    );
}

#[tokio::test]
async fn delete_subthread_removes_it_from_db_and_registry() {
    let (db, parent) = setup().await;
    let child_dir = std::path::Path::new(&parent.working_directory).join("del-child");
    std::fs::create_dir_all(&child_dir).unwrap();
    let (tools, registry) = build_tools(db.clone(), &parent).await;

    // Spawn a subthread.
    let spawn = tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let sub_id = out["subthread_id"].as_u64().unwrap();

    // It shows up in list_subthreads and the registry.
    let list = tools.get("list_subthreads").unwrap();
    let listed = list.call(serde_json::json!({})).await.expect("list");
    assert_eq!(listed["subthreads"].as_array().unwrap().len(), 1);
    assert!(registry.contains(sub_id));

    // Delete it via the tool.
    let delete = tools.get("delete_subthread").unwrap();
    let out = delete
        .call(serde_json::json!({ "subthread_id": sub_id }))
        .await
        .expect("delete");
    assert_eq!(out["deleted"], true);

    // Gone from the DB (no thread with that id, no children of parent).
    let mut db2 = db.clone();
    let row = toasty::query!(Thread FILTER .id == #sub_id)
        .first()
        .exec(&mut db2)
        .await
        .expect("query");
    assert!(row.is_none(), "subthread row should be deleted");

    let children = toasty::query!(Thread FILTER .own_by_id == #(parent.id))
        .exec(&mut db2)
        .await
        .expect("query children");
    assert!(children.is_empty());

    // Gone from the in-memory registry too.
    assert!(!registry.contains(sub_id));

    // list_subthreads now reports zero.
    let listed = list.call(serde_json::json!({})).await.expect("list");
    assert_eq!(listed["subthreads"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn delete_subthread_rejects_foreign_subthread() {
    let (db, parent) = setup().await;
    // Spawn a subthread under a different parent's tree using that parent's
    // tools, then attempt to delete it from the first parent's tools.
    let other_wd = std::env::temp_dir().join(format!(
        "nekocode_subthread_del_foreign_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&other_wd).unwrap();
    let mut db2 = db.clone();
    let other_parent = toasty::create!(Thread {
        working_directory: other_wd.to_string_lossy().to_string(),
        model: "default".to_string(),
    })
    .exec(&mut db2)
    .await
    .expect("create other parent");
    let other_child_dir = other_wd.join("oc");
    std::fs::create_dir_all(&other_child_dir).unwrap();

    let (other_tools, _other_registry) = build_tools(db.clone(), &other_parent).await;
    let spawn = other_tools.get("spawn_subthread").unwrap();
    let out = spawn
        .call(serde_json::json!({
            "working_directory": other_child_dir.to_string_lossy()
        }))
        .await
        .expect("spawn");
    let foreign_sub_id = out["subthread_id"].as_u64().unwrap();

    // Delete from the FIRST parent's tools: must be refused (ownership check).
    let (tools, _registry) = build_tools(db.clone(), &parent).await;
    let delete = tools.get("delete_subthread").unwrap();
    let err = delete
        .call(serde_json::json!({ "subthread_id": foreign_sub_id }))
        .await
        .expect_err("must reject foreign subthread");
    let msg = err.to_string();
    assert!(
        msg.contains("not a subthread") || msg.contains("No subthread"),
        "got: {msg}"
    );

    // The foreign subthread must still exist (deletion was refused).
    let mut db3 = db.clone();
    let row = toasty::query!(Thread FILTER .id == #foreign_sub_id)
        .first()
        .exec(&mut db3)
        .await
        .expect("query");
    assert!(row.is_some(), "foreign subthread must survive refused delete");
}
