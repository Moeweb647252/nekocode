//! DB I/O for agent turns, kept in the API layer so the core agent
//! (`Agent::run_loop`) stays free of storage concerns.
//!
//! [`load_turn_context`] reads a thread's history as in-memory
//! [`nekocode_types::generate::Turn`]s (flattening messages, preserving the
//! `generate_start_turn_id` context-compaction slice), which the agent consumes
//! as immutable working history. [`persist_turn`] writes a turn produced by the
//! agent back to the DB — a `Turn` row plus one `Message` row per message, with
//! per-message usage restored from the in-memory `Message.usage`.

use anyhow::Context as _;
use nekocode_entities::{message::Message as EntityMessage, thread::Thread, turn::Turn as EntityTurn};
use nekocode_types::generate::{Message, Turn};
use toasty::{Json, query};

/// Load a thread's turn history (with messages) as in-memory [`Turn`]s.
///
/// Mirrors the history-loading the old `run_loop` did inline: if the thread has
/// a `generate_start_turn_id` (a context-compaction anchor, currently written
/// nowhere but reserved for that feature), history is sliced to turns with
/// `id >=` that anchor; otherwise the full thread history is loaded.
pub async fn load_turn_context(db: &toasty::Db, thread_id: u64) -> anyhow::Result<Vec<Turn>> {
    let mut db = db.clone();
    let thread = query!(Thread FILTER .id == #thread_id)
        .first()
        .exec(&mut db)
        .await
        .context("query thread for turn context")?
        .context(format!("Thread not found: {thread_id}"))?;

    let entity_turns = if let Some(start_turn_id) = thread.generate_start_turn_id {
        query!(EntityTurn FILTER .id >= #start_turn_id AND .thread_id == #thread_id ORDER BY .created_at ASC)
            .include(EntityTurn::fields().messages())
            .exec(&mut db)
            .await
            .context("query sliced turns")?
    } else {
        query!(EntityTurn FILTER .thread_id == #thread_id ORDER BY .created_at ASC)
            .include(EntityTurn::fields().messages())
            .exec(&mut db)
            .await
            .context("query turns")?
    };

    let mut turns = Vec::with_capacity(entity_turns.len());
    for et in entity_turns {
        let messages = et
            .messages
            .get()
            .iter()
            .map(|m| Message {
                created_at: m.created_at,
                data: m.content.0.clone(),
                // Per-message usage isn't needed by the generation loop; only
                // assistant messages written by `persist_turn` carry it.
                usage: None,
            })
            .collect();
        turns.push(Turn {
            messages,
            usage: et.usage.0.clone(),
            finished: et.finished,
        });
    }
    Ok(turns)
}

/// Persist a turn produced by the agent: one `Turn` row and one `Message` row
/// per message. `turn_index` is derived from the current turn count for the
/// thread. Per-message `usage` (assistant messages) is written to the
/// `Message.usage` column.
pub async fn persist_turn(db: &toasty::Db, thread_id: u64, turn: Turn) -> anyhow::Result<()> {
    let mut db = db.clone();
    let turn_index = query!(EntityTurn FILTER .thread_id == #thread_id)
        .exec(&mut db)
        .await
        .context("count existing turns")?
        .len() as u64;

    let created_turn = toasty::create!(EntityTurn {
        thread_id,
        turn_index,
        usage: Json(turn.usage),
        finished: turn.finished,
    })
    .exec(&mut db)
    .await
    .context("create turn row")?;

    for (i, msg) in turn.messages.into_iter().enumerate() {
        toasty::create!(EntityMessage {
            turn_id: created_turn.id,
            message_index: i as u64,
            content: Json(msg.data),
            usage: msg.usage.map(Json),
        })
        .exec(&mut db)
        .await
        .context(format!("create message row {i}"))?;
    }
    Ok(())
}
