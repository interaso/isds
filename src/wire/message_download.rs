use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{Message, MessageId};
use crate::wire::message_common;
use crate::wire::soap::{self, ISDS_NS};

pub(crate) fn download_request(id: &MessageId) -> Result<Vec<u8>> {
    soap::request("MessageDownload", |writer| {
        soap::text_element(writer, "dmID", id.as_str())
    })
}

pub(crate) fn decode_download(response: Node<'_, '_>) -> Result<Message> {
    if soap::child(response, ISDS_NS, "dmStatus").is_some() {
        soap::status(response, "dm")?.into_result()?;
    }
    let returned = soap::child(response, ISDS_NS, "dmReturnedMessage")
        .filter(|node| !soap::is_nil(*node))
        .ok_or_else(|| Error::invalid_response("dmReturnedMessage is missing"))?;
    let contents = soap::child(returned, ISDS_NS, "dmDm")
        .ok_or_else(|| Error::invalid_response("dmReturnedMessage/dmDm is missing"))?;

    let envelope = message_common::parse_envelope(contents)?;
    let attachments = message_common::parse_attachments(contents)?;
    let hash = message_common::parse_hash(returned)?;
    let qualified_timestamp = soap::child(returned, ISDS_NS, "dmQTimestamp")
        .filter(|node| !soap::is_nil(*node))
        .map(|node| message_common::decode_base64(node.text().unwrap_or(""), "dmQTimestamp"))
        .transpose()?;
    let delivery_time = message_common::optional_timestamp(returned, "dmDeliveryTime")?;
    let acceptance_time = message_common::optional_timestamp(returned, "dmAcceptanceTime")?;
    let state = message_common::required_state(returned)?;
    let attachment_size = message_common::optional_i64(returned, "dmAttachmentSize")?;
    let message_type = returned.attribute("dmType").map(str::to_owned);
    let special_message_flag = returned
        .attribute("specMessFlag")
        .map(|value| soap::parse(value, "specMessFlag"))
        .transpose()?;

    Ok(Message {
        envelope,
        attachments,
        hash,
        qualified_timestamp,
        delivery_time,
        acceptance_time,
        state,
        attachment_size,
        message_type,
        special_message_flag,
    })
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::model::{AttachmentType, MessageState};

    use super::*;

    const RESPONSE: &str = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
      <soap:Body><MessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20"
        xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
        <dmReturnedMessage dmType="O" specMessFlag="9"><dmDm>
          <dmID>00042</dmID><dbIDSender>egtk37z</dbIDSender><dmSender>Sender</dmSender>
          <dmSenderType>31</dmSenderType><dbIDRecipient>n94k37v</dbIDRecipient>
          <dmAnnotation>Synthetic message</dmAnnotation><dmPersonalDelivery>false</dmPersonalDelivery>
          <dmFiles>
            <dmFile dmMimeType="application/octet-stream" dmFileMetaType="main"
              dmFileDescr="data.bin"><dmEncodedContent>AAEC
              /f7/</dmEncodedContent></dmFile>
            <dmFile dmMimeType="application/xml" dmFileMetaType="enclosure"
              dmFileDescr="form.xml" dmFileGuid="guid-1"><dmXMLContent><note xmlns="urn:example"><value>A&amp;B</value></note></dmXMLContent></dmFile>
          </dmFiles>
        </dmDm><dmHash algorithm="SHA-256">AAEC/f7/</dmHash>
        <dmQTimestamp xsi:nil="true"/><dmDeliveryTime>2026-11-19T12:00:00+01:00</dmDeliveryTime>
        <dmAcceptanceTime>2026-11-19T12:01:00+01:00</dmAcceptanceTime>
        <dmMessageStatus>6</dmMessageStatus><dmAttachmentSize>1</dmAttachmentSize>
        </dmReturnedMessage>
      </MessageDownloadResponse></soap:Body></soap:Envelope>"#;

    #[test]
    fn request_uses_the_opaque_message_id() {
        let id = MessageId::new("00042").unwrap();
        let xml = String::from_utf8(download_request(&id).unwrap()).unwrap();
        assert!(xml.contains("<MessageDownload "));
        assert!(xml.contains("<dmID>00042</dmID>"));
    }

    #[test]
    fn downloads_envelope_binary_and_xml_attachments_without_optional_status() {
        let message = soap::decode(
            RESPONSE.as_bytes(),
            "MessageDownloadResponse",
            decode_download,
        )
        .unwrap();
        assert_eq!(message.envelope.id.as_str(), "00042");
        assert_eq!(message.envelope.sender_type, 31);
        assert_eq!(message.envelope.recipient_id.unwrap().as_str(), "n94k37v");
        assert_eq!(message.state, MessageState::DeliveredByLogin);
        assert_eq!(message.message_type.as_deref(), Some("O"));
        assert_eq!(message.special_message_flag, Some(9));
        assert_eq!(message.hash.algorithm.as_deref(), Some("SHA-256"));
        assert_eq!(
            message.hash.bytes,
            Bytes::from_static(&[0, 1, 2, 253, 254, 255])
        );
        assert_eq!(message.qualified_timestamp, None);
        assert_eq!(message.attachments.len(), 2);
        assert_eq!(message.attachments[0].meta_type(), &AttachmentType::Main);
        assert_eq!(
            message.attachments[0].content(),
            &Bytes::from_static(&[0, 1, 2, 253, 254, 255])
        );
        assert_eq!(
            message.attachments[1].xml_content(),
            Some(r#"<note xmlns="urn:example"><value>A&amp;B</value></note>"#)
        );
        assert_eq!(message.attachments[1].guid(), Some("guid-1"));
    }

    #[test]
    fn malformed_base64_is_an_invalid_response() {
        let invalid = RESPONSE.replace("AAEC\n              /f7/", "not base64!");
        let error = soap::decode(
            invalid.as_bytes(),
            "MessageDownloadResponse",
            decode_download,
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }
}
