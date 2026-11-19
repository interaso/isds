use std::fmt;

/// An error returned by the ISDS service after a syntactically valid response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceError {
    code: String,
    message: String,
    reference_number: Option<String>,
}

impl ServiceError {
    pub(crate) fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        reference_number: Option<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            reference_number,
        }
    }

    /// Returns the untouched ISDS status code, including leading zeroes.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the human-readable message supplied by ISDS.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the optional ISDS request reference number.
    pub fn reference_number(&self) -> Option<&str> {
        self.reference_number.as_deref()
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ISDS status {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

/// Errors produced while configuring or calling the ISDS client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The supplied identifier, query, or client configuration is invalid.
    #[error("invalid input for {field}: {message}")]
    InvalidInput {
        /// Name of the invalid input field.
        field: &'static str,
        /// Description of the violated invariant.
        message: String,
    },

    /// An HTTP request could not be completed.
    ///
    /// If this occurs while sending a message, the server may already have
    /// accepted it. Reconcile through sent-message queries before resending.
    #[error("HTTP transport failed: {0}")]
    Transport(#[from] reqwest::Error),

    /// ISDS rejected the supplied authentication credentials.
    #[error("ISDS authentication failed with HTTP status {status}")]
    Authentication {
        /// Numeric HTTP status code.
        status: u16,
    },

    /// ISDS returned an HTTP error without a usable SOAP fault.
    #[error("ISDS returned HTTP status {status}")]
    Http {
        /// Numeric HTTP status code.
        status: u16,
    },

    /// The SOAP endpoint returned a fault.
    #[error("SOAP fault {code}: {message}")]
    SoapFault {
        /// SOAP fault code.
        code: String,
        /// SOAP fault message.
        message: String,
    },

    /// ISDS returned a non-success application status.
    #[error("{0}")]
    Service(#[from] ServiceError),

    /// The response exceeded the configured byte limit.
    #[error("ISDS response exceeded the configured {limit}-byte limit")]
    ResponseTooLarge {
        /// Maximum permitted response size in bytes.
        limit: usize,
    },

    /// The response was not a valid or complete representation of the operation.
    #[error("invalid ISDS response: {message}")]
    InvalidResponse {
        /// Description of the response mismatch; never contains the response body.
        message: String,
    },
}

impl Error {
    pub(crate) fn invalid_input(field: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidInput {
            field,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_response(message: impl Into<String>) -> Self {
        Self::InvalidResponse {
            message: message.into(),
        }
    }
}

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;
