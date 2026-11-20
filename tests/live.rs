//! Ignored integration tests for the official ISDS test environment.
//!
//! These tests require `ISDS_RUN_LIVE_TESTS=1` and environment-only credentials.

use std::io;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use isds::{
    Attachment, Client, Credentials, CreditQuery, DataBoxId, Environment, Error, Message,
    MessageId, MessageListQuery, MessageMode, MessageRecord, MessageState, OutgoingMessage,
    ServiceErrorKind,
};
use jiff::{Span, Timestamp};
use uuid::Uuid;

type LiveResult = Result<(), Box<dyn std::error::Error>>;

struct Identity {
    client: Client,
    box_id: DataBoxId,
}

impl Identity {
    fn load(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let username = required_env(&format!("{prefix}_USERNAME"))?;
        let password = required_env(&format!("{prefix}_PASSWORD"))?;
        let box_id = required_env(&format!("{prefix}_BOX_ID"))?.parse()?;
        let client = Client::builder(Environment::Test)
            .credentials(Credentials::new(username, password))
            .timeout(Duration::from_secs(300))
            .build()?;
        Ok(Self { client, box_id })
    }
}

fn required_env(name: &str) -> Result<String, io::Error> {
    std::env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("required live-test variable {name} is missing or not Unicode"),
        )
    })
}

fn enabled() -> Result<bool, io::Error> {
    match std::env::var("ISDS_RUN_LIVE_TESTS") {
        Ok(value) if value == "1" => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ISDS_RUN_LIVE_TESTS must be exactly 1 when set",
        )),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(std::env::VarError::NotUnicode(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ISDS_RUN_LIVE_TESTS is not Unicode",
        )),
    }
}

fn identities() -> Result<[Identity; 3], Box<dyn std::error::Error>> {
    Ok([
        Identity::load("ISDS_TEST_SENDER")?,
        Identity::load("ISDS_TEST_RECIPIENT1")?,
        Identity::load("ISDS_TEST_RECIPIENT2")?,
    ])
}

#[tokio::test]
#[ignore = "requires explicit ISDS test-environment opt-in and credentials"]
async fn authenticates_and_queries_all_test_accounts() -> LiveResult {
    if !enabled()? {
        eprintln!(
            "skipping live ISDS test: set ISDS_RUN_LIVE_TESTS=1 and the nine credentials variables"
        );
        return Ok(());
    }
    for identity in &identities()? {
        let owner = identity.client.owner_info().await?;
        assert_eq!(owner.id.as_ref(), Some(&identity.box_id));
        assert!(
            identity
                .client
                .check_data_box(&identity.box_id)
                .await?
                .is_active()
        );
        let found = identity.client.find_data_box(&identity.box_id).await?;
        assert_eq!(
            found.and_then(|data_box| data_box.id),
            Some(identity.box_id.clone())
        );
        let credit = identity
            .client
            .credit_info(&CreditQuery::current(identity.box_id.clone()))
            .await?;
        assert!(credit.events.is_empty());

        let today = jiff::Zoned::now().date();
        let history_start = today.checked_sub(Span::new().days(7))?;
        identity
            .client
            .credit_info(&CreditQuery::history(
                identity.box_id.clone(),
                history_start,
                today,
            )?)
            .await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "sends messages; requires explicit ISDS test-environment opt-in"]
async fn sends_downloads_and_marks_messages_for_both_recipients() -> LiveResult {
    if !enabled()? {
        eprintln!(
            "skipping live ISDS lifecycle: set ISDS_RUN_LIVE_TESTS=1 and the nine credentials variables"
        );
        return Ok(());
    }
    let [sender, recipient1, recipient2] = identities()?;
    let started = Timestamp::now().checked_sub(Span::new().minutes(5))?;

    for (index, recipient) in [&recipient1, &recipient2].into_iter().enumerate() {
        let mode = if index == 0 {
            None
        } else {
            Some(MessageMode::Standard)
        };
        let correlation = Uuid::new_v4();
        let annotation = format!("ISDS Rust synthetic test {correlation} — žluťoučký");
        let main_xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><message xmlns=\"urn:isds-rs:test\"><id>{correlation}</id><text>Synthetic test — žluťoučký</text></message>"
        );
        let png = BASE64.decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
        )?;
        let outgoing = OutgoingMessage::new(recipient.box_id.clone(), annotation.clone())
            .with_attachment(Attachment::main(
                "synthetic.xml",
                "application/xml".parse()?,
                Bytes::from(main_xml),
            ))
            .with_attachment(Attachment::new(
                "synthetic.png",
                mime::IMAGE_PNG,
                Bytes::from(png.clone()),
            ));
        let sent = sender.client.send_message(&outgoing, mode).await?;
        wait_for_sent(&sender, &sent.id, started).await?;

        let downloaded = download_when_ready(recipient, &sent.id, started, mode).await?;
        assert_eq!(downloaded.envelope.id, sent.id);
        assert_eq!(
            downloaded.envelope.annotation.as_deref(),
            Some(annotation.as_str())
        );
        assert_eq!(
            downloaded.envelope.recipient_id.as_ref(),
            Some(&recipient.box_id)
        );
        assert_eq!(downloaded.attachments.len(), 2);
        assert_eq!(
            downloaded.attachments[0].content(),
            outgoing.attachments[0].content()
        );
        assert_eq!(
            downloaded.attachments[1].content(),
            outgoing.attachments[1].content()
        );

        let delivery = recipient.client.delivery_info(&sent.id).await?;
        assert_eq!(delivery.envelope.id, sent.id);
        let envelope = recipient.client.download_message_envelope(&sent.id).await?;
        assert_eq!(envelope.envelope.id, sent.id);
        assert_ne!(envelope.mode, Some(MessageMode::Large));
        recipient.client.mark_message_downloaded(&sent.id).await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "uploads a message over 20 MiB; requires explicit ISDS test-environment opt-in"]
async fn automatically_sends_and_downloads_a_large_message() -> LiveResult {
    if !enabled()? {
        eprintln!(
            "skipping live large-message test: set ISDS_RUN_LIVE_TESTS=1 and the credentials variables"
        );
        return Ok(());
    }
    let [sender, recipient, _] = identities()?;
    let started = Timestamp::now().checked_sub(Span::new().minutes(5))?;
    let correlation = Uuid::new_v4();
    let expected_content = Bytes::from(vec![b'x'; 20 * 1024 * 1024 + 1]);
    let outgoing = OutgoingMessage::new(
        recipient.box_id.clone(),
        format!("ISDS Rust automatic large-message test {correlation}"),
    )
    .with_attachment(Attachment::main(
        "large.txt",
        mime::TEXT_PLAIN,
        expected_content.clone(),
    ));

    let sent = sender.client.send_message(&outgoing, None).await?;
    let sent_record = wait_for_sent(&sender, &sent.id, started).await?;
    assert_eq!(sent_record.mode, Some(MessageMode::Large));

    let downloaded = download_when_ready(&recipient, &sent.id, started, None).await?;
    assert_eq!(downloaded.mode, Some(MessageMode::Large));
    assert_eq!(downloaded.attachment_count, Some(1));
    assert_eq!(downloaded.attachments.len(), 1);
    assert_eq!(downloaded.attachments[0].content(), &expected_content);

    let envelope = recipient.client.download_message_envelope(&sent.id).await?;
    assert_eq!(envelope.envelope.id, sent.id);
    assert_eq!(envelope.mode, Some(MessageMode::Large));
    assert_eq!(envelope.attachment_count, Some(1));
    let delivery = recipient.client.delivery_info(&sent.id).await?;
    assert_eq!(delivery.envelope.id, sent.id);
    recipient.client.mark_message_downloaded(&sent.id).await?;
    Ok(())
}

async fn wait_for_sent(
    sender: &Identity,
    id: &MessageId,
    started: Timestamp,
) -> Result<MessageRecord, Box<dyn std::error::Error>> {
    let query = MessageListQuery {
        from: Some(started),
        limit: 100,
        ..MessageListQuery::default()
    };
    for _ in 0..15 {
        let page = sender.client.list_sent_messages(&query).await?;
        if let Some(record) = page.records.iter().find(|record| &record.envelope.id == id) {
            return Ok(record.clone());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "sent message did not appear in the sender's list",
    )
    .into())
}

async fn download_when_ready(
    recipient: &Identity,
    id: &MessageId,
    started: Timestamp,
    mode: Option<MessageMode>,
) -> Result<Message, Box<dyn std::error::Error>> {
    let query = MessageListQuery {
        from: Some(started),
        limit: 100,
        ..MessageListQuery::default()
    };
    for _ in 0..30 {
        // This intentionally runs only in the test environment. Listing received
        // messages performs ISDS delivery-by-login for accessible messages.
        match recipient.client.list_received_messages(&query).await {
            Ok(page) => {
                let Some(record) = page.records.iter().find(|record| &record.envelope.id == id)
                else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                };
                if matches!(
                    record.state,
                    MessageState::PlacedInRecipientBox | MessageState::DeliveredByFiction
                ) {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let result = recipient.client.download_message(id, mode).await;
                match result {
                    Ok(message) => return Ok(message),
                    Err(Error::Service(service))
                        if service.kind() == ServiceErrorKind::MessageNotDownloadable => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(Error::Service(service))
                if service.kind() == ServiceErrorKind::DeliveryInProgress => {}
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "received message did not become downloadable",
    )
    .into())
}
