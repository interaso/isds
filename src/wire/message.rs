use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{Attachment, MessageId, OutgoingMessage, SentMessage};
use crate::wire::soap::{self, ISDS_NS};

pub(crate) fn create_message_request(message: &OutgoingMessage) -> Result<Vec<u8>> {
    validate_outgoing(message)?;
    soap::request("CreateMessage", |writer| {
        write_envelope(writer, message)?;
        write_files(writer, message)
    })
}

pub(crate) fn decode_create_message(response: Node<'_, '_>) -> Result<SentMessage> {
    soap::status(response, "dm")?.into_result()?;
    let id = soap::required_text(response, ISDS_NS, "dmID")?;
    let id = MessageId::new(id)
        .map_err(|_| Error::invalid_response("dmID is empty or exceeds 20 characters"))?;
    Ok(SentMessage { id })
}

pub(crate) fn validate_outgoing(message: &OutgoingMessage) -> Result<()> {
    if message.attachments.is_empty() {
        return Err(Error::invalid_input(
            "attachments",
            "CreateMessage requires at least one attachment",
        ));
    }
    if message.attachments.len() > 100 {
        return Err(Error::invalid_input(
            "attachments",
            "a message cannot contain more than 100 attachments",
        ));
    }
    for attachment in &message.attachments {
        validate_attachment_type(attachment)?;
    }
    validate_length(&message.annotation, "annotation", 255)?;
    validate_optional_length(
        message.metadata.recipient_reference_number.as_deref(),
        "recipient_reference_number",
    )?;
    validate_optional_length(
        message.metadata.sender_reference_number.as_deref(),
        "sender_reference_number",
    )?;
    validate_optional_length(
        message.metadata.recipient_identification.as_deref(),
        "recipient_identification",
    )?;
    validate_optional_length(
        message.metadata.sender_identification.as_deref(),
        "sender_identification",
    )?;
    if message
        .message_type
        .as_deref()
        .is_some_and(|value| value.chars().count() != 1)
    {
        return Err(Error::invalid_input(
            "message_type",
            "dmType must contain exactly one character",
        ));
    }
    Ok(())
}

pub(crate) fn validate_attachment_type(attachment: &Attachment) -> Result<()> {
    if attachment.meta_type().as_str().is_empty() {
        return Err(Error::invalid_input(
            "attachments",
            "attachment dmFileMetaType must not be empty",
        ));
    }
    Ok(())
}

fn validate_length(value: &str, field: &'static str, maximum: usize) -> Result<()> {
    if value.chars().count() > maximum {
        return Err(Error::invalid_input(
            field,
            format!("value must not exceed {maximum} Unicode characters"),
        ));
    }
    Ok(())
}

fn validate_optional_length(value: Option<&str>, field: &'static str) -> Result<()> {
    if let Some(value) = value {
        validate_length(value, field, 50)?;
    }
    Ok(())
}

pub(crate) fn write_envelope(
    writer: &mut Writer<Vec<u8>>,
    message: &OutgoingMessage,
) -> soap::XmlResult {
    let mut envelope = BytesStart::new("dmEnvelope");
    if let Some(message_type) = &message.message_type {
        envelope.push_attribute(("dmType", message_type.as_str()));
    }
    writer.write_event(Event::Start(envelope))?;

    let metadata = &message.metadata;
    soap::nillable_text_element(
        writer,
        "dmSenderOrgUnit",
        metadata.sender_org_unit.as_deref(),
    )?;
    nillable_value(
        writer,
        "dmSenderOrgUnitNum",
        metadata.sender_org_unit_number,
    )?;
    soap::text_element(writer, "dbIDRecipient", message.recipient.as_str())?;
    soap::nillable_text_element(
        writer,
        "dmRecipientOrgUnit",
        metadata.recipient_org_unit.as_deref(),
    )?;
    nillable_value(
        writer,
        "dmRecipientOrgUnitNum",
        metadata.recipient_org_unit_number,
    )?;
    soap::nillable_text_element(writer, "dmToHands", metadata.to_hands.as_deref())?;
    soap::text_element(writer, "dmAnnotation", &message.annotation)?;
    soap::nillable_text_element(
        writer,
        "dmRecipientRefNumber",
        metadata.recipient_reference_number.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dmSenderRefNumber",
        metadata.sender_reference_number.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dmRecipientIdent",
        metadata.recipient_identification.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dmSenderIdent",
        metadata.sender_identification.as_deref(),
    )?;
    nillable_value(writer, "dmLegalTitleLaw", metadata.legal_title_law)?;
    nillable_value(writer, "dmLegalTitleYear", metadata.legal_title_year)?;
    soap::nillable_text_element(
        writer,
        "dmLegalTitleSect",
        metadata.legal_title_section.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dmLegalTitlePar",
        metadata.legal_title_paragraph.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dmLegalTitlePoint",
        metadata.legal_title_point.as_deref(),
    )?;
    nillable_value(writer, "dmPersonalDelivery", metadata.personal_delivery)?;
    nillable_value(
        writer,
        "dmAllowSubstDelivery",
        metadata.allow_substitute_delivery,
    )?;
    if let Some(public_authority_mode) = message.public_authority_mode {
        soap::text_element(writer, "dmOVM", soap::xml_bool(public_authority_mode))?;
    }
    if let Some(publish) = message.publish_own_id {
        let mut element = BytesStart::new("dmPublishOwnID");
        let level = publish.identification_level.map(|value| value.to_string());
        if let Some(level) = &level {
            element.push_attribute(("IdLevel", level.as_str()));
        }
        writer.write_event(Event::Start(element))?;
        writer.write_event(Event::Text(BytesText::new(soap::xml_bool(publish.enabled))))?;
        writer.write_event(Event::End(BytesEnd::new("dmPublishOwnID")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("dmEnvelope")))?;
    Ok(())
}

fn nillable_value<T: ToString>(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    value: Option<T>,
) -> soap::XmlResult {
    soap::nillable_text_element(
        writer,
        name,
        value.map(|value| value.to_string()).as_deref(),
    )
}

fn write_files(writer: &mut Writer<Vec<u8>>, message: &OutgoingMessage) -> soap::XmlResult {
    writer.write_event(Event::Start(BytesStart::new("dmFiles")))?;
    for attachment in &message.attachments {
        let mut file = BytesStart::new("dmFile");
        let mime_type = attachment.mime_type().to_string();
        file.push_attribute(("dmMimeType", mime_type.as_str()));
        file.push_attribute(("dmFileMetaType", attachment.meta_type().as_str()));
        file.push_attribute(("dmFileDescr", attachment.description()));
        if let Some(guid) = attachment.guid() {
            file.push_attribute(("dmFileGuid", guid));
        }
        if let Some(guid) = attachment.up_file_guid() {
            file.push_attribute(("dmUpFileGuid", guid));
        }
        if let Some(format) = attachment.format() {
            file.push_attribute(("dmFormat", format));
        }
        writer.write_event(Event::Start(file))?;
        soap::text_element(
            writer,
            "dmEncodedContent",
            &BASE64.encode(attachment.content()),
        )?;
        writer.write_event(Event::End(BytesEnd::new("dmFile")))?;
    }
    writer.write_event(Event::End(BytesEnd::new("dmFiles")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::model::{Attachment, AttachmentType, DataBoxId, MessageMetadata, PublishOwnId};

    use super::*;

    fn message() -> OutgoingMessage {
        OutgoingMessage::new(DataBoxId::new("n94k37v").unwrap(), "Příliš & <žluťoučký>")
            .with_attachment(
                Attachment::main(
                    "document & data.bin",
                    mime::APPLICATION_OCTET_STREAM,
                    Bytes::from_static(&[0, 1, 2, 253, 254, 255]),
                )
                .with_guid("guid-1")
                .with_up_file_guid("parent-1"),
            )
    }

    #[test]
    fn request_preserves_schema_order_and_escapes_text_and_attributes() {
        let mut message = message();
        message.metadata = MessageMetadata {
            sender_org_unit_number: Some(10),
            recipient_reference_number: Some("ref-1".to_owned()),
            personal_delivery: Some(true),
            allow_substitute_delivery: Some(false),
            ..MessageMetadata::default()
        };
        message.public_authority_mode = Some(true);
        message.publish_own_id = Some(PublishOwnId {
            enabled: true,
            identification_level: Some(3),
        });
        message.message_type = Some("O".to_owned());

        let xml = String::from_utf8(create_message_request(&message).unwrap()).unwrap();
        assert!(xml.contains("dmType=\"O\""));
        assert!(xml.contains("Příliš &amp; &lt;žluťoučký&gt;"));
        assert!(xml.contains("dmFileDescr=\"document &amp; data.bin\""));
        assert!(xml.contains("dmFileMetaType=\"main\""));
        assert!(xml.contains("dmFileGuid=\"guid-1\""));
        assert!(xml.contains("dmUpFileGuid=\"parent-1\""));
        assert!(xml.contains("dmPublishOwnID IdLevel=\"3\""));
        assert!(xml.contains("<dmEncodedContent>AAEC/f7/</dmEncodedContent>"));

        let recipient = xml.find("<dbIDRecipient").unwrap();
        let annotation = xml.find("<dmAnnotation").unwrap();
        let legal_title = xml.find("<dmLegalTitleLaw").unwrap();
        let personal = xml.find("<dmPersonalDelivery").unwrap();
        let public_authority_mode = xml.find("<dmOVM").unwrap();
        let files = xml.find("<dmFiles").unwrap();
        assert!(recipient < annotation && annotation < legal_title);
        assert!(
            legal_title < personal
                && personal < public_authority_mode
                && public_authority_mode < files
        );
    }

    #[test]
    fn validates_before_serializing() {
        let mut empty = message();
        empty.attachments.clear();
        assert!(matches!(
            create_message_request(&empty),
            Err(Error::InvalidInput {
                field: "attachments",
                ..
            })
        ));

        let mut long = message();
        long.annotation = "ř".repeat(256);
        assert!(matches!(
            create_message_request(&long),
            Err(Error::InvalidInput {
                field: "annotation",
                ..
            })
        ));

        let mut wrong_type = message();
        wrong_type.message_type = Some("too-long".to_owned());
        assert!(matches!(
            create_message_request(&wrong_type),
            Err(Error::InvalidInput {
                field: "message_type",
                ..
            })
        ));
    }

    #[test]
    fn empty_custom_attachment_type_is_rejected_but_future_codes_are_preserved() {
        let mut empty = message();
        empty.attachments[0] = empty.attachments[0]
            .clone()
            .with_meta_type(AttachmentType::Other(String::new()));
        assert!(matches!(
            create_message_request(&empty),
            Err(Error::InvalidInput {
                field: "attachments",
                ..
            })
        ));

        let mut future = message();
        future.attachments[0] = future.attachments[0]
            .clone()
            .with_meta_type(AttachmentType::Other("future".to_owned()));
        let xml = String::from_utf8(create_message_request(&future).unwrap()).unwrap();
        assert!(xml.contains("dmFileMetaType=\"future\""));
    }

    #[test]
    fn parses_an_opaque_message_id_with_leading_zeroes() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><CreateMessageResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmID>00000000000000000123</dmID>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </CreateMessageResponse></soap:Body></soap:Envelope>"#;
        let sent = soap::decode(xml, "CreateMessageResponse", decode_create_message).unwrap();
        assert_eq!(sent.id.as_str(), "00000000000000000123");
    }

    #[test]
    fn attachment_debug_does_not_expose_content() {
        let attachment = Attachment::new(
            "secret.bin",
            mime::APPLICATION_OCTET_STREAM,
            Bytes::from_static(b"sentinel-attachment-content"),
        )
        .with_meta_type(AttachmentType::Other("future".to_owned()));
        let rendered = format!("{attachment:?}");
        assert!(!rendered.contains("sentinel-attachment-content"));
        assert!(rendered.contains("27"));
        assert_eq!(attachment.meta_type().as_str(), "future");
    }
}
