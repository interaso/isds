#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc = include_str!("../README.md")]

mod client;
mod error;
mod model;
mod wire;

pub use client::{Client, ClientBuilder, Credentials, Environment};
pub use error::{Error, Result, ServiceError};
pub use model::{
    Attachment, AttachmentType, CreditEvent, CreditEventDetails, CreditInfo, CreditQuery, DataBox,
    DataBoxId, DataBoxSearchResult, DataBoxState, DataBoxType, DataVaultChange, DeliveryEvent,
    DeliveryInfo, Message, MessageEnvelope, MessageEnvelopeDetails, MessageHash, MessageId,
    MessageListQuery, MessageMetadata, MessageMode, MessagePage, MessageRecord, MessageState,
    OutgoingMessage, PublishOwnId, SentMessage,
};
