use std::collections::HashSet;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use jiff::Timestamp;
use mime::Mime;
use quick_xml::events::Event;
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
        sender: optional_owned(node, "dmSender")?,
        sender_address: optional_owned(node, "dmSenderAddress")?,
        sender_type: soap::parse(
            &soap::required_text(node, ISDS_NS, "dmSenderType")?,
            "dmSenderType",
        )?,
        recipient: optional_owned(node, "dmRecipient")?,
        recipient_address: optional_owned(node, "dmRecipientAddress")?,
        ambiguous_recipient: optional_bool(node, "dmAmbiguousRecipient")?,
        recipient_id: optional_id(node, "dbIDRecipient")?,
        annotation: optional_owned(node, "dmAnnotation")?,
        metadata: MessageMetadata {
            sender_org_unit: optional_owned(node, "dmSenderOrgUnit")?,
            sender_org_unit_number: optional_parsed(node, "dmSenderOrgUnitNum")?,
            recipient_org_unit: optional_owned(node, "dmRecipientOrgUnit")?,
            recipient_org_unit_number: optional_parsed(node, "dmRecipientOrgUnitNum")?,
            to_hands: optional_owned(node, "dmToHands")?,
            recipient_reference_number: optional_owned(node, "dmRecipientRefNumber")?,
            sender_reference_number: optional_owned(node, "dmSenderRefNumber")?,
            recipient_identification: optional_owned(node, "dmRecipientIdent")?,
            sender_identification: optional_owned(node, "dmSenderIdent")?,
            legal_title_law: optional_parsed(node, "dmLegalTitleLaw")?,
            legal_title_year: optional_parsed(node, "dmLegalTitleYear")?,
            legal_title_section: optional_owned(node, "dmLegalTitleSect")?,
            legal_title_paragraph: optional_owned(node, "dmLegalTitlePar")?,
            legal_title_point: optional_owned(node, "dmLegalTitlePoint")?,
            personal_delivery: optional_bool(node, "dmPersonalDelivery")?,
            allow_substitute_delivery: optional_bool(node, "dmAllowSubstDelivery")?,
        },
    })
}

pub(crate) fn parse_attachments(node: Node<'_, '_>) -> Result<Vec<Attachment>> {
    let files = soap::child(node, ISDS_NS, "dmFiles")?
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
    let raw_meta_type = required_attribute(node, "dmFileMetaType")?;
    if raw_meta_type.is_empty() {
        return Err(Error::invalid_response("dmFileMetaType must not be empty"));
    }
    let meta_type = AttachmentType::from_raw(raw_meta_type);

    let encoded = soap::child(node, ISDS_NS, "dmEncodedContent")?;
    let xml = soap::child(node, ISDS_NS, "dmXMLContent")?;
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
                decode_base64(&content, "dmEncodedContent")?,
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
            Attachment::xml(description, mime_type, standalone_xml(root)?)
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

fn standalone_xml(root: Node<'_, '_>) -> Result<String> {
    let input = root.document().input_text();
    let source = input
        .get(root.range())
        .ok_or_else(|| Error::invalid_response("dmXMLContent source range is invalid"))?;
    let invalid = || Error::invalid_response("could not extract dmXMLContent");
    let mut reader = quick_xml::Reader::from_str(source);
    let mut writer = quick_xml::Writer::new(Vec::with_capacity(source.len()));
    let mut root_pending = true;
    loop {
        let event = reader.read_event().map_err(|_| invalid())?;
        match event {
            Event::Eof => break,
            Event::Start(mut element) if root_pending => {
                let additions = inherited_namespaces(root, &element)?;
                for (name, uri) in &additions {
                    element.push_attribute((name.as_str(), uri.as_str()));
                }
                writer
                    .write_event(Event::Start(element))
                    .map_err(|_| invalid())?;
                root_pending = false;
            }
            Event::Empty(mut element) if root_pending => {
                let additions = inherited_namespaces(root, &element)?;
                for (name, uri) in &additions {
                    element.push_attribute((name.as_str(), uri.as_str()));
                }
                writer
                    .write_event(Event::Empty(element))
                    .map_err(|_| invalid())?;
                root_pending = false;
            }
            event => writer.write_event(event).map_err(|_| invalid())?,
        }
    }
    if root_pending {
        return Err(invalid());
    }
    String::from_utf8(writer.into_inner()).map_err(|_| invalid())
}

fn inherited_namespaces(
    root: Node<'_, '_>,
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<(String, String)>> {
    let invalid = || Error::invalid_response("could not extract dmXMLContent");
    let mut declared = HashSet::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|_| invalid())?;
        let name = attribute.key.as_ref();
        if name == "xmlns" || name.starts_with("xmlns:") {
            declared.insert(name.to_owned());
        }
    }

    // `roxmltree` exposes the effective in-scope set, nearest binding first.
    // Add bindings absent from the root tag itself, without redeclaring a prefix.
    let mut additions = Vec::new();
    for namespace in root.namespaces() {
        let prefix = namespace.name();
        if prefix == Some("xml") {
            continue;
        }
        let name = prefix.map_or_else(|| "xmlns".to_owned(), |prefix| format!("xmlns:{prefix}"));
        if declared.insert(name.clone()) {
            additions.push((name, namespace.uri().to_owned()));
        }
    }
    Ok(additions)
}

pub(crate) fn parse_hash(node: Node<'_, '_>) -> Result<MessageHash> {
    let hash = soap::child(node, ISDS_NS, "dmHash")?
        .ok_or_else(|| Error::invalid_response("dmHash is missing"))?;
    let content = soap::required_text_or_empty(node, ISDS_NS, "dmHash")?;
    Ok(MessageHash {
        bytes: decode_base64(&content, "dmHash")?,
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
        &soap::required_text(node, ISDS_NS, "dmMessageStatus")?,
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
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| soap::parse_bool(&value, name))
        .transpose()
}

fn optional_id(node: Node<'_, '_>, name: &str) -> Result<Option<DataBoxId>> {
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| {
            DataBoxId::new(value)
                .map_err(|_| Error::invalid_response(format!("{name} is not a valid databox ID")))
        })
        .transpose()
}

fn optional_owned(node: Node<'_, '_>, name: &str) -> Result<Option<String>> {
    soap::optional_text(node, ISDS_NS, name)
}

fn optional_parsed<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<Option<T>> {
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| soap::parse(&value, name))
        .transpose()
}

fn required_attribute<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str> {
    node.attribute(name)
        .ok_or_else(|| Error::invalid_response(format!("dmFile is missing attribute {name}")))
}

#[cfg(test)]
mod tests {
    use roxmltree::Document;

    use super::*;

    fn parse_xml_attachment(input: &str) -> Attachment {
        let document = Document::parse(input).unwrap();
        let file = document
            .descendants()
            .find(|node| node.has_tag_name((ISDS_NS, "dmFile")))
            .unwrap();
        parse_attachment(file).unwrap()
    }

    #[test]
    fn empty_attachment_type_is_rejected() {
        let document = Document::parse(
            r#"<root xmlns:i="http://isds.czechpoint.cz/v20">
              <i:dmFile dmMimeType="application/octet-stream" dmFileMetaType="" dmFileDescr="empty.bin">
                <i:dmEncodedContent>AA==</i:dmEncodedContent>
              </i:dmFile></root>"#,
        )
        .unwrap();
        let file = document
            .descendants()
            .find(|node| node.has_tag_name((ISDS_NS, "dmFile")))
            .unwrap();
        assert!(matches!(
            parse_attachment(file),
            Err(Error::InvalidResponse { .. })
        ));
    }

    #[test]
    fn inline_xml_preserves_inherited_prefix_default_and_attribute_namespaces() {
        let attachment = parse_xml_attachment(
            r#"<root xmlns="urn:default" xmlns:i="http://isds.czechpoint.cz/v20" xmlns:p="urn:prefix">
              <i:dmFile dmMimeType="application/xml" dmFileMetaType="enclosure" dmFileDescr="inline.xml">
                <i:dmXMLContent><note p:kind="inherited"><p:child>value</p:child></note></i:dmXMLContent>
              </i:dmFile></root>"#,
        );
        let xml = attachment.xml_content().unwrap();
        let standalone = Document::parse(xml).unwrap();
        let root = standalone.root_element();
        assert_eq!(root.tag_name().namespace(), Some("urn:default"));
        assert_eq!(root.attribute(("urn:prefix", "kind")), Some("inherited"));
        assert_eq!(
            root.first_element_child().unwrap().tag_name().namespace(),
            Some("urn:prefix")
        );
        assert_eq!(xml.matches("xmlns:p=").count(), 1);
        assert_eq!(xml.matches("xmlns=\"urn:default\"").count(), 1);
    }

    #[test]
    fn inline_xml_does_not_duplicate_or_override_root_declarations() {
        let attachment = parse_xml_attachment(
            r#"<root xmlns="urn:outer-default" xmlns:p="urn:outer-prefix" xmlns:i="http://isds.czechpoint.cz/v20">
              <i:dmFile dmMimeType="application/xml" dmFileMetaType="enclosure" dmFileDescr="inline.xml">
                <i:dmXMLContent><p:note xmlns="urn:inner-default" xmlns:p="urn:inner-prefix"/></i:dmXMLContent>
              </i:dmFile></root>"#,
        );
        let xml = attachment.xml_content().unwrap();
        let standalone = Document::parse(xml).unwrap();
        let root = standalone.root_element();
        assert_eq!(root.tag_name().namespace(), Some("urn:inner-prefix"));
        assert_eq!(root.default_namespace(), Some("urn:inner-default"));
        assert_eq!(xml.matches("xmlns:p=").count(), 1);
        assert_eq!(xml.matches("xmlns=\"urn:inner-default\"").count(), 1);
        assert!(!xml.contains("urn:outer-prefix"));
        assert!(!xml.contains("urn:outer-default"));
    }
}
