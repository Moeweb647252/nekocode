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
use nekocode_entities::{
    message::Message as EntityMessage, thread::Thread, turn::Turn as EntityTurn,
};
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
            .exec(&mut db)
            .await
            .context("query sliced turns")?
    } else {
        query!(EntityTurn FILTER .thread_id == #thread_id ORDER BY .created_at ASC)
            .exec(&mut db)
            .await
            .context("query turns")?
    };

    let mut turns = Vec::with_capacity(entity_turns.len());
    for et in entity_turns {
        let entity_messages =
            query!(EntityMessage FILTER .turn_id == #(et.id) ORDER BY .message_index ASC)
                .exec(&mut db)
                .await
                .context("query ordered messages")?;
        let messages = entity_messages
            .into_iter()
            .map(|m| Message {
                created_at: m.created_at,
                data: m.content.0,
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
    let mut transaction = db.transaction().await.context("begin turn transaction")?;
    let turn_index = query!(EntityTurn FILTER .thread_id == #thread_id)
        .exec(&mut transaction)
        .await
        .context("count existing turns")?
        .len() as u64;

    let created_turn = toasty::create!(EntityTurn {
        thread_id,
        turn_index,
        usage: Json(turn.usage),
        finished: turn.finished,
    })
    .exec(&mut transaction)
    .await
    .context("create turn row")?;

    for (i, msg) in turn.messages.into_iter().enumerate() {
        toasty::create!(EntityMessage {
            turn_id: created_turn.id,
            message_index: i as u64,
            content: Json(msg.data),
            usage: msg.usage.map(Json),
        })
        .exec(&mut transaction)
        .await
        .context(format!("create message row {i}"))?;
    }
    transaction
        .commit()
        .await
        .context("commit turn transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekocode_types::generate::{MessageContent, MessageType, Usage};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn persistence_roundtrip_preserves_message_order() {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nekocode_turn_io_{}_{}.db", std::process::id(), n));
        let mut db = nekocode_entities::prepare_db(path).await.unwrap();
        let thread = toasty::create!(Thread {
            working_directory: std::env::temp_dir().to_string_lossy().into_owned(),
            model: "test".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        let message = |content: &str| Message {
            created_at: jiff::Timestamp::now(),
            data: MessageType::User(vec![MessageContent::Text {
                content: content.to_string(),
            }]),
            usage: None,
        };
        persist_turn(
            &db,
            thread.id,
            Turn {
                messages: vec![message("first"), message("second"), message("third")],
                usage: Usage::default(),
                finished: true,
            },
        )
        .await
        .unwrap();

        let loaded = load_turn_context(&db, thread.id).await.unwrap();
        let contents: Vec<_> = loaded[0]
            .messages
            .iter()
            .map(|message| match &message.data {
                MessageType::User(blocks) => match &blocks[0] {
                    MessageContent::Text { content } => content.as_str(),
                },
                _ => panic!("expected user message"),
            })
            .collect();
        assert_eq!(contents, ["first", "second", "third"]);
    }
}
