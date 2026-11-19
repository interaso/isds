//! Sends one buffered attachment, automatically using large-message transport when needed.

use std::error::Error;
use std::path::PathBuf;
use std::{env, io};

use bytes::Bytes;
use isds::{Attachment, DataBoxId, OutgoingMessage};

mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let recipient = arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: send_message <recipient-id> <file> [mime-type] [sender-reference]",
        )
    })?;
    let path = PathBuf::from(arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "an attachment file is required",
        )
    })?);
    let mime_type = arguments
        .next()
        .unwrap_or_else(|| "application/octet-stream".to_owned())
        .parse()?;
    let sender_reference = arguments.next();

    let content = tokio::fs::read(&path).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("the attachment filename is not Unicode"))?
        .to_owned();

    let recipient = DataBoxId::new(recipient)?;
    let mut message = OutgoingMessage::new(recipient, format!("Example document: {file_name}"))
        .with_attachment(Attachment::main(file_name, mime_type, Bytes::from(content)));
    message.metadata.sender_reference_number = sender_reference;

    let client = common::client()?;
    // The same call automatically handles ordinary and large-volume messages.
    // Acceptance is not delivery; use delivery_info or sent-message lists later.
    let sent = client.send_message(&message, None).await?;
    println!("accepted message {}", sent.id);
    Ok(())
}
