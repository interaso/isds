use std::fmt;
use std::str::FromStr;

use bytes::Bytes;
use jiff::Timestamp;
use mime::Mime;

use crate::error::{Error, Result};
use crate::model::DataBoxId;

/// An opaque ISDS message identifier.
///
/// Message IDs have no protocol-defined chronological order and must not be
/// treated as integers. `Ord` is deterministic lexicographic ordering for
/// container use only; it never represents chronology or pagination. With the
/// `serde` feature, this serializes as a string and deserialization performs the
/// same validation as [`MessageId::new`].
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

impl TryFrom<String> for MessageId {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        Self::new(value)
    }
}

impl From<MessageId> for String {
    fn from(id: MessageId) -> Self {
        id.0
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for MessageId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MessageId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Standard or large-message behavior for sending, downloading, and metadata.
///
/// Pass `Some(mode)` to force a mode, or `None` to select it automatically.
/// Returned messages use the same optional representation when ISDS supplies it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum MessageMode {
    /// Use ordinary `CreateMessage` or `MessageDownload` operations.
    Standard,
    /// Use large-message upload/creation or `BigMessageDownload` operations.
    Large,
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

    pub(crate) fn from_raw(value: &str) -> Self {
        match value {
            "main" => Self::Main,
            "enclosure" => Self::Enclosure,
            "signature" => Self::Signature,
            "meta" => Self::Meta,
            value => Self::Other(value.to_owned()),
        }
    }
}

/// A fully buffered ordinary or large-volume message attachment.
///
/// SOAP base64 encoding increases the encoded size. MTOM/XOP streaming is not
/// currently supported.
#[derive(Clone)]
pub struct Attachment {
    description: String,
    mime_type: Mime,
    content: Bytes,
    xml_element: bool,
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
            xml_element: false,
            meta_type: AttachmentType::Enclosure,
            guid: None,
            up_file_guid: None,
            format: None,
        }
    }

    /// Creates an attachment from the contents of an ISDS `dmXMLContent` element.
    ///
    /// When sent through this crate, the same XML bytes are encoded using the
    /// schema's equivalent `dmEncodedContent` choice.
    pub fn xml(description: impl Into<String>, mime_type: Mime, xml: impl Into<String>) -> Self {
        let mut attachment = Self::new(description, mime_type, Bytes::from(xml.into()));
        attachment.xml_element = true;
        attachment
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

    /// Returns the XML source when this attachment used `dmXMLContent`.
    pub fn xml_content(&self) -> Option<&str> {
        if self.xml_element {
            std::str::from_utf8(&self.content).ok()
        } else {
            None
        }
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
            .field("xml_element", &self.xml_element)
            .field("meta_type", &self.meta_type)
            .field("guid", &self.guid)
            .field("up_file_guid", &self.up_file_guid)
            .field("format", &self.format)
            .finish()
    }
}

/// Optional metadata shared by outgoing and returned messages.
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

/// A message ready for automatic ordinary or large-volume sending.
#[derive(Clone, Debug)]
pub struct OutgoingMessage {
    /// Recipient databox ID.
    pub recipient: DataBoxId,
    /// Message subject or annotation, at most 255 Unicode characters.
    pub annotation: String,
    /// Optional envelope metadata.
    pub metadata: MessageMetadata,
    /// Whether to send the message in public-authority mode.
    pub public_authority_mode: Option<bool>,
    /// Optional sender-ID publication setting.
    pub publish_own_id: Option<PublishOwnId>,
    /// Optional one-character `dmType` attribute.
    pub message_type: Option<String>,
    /// Attachments; ISDS requires at least one and permits at most 100.
    pub attachments: Vec<Attachment>,
}

impl OutgoingMessage {
    /// Creates a message with no attachments or optional metadata.
    pub fn new(recipient: DataBoxId, annotation: impl Into<String>) -> Self {
        Self {
            recipient,
            annotation: annotation.into(),
            metadata: MessageMetadata::default(),
            public_authority_mode: None,
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

/// Successful acknowledgement from `CreateMessage` or `CreateBigMessage`.
///
/// This confirms acceptance by ISDS, not delivery to the recipient.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SentMessage {
    /// ID assigned by ISDS.
    pub id: MessageId,
}

/// A message state returned by ISDS.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum MessageState {
    /// Submitted (`1`).
    Submitted,
    /// Submission timestamp assigned (`2`).
    SubmissionTimestamped,
    /// Rejected by antivirus processing (`3`).
    AntivirusRejected,
    /// Placed in the recipient databox but not necessarily legally delivered (`4`).
    PlacedInRecipientBox,
    /// Delivered by legal fiction (`5`).
    DeliveredByFiction,
    /// Delivered by authorized login (`6`).
    DeliveredByLogin,
    /// Marked as read (`7`).
    Read,
    /// Undeliverable (`8`).
    Undeliverable,
    /// Erased (`9`).
    Erased,
    /// Stored in Datový trezor (`10`).
    InDataVault,
    /// A state introduced after this crate version.
    Unknown(i32),
}

impl MessageState {
    /// Returns the numeric ISDS state code.
    pub fn code(self) -> i32 {
        match self {
            Self::Submitted => 1,
            Self::SubmissionTimestamped => 2,
            Self::AntivirusRejected => 3,
            Self::PlacedInRecipientBox => 4,
            Self::DeliveredByFiction => 5,
            Self::DeliveredByLogin => 6,
            Self::Read => 7,
            Self::Undeliverable => 8,
            Self::Erased => 9,
            Self::InDataVault => 10,
            Self::Unknown(code) => code,
        }
    }

    pub(crate) fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Submitted,
            2 => Self::SubmissionTimestamped,
            3 => Self::AntivirusRejected,
            4 => Self::PlacedInRecipientBox,
            5 => Self::DeliveredByFiction,
            6 => Self::DeliveredByLogin,
            7 => Self::Read,
            8 => Self::Undeliverable,
            9 => Self::Erased,
            10 => Self::InDataVault,
            code => Self::Unknown(code),
        }
    }
}

/// Query and paging parameters for sent- or received-message lists.
///
/// `from` and `to` filter `dmDeliveryTime`, not submission or acceptance time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageListQuery {
    /// Optional inclusive lower delivery-time bound.
    pub from: Option<Timestamp>,
    /// Optional upper delivery-time bound.
    pub to: Option<Timestamp>,
    /// Sender or recipient organizational-unit number, depending on the operation.
    pub organization_unit_number: Option<i64>,
    /// States to include; `None` sends the ISDS all-permitted-states value `-1`.
    pub states: Option<Vec<MessageState>>,
    /// One-based result offset. The default is `1`.
    pub offset: u64,
    /// Maximum number of records requested. The default is `1000`.
    pub limit: u64,
}

impl Default for MessageListQuery {
    fn default() -> Self {
        Self {
            from: None,
            to: None,
            organization_unit_number: None,
            states: None,
            offset: 1,
            limit: 1000,
        }
    }
}

/// The common envelope returned in lists, downloads, and delivery information.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MessageEnvelope {
    /// Opaque message ID.
    pub id: MessageId,
    /// Sender databox ID.
    pub sender_id: Option<DataBoxId>,
    /// Sender display name.
    pub sender: Option<String>,
    /// Sender address.
    pub sender_address: Option<String>,
    /// Raw sender type code; newer fine-grained values are preserved.
    pub sender_type: i32,
    /// Recipient display name.
    pub recipient: Option<String>,
    /// Recipient address.
    pub recipient_address: Option<String>,
    /// Whether the recipient was ambiguous.
    pub ambiguous_recipient: Option<bool>,
    /// Recipient databox ID.
    pub recipient_id: Option<DataBoxId>,
    /// Message annotation.
    pub annotation: Option<String>,
    /// Shared optional message metadata.
    pub metadata: MessageMetadata,
}

/// One entry in a sent- or received-message page.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MessageRecord {
    /// One-based ordinal within the returned result set.
    pub ordinal: u32,
    /// Message envelope.
    pub envelope: MessageEnvelope,
    /// Current message state.
    pub state: MessageState,
    /// Rounded attachment size in kilobytes.
    pub attachment_size: Option<i64>,
    /// Time at which the message entered the databox.
    pub delivery_time: Option<Timestamp>,
    /// Time of legal acceptance, if applicable.
    pub acceptance_time: Option<Timestamp>,
    /// Optional raw `dmType` attribute.
    pub message_type: Option<String>,
    /// Standard or large-message mode, if supplied by ISDS.
    pub mode: Option<MessageMode>,
    /// Optional raw special-message flag.
    pub special_message_flag: Option<i64>,
}

/// One explicitly requested page of message records.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct MessagePage {
    /// Returned records.
    pub records: Vec<MessageRecord>,
    /// One-based offset used for this page.
    pub offset: u64,
    /// Limit used for this page.
    pub limit: u64,
}

/// Base64-decoded message hash and its optional algorithm identifier.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct MessageHash {
    /// Raw hash bytes.
    pub bytes: Bytes,
    /// Hash algorithm name supplied by ISDS.
    pub algorithm: Option<String>,
}

impl fmt::Debug for MessageHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageHash")
            .field("length", &self.bytes.len())
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

/// A downloaded ordinary or large-volume received message.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Message {
    /// Message envelope.
    pub envelope: MessageEnvelope,
    /// Decoded binary or XML attachments.
    pub attachments: Vec<Attachment>,
    /// ISDS-computed message hash.
    pub hash: MessageHash,
    /// Qualified timestamp token, if present.
    pub qualified_timestamp: Option<Bytes>,
    /// Time at which the message entered the databox.
    pub delivery_time: Option<Timestamp>,
    /// Time of legal acceptance, if applicable.
    pub acceptance_time: Option<Timestamp>,
    /// Current message state.
    pub state: MessageState,
    /// Rounded attachment size in kilobytes.
    pub attachment_size: Option<i64>,
    /// Optional raw `dmType` attribute.
    pub message_type: Option<String>,
    /// Optional raw special-message flag.
    pub special_message_flag: Option<i64>,
    /// Standard or large-message mode, if supplied by ISDS.
    pub mode: Option<MessageMode>,
    /// Total attachment count supplied for a large-volume message.
    pub attachment_count: Option<u32>,
}

/// A standalone received-message envelope returned by `MessageEnvelopeDownload`.
///
/// Unlike [`Message`], this result intentionally contains no attachment content.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MessageEnvelopeDetails {
    /// Message envelope.
    pub envelope: MessageEnvelope,
    /// ISDS-computed message hash.
    pub hash: MessageHash,
    /// Qualified timestamp token.
    pub qualified_timestamp: Bytes,
    /// Time at which the message entered the databox.
    pub delivery_time: Option<Timestamp>,
    /// Time of legal acceptance, if applicable.
    pub acceptance_time: Option<Timestamp>,
    /// Current message state.
    pub state: MessageState,
    /// Rounded attachment size in kilobytes.
    pub attachment_size: Option<i64>,
    /// Optional raw `dmType` attribute.
    pub message_type: Option<String>,
    /// Standard or large-message mode, if supplied by ISDS.
    pub mode: Option<MessageMode>,
    /// Total attachment count supplied for a large-volume message.
    pub attachment_count: Option<u32>,
    /// Optional raw special-message flag.
    pub special_message_flag: Option<i64>,
}

/// One event in a message's delivery history.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DeliveryEvent {
    /// Event timestamp.
    pub time: Option<Timestamp>,
    /// Human-readable event description.
    pub description: Option<String>,
}

/// Unsigned delivery information for a sent or received message.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DeliveryInfo {
    /// Message envelope without attachments.
    pub envelope: MessageEnvelope,
    /// ISDS-computed message hash.
    pub hash: MessageHash,
    /// Qualified timestamp token.
    pub qualified_timestamp: Bytes,
    /// Time at which the message entered the databox.
    pub delivery_time: Option<Timestamp>,
    /// Time of legal acceptance, if applicable.
    pub acceptance_time: Option<Timestamp>,
    /// Current message state.
    pub state: MessageState,
    /// Ordered delivery events.
    pub events: Vec<DeliveryEvent>,
}
