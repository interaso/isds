use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use jiff::Timestamp;
use mime::Mime;
use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{
    Attachment, AttachmentType, DataBoxId, MessageEnvelope, MessageHash, MessageId,
    MessageMetadata, MessageMode, MessageState,
};
use crate::wire::soap::{self, ISDS_NS};

pub(crate) fn parse_envelope(node: Node<'_, '_>) -> Result<MessageEnvelope> {
    let id = soap::required_text(node, ISDS_NS, "dmID")?;
    let id = MessageId::new(id)
        .map_err(|_| Error::invalid_response("dmID is empty or exceeds 20 characters"))?;
    Ok(MessageEnvelope {
        id,
        sender_id: optional_id(node, "dbIDSender")?,
        sender: optional_owned(node, "dmSender"),
        sender_address: optional_owned(node, "dmSenderAddress"),
        sender_type: soap::parse(
            soap::required_text(node, ISDS_NS, "dmSenderType")?,
            "dmSenderType",
        )?,
        recipient: optional_owned(node, "dmRecipient"),
        recipient_address: optional_owned(node, "dmRecipientAddress"),
        ambiguous_recipient: optional_bool(node, "dmAmbiguousRecipient")?,
        recipient_id: optional_id(node, "dbIDRecipient")?,
        annotation: optional_owned(node, "dmAnnotation"),
        metadata: MessageMetadata {
            sender_org_unit: optional_owned(node, "dmSenderOrgUnit"),
            sender_org_unit_number: optional_parsed(node, "dmSenderOrgUnitNum")?,
            recipient_org_unit: optional_owned(node, "dmRecipientOrgUnit"),
            recipient_org_unit_number: optional_parsed(node, "dmRecipientOrgUnitNum")?,
            to_hands: optional_owned(node, "dmToHands"),
            recipient_reference_number: optional_owned(node, "dmRecipientRefNumber"),
            sender_reference_number: optional_owned(node, "dmSenderRefNumber"),
            recipient_identification: optional_owned(node, "dmRecipientIdent"),
            sender_identification: optional_owned(node, "dmSenderIdent"),
            legal_title_law: optional_parsed(node, "dmLegalTitleLaw")?,
            legal_title_year: optional_parsed(node, "dmLegalTitleYear")?,
            legal_title_section: optional_owned(node, "dmLegalTitleSect"),
            legal_title_paragraph: optional_owned(node, "dmLegalTitlePar"),
            legal_title_point: optional_owned(node, "dmLegalTitlePoint"),
            personal_delivery: optional_bool(node, "dmPersonalDelivery")?,
            allow_substitute_delivery: optional_bool(node, "dmAllowSubstDelivery")?,
        },
    })
}

pub(crate) fn parse_attachments(node: Node<'_, '_>) -> Result<Vec<Attachment>> {
    let files = soap::child(node, ISDS_NS, "dmFiles")
        .ok_or_else(|| Error::invalid_response("dmFiles is missing"))?;
    let attachments = soap::children(files, ISDS_NS, "dmFile")
        .map(parse_attachment)
        .collect::<Result<Vec<_>>>()?;
    if attachments.is_empty() {
        return Err(Error::invalid_response("dmFiles contains no attachments"));
    }
    Ok(attachments)
}

fn parse_attachment(node: Node<'_, '_>) -> Result<Attachment> {
    let description = required_attribute(node, "dmFileDescr")?;
    let mime_type = required_attribute(node, "dmMimeType")?
        .parse::<Mime>()
        .map_err(|_| Error::invalid_response("dmFile has an invalid dmMimeType"))?;
    let meta_type = AttachmentType::from_raw(required_attribute(node, "dmFileMetaType")?);

    let encoded = soap::child(node, ISDS_NS, "dmEncodedContent");
    let xml = soap::child(node, ISDS_NS, "dmXMLContent");
    let mut attachment = match (encoded, xml) {
        (Some(_), Some(_)) => {
            return Err(Error::invalid_response(
                "dmFile contains both dmEncodedContent and dmXMLContent",
            ));
        }
        (Some(_), None) => {
            let content = soap::required_text_or_empty(node, ISDS_NS, "dmEncodedContent")?;
            Attachment::new(
                description,
                mime_type,
                decode_base64(content, "dmEncodedContent")?,
            )
        }
        (None, Some(xml)) => {
            let mut elements = xml.children().filter(|child| child.is_element());
            let root = elements.next().ok_or_else(|| {
                Error::invalid_response("dmXMLContent does not contain an XML element")
            })?;
            if elements.next().is_some() {
                return Err(Error::invalid_response(
                    "dmXMLContent contains multiple root elements",
                ));
            }
            let input = root.document().input_text();
            let source = input
                .get(root.range())
                .ok_or_else(|| Error::invalid_response("dmXMLContent source range is invalid"))?;
            Attachment::xml(description, mime_type, source)
        }
        (None, None) => {
            return Err(Error::invalid_response(
                "dmFile has neither dmEncodedContent nor dmXMLContent",
            ));
        }
    };
    attachment = attachment.with_meta_type(meta_type);

    if let Some(guid) = node.attribute("dmFileGuid") {
        attachment = attachment.with_guid(guid);
    }
    if let Some(guid) = node.attribute("dmUpFileGuid") {
        attachment = attachment.with_up_file_guid(guid);
    }
    if let Some(format) = node.attribute("dmFormat") {
        attachment = attachment.with_format(format);
    }
    Ok(attachment)
}

pub(crate) fn parse_hash(node: Node<'_, '_>) -> Result<MessageHash> {
    let hash = soap::child(node, ISDS_NS, "dmHash")
        .ok_or_else(|| Error::invalid_response("dmHash is missing"))?;
    let content = soap::required_text_or_empty(node, ISDS_NS, "dmHash")?;
    Ok(MessageHash {
        bytes: decode_base64(content, "dmHash")?,
        algorithm: hash.attribute("algorithm").map(str::to_owned),
    })
}

pub(crate) fn decode_base64(value: &str, name: &str) -> Result<Bytes> {
    let compact: String = value
        .chars()
        .filter(|character| !matches!(character, ' ' | '\r' | '\n' | '\t'))
        .collect();
    BASE64
        .decode(compact.as_bytes())
        .map(Bytes::from)
        .map_err(|_| Error::invalid_response(format!("{name} contains invalid base64")))
}

pub(crate) fn optional_timestamp(node: Node<'_, '_>, name: &str) -> Result<Option<Timestamp>> {
    optional_parsed(node, name)
}

pub(crate) fn optional_i64(node: Node<'_, '_>, name: &str) -> Result<Option<i64>> {
    optional_parsed(node, name)
}

pub(crate) fn required_state(node: Node<'_, '_>) -> Result<MessageState> {
    let code = soap::parse::<i32>(
        soap::required_text(node, ISDS_NS, "dmMessageStatus")?,
        "dmMessageStatus",
    )?;
    Ok(MessageState::from_code(code))
}

pub(crate) fn optional_message_mode(node: Node<'_, '_>) -> Result<Option<MessageMode>> {
    let large = node
        .attribute("dmVODZ")
        .map(|value| soap::parse_bool(value, "dmVODZ"))
        .transpose()?;
    Ok(large.map(|large| {
        if large {
            MessageMode::Large
        } else {
            MessageMode::Standard
        }
    }))
}

pub(crate) fn optional_bool(node: Node<'_, '_>, name: &str) -> Result<Option<bool>> {
    soap::optional_text(node, ISDS_NS, name)
        .map(|value| soap::parse_bool(value, name))
        .transpose()
}

fn optional_id(node: Node<'_, '_>, name: &str) -> Result<Option<DataBoxId>> {
    soap::optional_text(node, ISDS_NS, name)
        .map(|value| {
            DataBoxId::new(value)
                .map_err(|_| Error::invalid_response(format!("{name} is not a valid databox ID")))
        })
        .transpose()
}

fn optional_owned(node: Node<'_, '_>, name: &str) -> Option<String> {
    soap::optional_text(node, ISDS_NS, name).map(str::to_owned)
}

fn optional_parsed<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<Option<T>> {
    soap::optional_text(node, ISDS_NS, name)
        .map(|value| soap::parse(value, name))
        .transpose()
}

fn required_attribute<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str> {
    node.attribute(name)
        .ok_or_else(|| Error::invalid_response(format!("dmFile is missing attribute {name}")))
}
