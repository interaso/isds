//! Downloads a message automatically and displays its metadata and delivery events.

use std::error::Error;
use std::{env, io};

use isds::MessageId;

mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let raw_id = env::args().nth(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: download_message <message-id>",
        )
    })?;
    let id = MessageId::new(raw_id)?;
    let client = common::client()?;

    // Automatically selects ordinary or large-volume download.
    let message = client.download_message(&id, None).await?;
    println!(
        "message {}: state={:?}, large={}, attachments={}",
        message.envelope.id,
        message.state,
        message.is_large_message.unwrap_or(false),
        message.attachments.len()
    );
    for attachment in &message.attachments {
        println!(
            "attachment {}: type={}, role={}, bytes={}",
            attachment.description(),
            attachment.mime_type(),
            attachment.meta_type().as_str(),
            attachment.content().len()
        );
    }

    let delivery = client.delivery_info(&id).await?;
    println!("delivery state: {:?}", delivery.state);
    for event in delivery.events {
        println!("event at {:?}: {:?}", event.time, event.description);
    }

    // Persist the message durably and idempotently in your application first.
    // After that transaction succeeds, mark it as read:
    // client.mark_message_downloaded(&id).await?;

    Ok(())
}
