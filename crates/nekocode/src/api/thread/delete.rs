use std::sync::Arc;

use crate::api::prelude::*;

#[derive(Deserialize)]
pub struct DeleteThread {
    pub id: u64,
}

/// Delete a thread and, if it is a parent, all of its subthreads recursively.
/// Refuses if the parent (or any subthread) is mid-generation. Delegates to
/// [`delete_threads_cascade`] for the shared cascade logic also used by the
/// `delete_subthread` tool.
pub async fn delete_thread(
    State(state): State<AppState>,
    Json(payload): Json<DeleteThread>,
) -> ApiResult {
    delete_threads_cascade(
        &state.db,
        &state.active_threads,
        &state.generate_states,
        payload.id,
    )
    .await?;
    ApiResponse::ok(())
}

/// Delete `root` and every thread reachable from it via `own_by_id`, in one
/// transaction. For each thread in the closure:
/// 1. If it is currently activated, read its per-parent `SubthreadRegistry`
///    and `SubagentRegistry` from `Agent.extensions` and abort every in-flight
///    subthread and subagent background task it owns (children first, though
///    order is not load-bearing).
/// 2. Evict it from `active_threads` / `generate_states`.
/// 3. Delete its messages → turns → middlewares → thread row.
///
/// Refuses if any thread in the set is mid-generation (`generate_states`).
/// Shared by the `/thread/delete` handler and the `delete_subthread` tool
/// (via `ApiThreadController`).
pub async fn delete_threads_cascade(
    db: &toasty::Db,
    active_threads: &dashmap::DashMap<u64, Arc<tokio::sync::RwLock<nekocode_core::agent::Agent>>>,
    generate_states: &dashmap::DashMap<u64, Arc<crate::api::generate::GenerateState>>,
    root: u64,
) -> Result<(), ApiError> {
    // Refuse to delete a thread that is mid-generation.
    if generate_states.contains_key(&root) {
        return Err(ApiError::ThreadGenerating);
    }

    // Collect the full transitive closure of threads to delete: the root plus
    // every thread reachable via own_by_id.
    let mut to_delete: Vec<u64> = vec![root];
    let mut frontier: Vec<u64> = vec![root];
    while let Some(parent) = frontier.pop() {
        let children = toasty::query!(Thread FILTER .own_by_id == #parent)
            .exec(&mut db.clone())
            .await?;
        for child in children {
            if !to_delete.contains(&child.id) {
                to_delete.push(child.id);
                frontier.push(child.id);
            }
        }
    }

    // Refuse if any thread in the set is mid-generation.
    for id in &to_delete {
        if generate_states.contains_key(id) {
            return Err(ApiError::ThreadGenerating);
        }
    }

    // Abort any in-flight subthread and subagent background tasks owned by
    // each thread in the set, then evict the thread itself from the caches.
    // Iterate in reverse (children first) so a subthread's own sub-subthread
    // tasks are aborted before the subthread itself. Subagent tasks are
    // purely in-memory (no DB cascade), so abort_all_and_clear is the full
    // cleanup.
    for id in to_delete.iter().rev() {
        abort_subthread_tasks(active_threads, *id).await;
        abort_subagent_tasks(active_threads, *id).await;
        active_threads.remove(id);
        generate_states.remove(id);
    }

    let mut db = db.clone();
    let mut transaction = db.transaction().await?;
    for id in &to_delete {
        let turns = toasty::query!(Turn FILTER .thread_id == #id)
            .exec(&mut transaction)
            .await?;
        for turn in turns {
            toasty::query!(Message FILTER .turn_id == #(turn.id))
                .delete()
                .exec(&mut transaction)
                .await?;
        }
        toasty::query!(Turn FILTER .thread_id == #id)
            .delete()
            .exec(&mut transaction)
            .await?;
        toasty::query!(Middleware FILTER .thread_id == #id)
            .delete()
            .exec(&mut transaction)
            .await?;
        toasty::query!(Thread FILTER .id == #id)
            .delete()
            .exec(&mut transaction)
            .await?;
    }
    transaction.commit().await?;

    Ok(())
}

/// Read a thread's per-parent `SubthreadRegistry` from its activated `Agent`'s
/// extensions (if any) and abort every in-flight subthread background task it
/// owns. No-op when the thread isn't activated (it then has no in-flight
/// subthread tasks to abort).
async fn abort_subthread_tasks(
    active_threads: &dashmap::DashMap<u64, Arc<tokio::sync::RwLock<nekocode_core::agent::Agent>>>,
    thread_id: u64,
) {
    let registry: Option<Arc<nekocode_subthread::SubthreadRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent
                .extensions
                .get(nekocode_subthread::middleware::SUBTHREAD_EXTENSION_KEY)
                .and_then(|b| {
                    b.downcast_ref::<Arc<nekocode_subthread::SubthreadRegistry>>()
                        .cloned()
                })
        } else {
            None
        };

    if let Some(registry) = registry {
        // The thread is being deleted; abort every in-flight subthread task
        // it owns and clear the registry. The registry itself will be dropped
        // with the agent, but `abort_all_and_clear` releases task handles
        // promptly so the runtime can reclaim them before we touch the DB.
        let _aborted = registry.abort_all_and_clear();
    }
}

/// Read a thread's per-parent `SubagentRegistry` from its activated `Agent`'s
/// extensions (if any) and abort every in-flight subagent background task it
/// owns. No-op when the thread isn't activated. No DB cascade — subagent
/// state is purely in-memory.
async fn abort_subagent_tasks(
    active_threads: &dashmap::DashMap<u64, Arc<tokio::sync::RwLock<nekocode_core::agent::Agent>>>,
    thread_id: u64,
) {
    let registry: Option<Arc<nekocode_subagent::SubagentRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent
                .extensions
                .get(nekocode_subagent::SUBAGENT_EXTENSION_KEY)
                .and_then(|b| {
                    b.downcast_ref::<Arc<nekocode_subagent::SubagentRegistry>>()
                        .cloned()
                })
        } else {
            None
        };

    if let Some(registry) = registry {
        let _aborted = registry.abort_all_and_clear();
    }
}
