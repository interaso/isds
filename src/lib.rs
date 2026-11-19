#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! A high-level asynchronous client for Czech ISDS databoxes.
//!
//! The crate currently supports username/password authentication and ordinary
//! message operations. Callers must explicitly choose [`Environment::Test`] or
//! [`Environment::Production`]. No network request is made while constructing a
//! client.
//!
//! `GetListOfReceivedMessages` can cause legal delivery-by-login. Consult the
//! operation-level documentation before using received-message listing in production.

mod client;
mod error;
mod model;
mod wire;

pub use client::{Client, ClientBuilder, Credentials, Environment};
pub use error::{Error, Result, ServiceError};
pub use model::{
    Attachment, AttachmentType, CreditEvent, CreditEventDetails, CreditInfo, CreditQuery, DataBox,
    DataBoxId, DataBoxSearchResult, DataBoxState, DataBoxType, DataVaultChange, MessageId,
    MessageMetadata, OutgoingMessage, PublishOwnId, SentMessage,
};
