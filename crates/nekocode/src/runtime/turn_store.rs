//! Turn history and persistence owned by the runtime rather than a transport
//! adapter. The core agent remains storage-free.

use anyhow::Context as _;
use nekocode_entities::{
    message::Message as EntityMessage, thread::Thread, turn::Turn as EntityTurn,
};
use nekocode_types::generate::{Message, Turn};
use toasty::{Json, query};

pub(crate) async fn load_turn_context(
    db: &toasty::Db,
    thread_id: u64,
) -> anyhow::Result<Vec<Turn>> {
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
    for entity_turn in entity_turns {
        let messages =
            query!(EntityMessage FILTER .turn_id == #(entity_turn.id) ORDER BY .message_index ASC)
                .exec(&mut db)
                .await
                .context("query ordered messages")?
                .into_iter()
                .map(|message| Message {
                    created_at: message.created_at,
                    data: message.content.0,
                    usage: None,
                })
                .collect();
        turns.push(Turn {
            messages,
            usage: entity_turn.usage.0.clone(),
            finished: entity_turn.finished,
        });
    }
    Ok(turns)
}

pub(crate) async fn persist_turn(
    db: &toasty::Db,
    thread_id: u64,
    turn: Turn,
) -> anyhow::Result<()> {
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
    for (index, message) in turn.messages.into_iter().enumerate() {
        toasty::create!(EntityMessage {
            turn_id: created_turn.id,
            message_index: index as u64,
            content: Json(message.data),
            usage: message.usage.map(Json),
        })
        .exec(&mut transaction)
        .await
        .context(format!("create message row {index}"))?;
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

    async fn test_db() -> (toasty::Db, u64) {
        let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nekocode_runtime_turn_store_{}_{}.db",
            std::process::id(),
            sequence
        ));
        let mut db = nekocode_entities::prepare_db(path).await.unwrap();
        let thread = toasty::create!(Thread {
            working_directory: "/tmp".to_string(),
            model: "test".to_string(),
        })
        .exec(&mut db)
        .await
        .unwrap();
        (db, thread.id)
    }

    fn message(content: &str) -> Message {
        Message {
            created_at: jiff::Timestamp::now(),
            data: MessageType::User(vec![MessageContent::Text {
                content: content.to_string(),
            }]),
            usage: None,
        }
    }

    #[tokio::test]
    async fn persists_complete_and_partial_turn_flags() {
        let (db, thread_id) = test_db().await;
        persist_turn(
            &db,
            thread_id,
            Turn {
                messages: vec![message("complete")],
                usage: Usage::default(),
                finished: true,
            },
        )
        .await
        .unwrap();
        persist_turn(
            &db,
            thread_id,
            Turn {
                messages: vec![message("partial")],
                usage: Usage::default(),
                finished: false,
            },
        )
        .await
        .unwrap();
        let turns = load_turn_context(&db, thread_id).await.unwrap();
        assert_eq!(turns.len(), 2);
        assert!(turns[0].finished);
        assert!(!turns[1].finished);
    }
}
