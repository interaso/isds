use std::fmt;
use std::str::FromStr;

use bytes::Bytes;
use mime::Mime;

use crate::error::{Error, Result};
use crate::model::DataBoxId;

/// An opaque ISDS message identifier.
///
/// Message IDs are not ordered and must not be treated as integers.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MessageId(String);

impl MessageId {
    /// Creates a non-empty message ID within the schema's 20-character limit.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.chars().count() > 20 {
            return Err(Error::invalid_input(
                "message_id",
                "message ID must contain between 1 and 20 characters",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the identifier without changing its representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for MessageId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl FromStr for MessageId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<&str> for MessageId {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// ISDS attachment role.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum AttachmentType {
    /// Main document (`main`).
    Main,
    /// Ordinary enclosure (`enclosure`).
    Enclosure,
    /// Signature attachment (`signature`).
    Signature,
    /// Metadata attachment (`meta`).
    Meta,
    /// A value not known by this version of the crate.
    Other(String),
}

impl AttachmentType {
    /// Returns the exact ISDS attribute value.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Main => "main",
            Self::Enclosure => "enclosure",
            Self::Signature => "signature",
            Self::Meta => "meta",
            Self::Other(value) => value,
        }
    }
}

/// A fully buffered ordinary-message attachment.
///
/// SOAP base64 encoding increases the encoded size. Large VoDZ/MTOM attachments
/// are intentionally outside the initial ordinary-message API.
#[derive(Clone)]
pub struct Attachment {
    description: String,
    mime_type: Mime,
    content: Bytes,
    meta_type: AttachmentType,
    guid: Option<String>,
    up_file_guid: Option<String>,
    format: Option<String>,
}

impl Attachment {
    /// Creates an enclosure with binary content.
    pub fn new(description: impl Into<String>, mime_type: Mime, content: impl Into<Bytes>) -> Self {
        Self {
            description: description.into(),
            mime_type,
            content: content.into(),
            meta_type: AttachmentType::Enclosure,
            guid: None,
            up_file_guid: None,
            format: None,
        }
    }

    /// Creates a main-document attachment with binary content.
    pub fn main(
        description: impl Into<String>,
        mime_type: Mime,
        content: impl Into<Bytes>,
    ) -> Self {
        Self::new(description, mime_type, content).with_meta_type(AttachmentType::Main)
    }

    /// Changes the ISDS attachment role.
    pub fn with_meta_type(mut self, meta_type: AttachmentType) -> Self {
        self.meta_type = meta_type;
        self
    }

    /// Sets the optional attachment GUID.
    pub fn with_guid(mut self, guid: impl Into<String>) -> Self {
        self.guid = Some(guid.into());
        self
    }

    /// Sets the optional parent-signature attachment GUID.
    pub fn with_up_file_guid(mut self, guid: impl Into<String>) -> Self {
        self.up_file_guid = Some(guid.into());
        self
    }

    /// Sets the optional `dmFormat` value.
    ///
    /// The current ISDS schema documents this attribute as unused.
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Returns the attachment description or filename.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the declared MIME type.
    pub fn mime_type(&self) -> &Mime {
        &self.mime_type
    }

    /// Returns the decoded attachment bytes.
    pub fn content(&self) -> &Bytes {
        &self.content
    }

    /// Returns the ISDS attachment role.
    pub fn meta_type(&self) -> &AttachmentType {
        &self.meta_type
    }

    /// Returns the optional attachment GUID.
    pub fn guid(&self) -> Option<&str> {
        self.guid.as_deref()
    }

    /// Returns the optional parent-signature attachment GUID.
    pub fn up_file_guid(&self) -> Option<&str> {
        self.up_file_guid.as_deref()
    }

    /// Returns the optional, currently unused `dmFormat` value.
    pub fn format(&self) -> Option<&str> {
        self.format.as_deref()
    }
}

impl fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attachment")
            .field("description", &self.description)
            .field("mime_type", &self.mime_type)
            .field("content_length", &self.content.len())
            .field("meta_type", &self.meta_type)
            .field("guid", &self.guid)
            .field("up_file_guid", &self.up_file_guid)
            .field("format", &self.format)
            .finish()
    }
}

/// Optional metadata shared by outgoing and returned ordinary messages.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MessageMetadata {
    /// Sender organizational unit name.
    pub sender_org_unit: Option<String>,
    /// Sender organizational unit number.
    pub sender_org_unit_number: Option<i64>,
    /// Recipient organizational unit name.
    pub recipient_org_unit: Option<String>,
    /// Recipient organizational unit number.
    pub recipient_org_unit_number: Option<i64>,
    /// Informational name for personal delivery; not the legal personal-delivery flag.
    pub to_hands: Option<String>,
    /// Recipient reference number.
    pub recipient_reference_number: Option<String>,
    /// Sender reference number.
    pub sender_reference_number: Option<String>,
    /// Recipient file identifier.
    pub recipient_identification: Option<String>,
    /// Sender file identifier.
    pub sender_identification: Option<String>,
    /// Legal-title law number.
    pub legal_title_law: Option<i64>,
    /// Legal-title year.
    pub legal_title_year: Option<i64>,
    /// Legal-title section.
    pub legal_title_section: Option<String>,
    /// Legal-title paragraph.
    pub legal_title_paragraph: Option<String>,
    /// Legal-title point.
    pub legal_title_point: Option<String>,
    /// Whether delivery is restricted to the recipient personally (`do vlastních rukou`).
    pub personal_delivery: Option<bool>,
    /// Whether substitute delivery is allowed.
    pub allow_substitute_delivery: Option<bool>,
}

/// Controls whether the sender's databox ID is published to the recipient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublishOwnId {
    /// Whether the sender ID may be published.
    pub enabled: bool,
    /// Optional ISDS identification level.
    pub identification_level: Option<i64>,
}

/// An ordinary message ready for `CreateMessage`.
#[derive(Clone, Debug)]
pub struct OutgoingMessage {
    /// Recipient databox ID.
    pub recipient: DataBoxId,
    /// Message subject or annotation, at most 255 Unicode characters.
    pub annotation: String,
    /// Optional envelope metadata.
    pub metadata: MessageMetadata,
    /// Optional `dmOVM` flag.
    pub ovm: Option<bool>,
    /// Optional sender-ID publication setting.
    pub publish_own_id: Option<PublishOwnId>,
    /// Optional one-character `dmType` attribute.
    pub message_type: Option<String>,
    /// Ordinary attachments; ISDS requires at least one and permits at most 100.
    pub attachments: Vec<Attachment>,
}

impl OutgoingMessage {
    /// Creates an ordinary message with no attachments or optional metadata.
    pub fn new(recipient: DataBoxId, annotation: impl Into<String>) -> Self {
        Self {
            recipient,
            annotation: annotation.into(),
            metadata: MessageMetadata::default(),
            ovm: None,
            publish_own_id: None,
            message_type: None,
            attachments: Vec::new(),
        }
    }

    /// Appends an attachment.
    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Appends an attachment in place.
    pub fn add_attachment(&mut self, attachment: Attachment) {
        self.attachments.push(attachment);
    }
}

/// Successful acknowledgement from `CreateMessage`.
///
/// This confirms acceptance by ISDS, not delivery to the recipient.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SentMessage {
    /// ID assigned by ISDS.
    pub id: MessageId,
}
