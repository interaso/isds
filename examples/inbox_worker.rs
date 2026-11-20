//! Shows how to integrate received-message processing with durable application storage.

use std::error::Error;
use std::future::Future;

use isds::{Client, Error as IsdsError, Message, MessageListQuery, MessageState, ServiceErrorKind};

/// Processes one page for a caller that intentionally accepts delivery-by-login.
///
/// The persistence closure must commit durably and idempotently by message ID.
/// Call this function from your application's bounded scheduler with cancellation,
/// backoff, and jitter appropriate to that application.
pub async fn process_next_page<F, Fut>(
    client: &Client,
    mut persist: F,
) -> Result<usize, Box<dyn Error>>
where
    F: FnMut(Message) -> Fut,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
{
    // This operation can trigger legal delivery-by-login. The status filter keeps
    // already-read messages out of subsequent iterations.
    let page = client
        .list_received_messages(&MessageListQuery {
            states: Some(vec![MessageState::DeliveredByLogin]),
            limit: 100,
            ..MessageListQuery::default()
        })
        .await?;

    let mut processed = 0;
    for record in page.records {
        let message = match client.download_message(&record.envelope.id, None).await {
            Ok(message) => message,
            Err(IsdsError::Service(error))
                if error.kind() == ServiceErrorKind::MessageNotDownloadable =>
            {
                // Still being processed; do not repeat any message creation.
                continue;
            }
            Err(error) => return Err(error.into()),
        };

        let id = message.envelope.id.clone();
        persist(message).await?;
        client.mark_message_downloaded(&id).await?;
        processed += 1;
    }

    Ok(processed)
}

fn main() {
    println!(
        "This is an inbox-worker template. Integrate process_next_page with your durable store and scheduler."
    );
}

// A scheduler invokes the function with an application-owned persistence closure:
//
// let processed = process_next_page(&client, |message| async move {
//     database.save_idempotently(message).await
// }).await?;
