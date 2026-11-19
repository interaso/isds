use jiff::civil::Date;
use jiff::{Span, Timestamp};

use crate::error::{Error, Result};
use crate::model::DataBoxId;

/// Query for current databox credit or a bounded credit-history interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditQuery {
    id: DataBoxId,
    from: Option<Date>,
    to: Option<Date>,
}

impl CreditQuery {
    /// Creates a query that returns only the current credit.
    pub fn current(id: DataBoxId) -> Self {
        Self {
            id,
            from: None,
            to: None,
        }
    }

    /// Creates a credit-history query.
    ///
    /// ISDS limits history intervals to 12 months. Dates denote Czech-local
    /// midnight; use tomorrow's date as the exclusive upper bound when today's
    /// records should be included.
    pub fn history(id: DataBoxId, from: Date, to: Date) -> Result<Self> {
        if to < from {
            return Err(Error::invalid_input(
                "credit_history",
                "the end date must not precede the start date",
            ));
        }
        let maximum = from
            .checked_add(Span::new().years(1))
            .map_err(|error| Error::invalid_input("credit_history", error.to_string()))?;
        if to > maximum {
            return Err(Error::invalid_input(
                "credit_history",
                "the history interval must not exceed 12 months",
            ));
        }
        Ok(Self {
            id,
            from: Some(from),
            to: Some(to),
        })
    }

    /// Returns the databox whose credit is queried.
    pub fn id(&self) -> &DataBoxId {
        &self.id
    }

    /// Returns the optional history start date.
    pub fn from(&self) -> Option<Date> {
        self.from
    }

    /// Returns the optional history end date.
    pub fn to(&self) -> Option<Date> {
        self.to
    }
}

/// Current credit and requested credit-history events.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CreditInfo {
    /// Current credit in haléře, if supplied by ISDS.
    pub current_credit: Option<i64>,
    /// Notification e-mail address associated with the credit account.
    pub notification_email: Option<String>,
    /// Credit-history events, empty for a current-credit-only query.
    pub events: Vec<CreditEvent>,
}

/// One credit-account event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CreditEvent {
    /// Event timestamp, including the offset supplied by ISDS.
    pub time: Timestamp,
    /// Raw ISDS event type code.
    pub event_type: i64,
    /// Credit change in haléře.
    pub credit_change: i64,
    /// Credit balance after this event in haléře.
    pub credit_after: i64,
    /// Event-type-specific details.
    pub details: CreditEventDetails,
}

/// Additional data associated with a credit event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CreditEventDetails {
    /// A credit transaction associated with event types 1 and 2.
    Transaction {
        /// ISDS transaction identifier.
        transaction_id: String,
    },
    /// A postal data message charge associated with event type 3.
    PostalMessage {
        /// Recipient databox ID.
        recipient_id: DataBoxId,
        /// Postal data message identifier.
        postal_message_id: String,
    },
    /// A Datový trezor capacity change associated with event type 4.
    DataVault(DataVaultChange),
    /// An administrative event containing only its operator.
    Administrative {
        /// User or system that performed the event.
        done_by: String,
    },
    /// No currently recognized event-specific payload was present.
    Unknown,
}

/// Details of a Datový trezor capacity change.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DataVaultChange {
    /// New capacity in the unit defined by ISDS.
    pub new_capacity: i64,
    /// Start date of the new capacity.
    pub new_from: Date,
    /// End date of the new capacity.
    pub new_to: Date,
    /// Previous capacity, if supplied.
    pub old_capacity: Option<i64>,
    /// Start date of the previous capacity.
    pub old_from: Option<Date>,
    /// End date of the previous capacity.
    pub old_to: Option<Date>,
    /// User or system that performed the change.
    pub done_by: Option<String>,
}
