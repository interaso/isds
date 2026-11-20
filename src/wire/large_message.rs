use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use quick_xml::Writer;
use quick_xml::events::{BytesStart, Event};
use roxmltree::Node;
use sha2::{Digest as _, Sha256, Sha512};
use sha3::{Sha3_256, Sha3_512};

use crate::error::{Error, Result};
use crate::model::{Attachment, AttachmentType, Message, MessageId, OutgoingMessage, SentMessage};
use crate::wire::soap::{ISDS_NS, SoapVersion};
use crate::wire::{message, message_download, soap};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttachmentHash {
    value: String,
    algorithm: String,
}

#[derive(Clone, Debug)]
pub(crate) struct UploadedAttachment {
    attachment_id: String,
    hash1: AttachmentHash,
    hash2: AttachmentHash,
    meta_type: AttachmentType,
    guid: Option<String>,
    up_file_guid: Option<String>,
}

pub(crate) fn upload_attachment_request(attachment: &Attachment) -> Result<Vec<u8>> {
    message::validate_attachment_type(attachment)?;
    soap::request_with_version(SoapVersion::Soap12, "UploadAttachment", |writer| {
        let mut file = BytesStart::new("dmFile");
        let mime_type = attachment.mime_type().to_string();
        file.push_attribute(("dmMimeType", mime_type.as_str()));
        file.push_attribute(("dmFileDescr", attachment.description()));
        writer.write_event(Event::Start(file))?;
        soap::text_element(
            writer,
            "dmEncodedContent",
            &BASE64.encode(attachment.content()),
        )?;
        writer.write_event(Event::End(quick_xml::events::BytesEnd::new("dmFile")))?;
        Ok(())
    })
}

pub(crate) fn decode_upload_attachment(
    response: Node<'_, '_>,
    attachment: &Attachment,
) -> Result<UploadedAttachment> {
    soap::status(response, "dm")?.into_result()?;
    let attachment_id = soap::required_text(response, ISDS_NS, "dmAttID")?;
    let hash1 = required_hash(response, "dmAttHash1")?;
    let hash2 = required_hash(response, "dmAttHash2")?;
    verify_hash(attachment, &hash1)?;
    verify_hash(attachment, &hash2)?;
    Ok(UploadedAttachment {
        attachment_id,
        hash1,
        hash2,
        meta_type: attachment.meta_type().clone(),
        guid: attachment.guid().map(str::to_owned),
        up_file_guid: attachment.up_file_guid().map(str::to_owned),
    })
}

fn required_hash(response: Node<'_, '_>, name: &str) -> Result<AttachmentHash> {
    let node = soap::child(response, ISDS_NS, name)?
        .ok_or_else(|| Error::invalid_response(format!("{name} is missing")))?;
    let value = soap::required_text(response, ISDS_NS, name)?;
    let algorithm = node.attribute("AttHashAlg").ok_or_else(|| {
        Error::invalid_response(format!(
            "{name}/AttHashAlg is required for CreateBigMessage"
        ))
    })?;
    if algorithm.is_empty() {
        return Err(Error::invalid_response(format!(
            "{name}/AttHashAlg must not be empty"
        )));
    }
    Ok(AttachmentHash {
        value,
        algorithm: algorithm.to_owned(),
    })
}

fn verify_hash(attachment: &Attachment, hash: &AttachmentHash) -> Result<()> {
    let expected = match hash.algorithm.as_str() {
        "SHA-256" => hex::encode(Sha256::digest(attachment.content())),
        "SHA-512" => hex::encode(Sha512::digest(attachment.content())),
        "SHA3-256" => hex::encode(Sha3_256::digest(attachment.content())),
        "SHA3-512" => hex::encode(Sha3_512::digest(attachment.content())),
        // ISDS explicitly allows algorithms to change. Unknown algorithms remain
        // usable and are verified by ISDS again during CreateBigMessage.
        _ => return Ok(()),
    };
    if hash.value.eq_ignore_ascii_case(&expected) {
        Ok(())
    } else {
        Err(Error::invalid_response(format!(
            "{} does not match the uploaded attachment",
            hash.algorithm
        )))
    }
}

pub(crate) fn create_large_message_request(
    message: &OutgoingMessage,
    uploaded: &[UploadedAttachment],
) -> Result<Vec<u8>> {
    message::validate_outgoing(message)?;
    if uploaded.is_empty() {
        return Err(Error::invalid_input(
            "uploaded_attachments",
            "CreateBigMessage requires at least one uploaded attachment",
        ));
    }
    if uploaded.len() != message.attachments.len() {
        return Err(Error::invalid_input(
            "uploaded_attachments",
            "every large-message attachment must have exactly one upload result",
        ));
    }

    soap::request_with_version(SoapVersion::Soap12, "CreateBigMessage", |writer| {
        message::write_envelope(writer, message)?;
        write_external_files(writer, uploaded)
    })
}

fn write_external_files(
    writer: &mut Writer<Vec<u8>>,
    uploaded: &[UploadedAttachment],
) -> soap::XmlResult {
    writer.write_event(Event::Start(BytesStart::new("dmFiles")))?;
    for attachment in uploaded {
        let mut file = BytesStart::new("dmExtFile");
        file.push_attribute(("dmFileMetaType", attachment.meta_type.as_str()));
        file.push_attribute(("dmAttID", attachment.attachment_id.as_str()));
        file.push_attribute(("dmAttHash1", attachment.hash1.value.as_str()));
        file.push_attribute(("dmAttHash1Alg", attachment.hash1.algorithm.as_str()));
        file.push_attribute(("dmAttHash2", attachment.hash2.value.as_str()));
        file.push_attribute(("dmAttHash2Alg", attachment.hash2.algorithm.as_str()));
        if let Some(guid) = &attachment.guid {
            file.push_attribute(("dmFileGuid", guid.as_str()));
        }
        if let Some(guid) = &attachment.up_file_guid {
            file.push_attribute(("dmUpFileGuid", guid.as_str()));
        }
        writer.write_event(Event::Empty(file))?;
    }
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new("dmFiles")))?;
    Ok(())
}

pub(crate) fn decode_create_large_message(response: Node<'_, '_>) -> Result<SentMessage> {
    message::decode_create_message(response)
}

pub(crate) fn large_message_download_request(id: &MessageId) -> Result<Vec<u8>> {
    soap::request_with_version(SoapVersion::Soap12, "BigMessageDownload", |writer| {
        soap::text_element(writer, "dmID", id.as_str())
    })
}

pub(crate) fn decode_large_message_download(response: Node<'_, '_>) -> Result<Message> {
    message_download::decode_download(response)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::model::{DataBoxId, MessageMode};

    use super::*;

    fn attachment() -> Attachment {
        Attachment::main(
            "large & test.bin",
            mime::APPLICATION_OCTET_STREAM,
            Bytes::from_static(&[0, 1, 2, 253, 254, 255]),
        )
        .with_guid("guid-1")
    }

    fn uploaded() -> UploadedAttachment {
        decode_upload_attachment(
            Document::parse(UPLOAD_RESPONSE).unwrap().root_element(),
            &attachment(),
        )
        .unwrap()
    }

    const UPLOAD_RESPONSE: &str = r#"<UploadAttachmentResponse xmlns="http://isds.czechpoint.cz/v20">
      <dmAttID>upload-1</dmAttID>
      <dmAttHash1 AttHashAlg="SHA-256">3f2d1552cdc7483f40dd720c80b900225dfecfd5cae7cd168d79ab6ee5959885</dmAttHash1>
      <dmAttHash2 AttHashAlg="SHA3-256">6b16378d8f97e4084b5f780e94ffcdf3277662087722074bb1588600213fbacd</dmAttHash2>
      <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
    </UploadAttachmentResponse>"#;

    const LARGE_DOWNLOAD_RESPONSE: &str = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
      <env:Body><BigMessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
        <dmReturnedMessage dmType="E" dmVODZ="true" attsNum="1"><dmDm>
          <dmID>00042</dmID><dmSenderType>31</dmSenderType>
          <dmFiles><dmFile dmMimeType="application/octet-stream" dmFileMetaType="main"
            dmFileDescr="large.bin"><dmEncodedContent>AAEC/f7/</dmEncodedContent></dmFile></dmFiles>
        </dmDm><dmHash>AAEC/f7/</dmHash><dmQTimestamp></dmQTimestamp>
        <dmMessageStatus>6</dmMessageStatus></dmReturnedMessage>
        <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
      </BigMessageDownloadResponse></env:Body></env:Envelope>"#;

    use roxmltree::Document;

    #[test]
    fn upload_uses_soap_12_and_only_the_documented_attributes() {
        let xml = String::from_utf8(upload_attachment_request(&attachment()).unwrap()).unwrap();
        assert!(xml.contains("http://www.w3.org/2003/05/soap-envelope"));
        assert!(xml.contains("dmFileDescr=\"large &amp; test.bin\""));
        assert!(xml.contains("dmMimeType=\"application/octet-stream\""));
        assert!(xml.contains("<dmEncodedContent>AAEC/f7/</dmEncodedContent>"));
        assert!(!xml.contains("dmFileMetaType"));
    }

    #[test]
    fn upload_rejects_an_empty_custom_attachment_type() {
        let attachment = attachment().with_meta_type(AttachmentType::Other(String::new()));
        assert!(matches!(
            upload_attachment_request(&attachment),
            Err(Error::InvalidInput {
                field: "attachments",
                ..
            })
        ));
    }

    #[test]
    fn create_large_message_uses_external_ids_and_both_hash_algorithms() {
        let message = OutgoingMessage::new(DataBoxId::new("n94k37v").unwrap(), "Large message")
            .with_attachment(attachment());
        let xml = String::from_utf8(create_large_message_request(&message, &[uploaded()]).unwrap())
            .unwrap();
        assert!(xml.contains("<CreateBigMessage "));
        assert!(xml.contains("dmFileMetaType=\"main\""));
        assert!(xml.contains("dmAttID=\"upload-1\""));
        assert!(xml.contains(
            "dmAttHash1=\"3f2d1552cdc7483f40dd720c80b900225dfecfd5cae7cd168d79ab6ee5959885\""
        ));
        assert!(xml.contains("dmAttHash1Alg=\"SHA-256\""));
        assert!(xml.contains(
            "dmAttHash2=\"6b16378d8f97e4084b5f780e94ffcdf3277662087722074bb1588600213fbacd\""
        ));
        assert!(xml.contains("dmAttHash2Alg=\"SHA3-256\""));
        assert!(xml.contains("dmFileGuid=\"guid-1\""));
        assert!(xml.contains("<dmExtFile "));
        assert!(!xml.contains("<dmFile "));
    }

    #[test]
    fn upload_rejects_missing_hash_metadata() {
        let invalid = UPLOAD_RESPONSE.replace(" AttHashAlg=\"SHA-256\"", "");
        let error = decode_upload_attachment(
            Document::parse(&invalid).unwrap().root_element(),
            &attachment(),
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }

    #[test]
    fn upload_rejects_known_hash_mismatch_before_create() {
        let invalid = UPLOAD_RESPONSE.replace(
            "3f2d1552cdc7483f40dd720c80b900225dfecfd5cae7cd168d79ab6ee5959885",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let error = decode_upload_attachment(
            Document::parse(&invalid).unwrap().root_element(),
            &attachment(),
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }

    #[test]
    fn large_download_reuses_inline_attachment_decoding_and_preserves_metadata() {
        let message = soap::decode(
            LARGE_DOWNLOAD_RESPONSE.as_bytes(),
            "BigMessageDownloadResponse",
            decode_large_message_download,
        )
        .unwrap();
        assert_eq!(message.mode, Some(MessageMode::Large));
        assert_eq!(message.attachment_count, Some(1));
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(
            message.attachments[0].content().as_ref(),
            &[0, 1, 2, 253, 254, 255]
        );
    }

    #[test]
    fn large_download_rejects_attachment_count_mismatch() {
        let invalid = LARGE_DOWNLOAD_RESPONSE.replace("attsNum=\"1\"", "attsNum=\"2\"");
        let error = soap::decode(
            invalid.as_bytes(),
            "BigMessageDownloadResponse",
            decode_large_message_download,
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }
}
