use std::fmt;
use std::str::FromStr;

use jiff::civil::Date;

use crate::error::{Error, Result};

const ID_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// A validated seven-character ISDS databox identifier.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DataBoxId(String);

impl DataBoxId {
    /// Validates an ISDS databox ID, including its Luhn32 check character.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_id(&value)?;
        Ok(Self(value))
    }

    /// Returns the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DataBoxId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for DataBoxId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for DataBoxId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl FromStr for DataBoxId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl TryFrom<&str> for DataBoxId {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// A databox type code such as `FO`, `PFO`, `PO`, or `OVM`.
///
/// The raw value is retained so newly introduced ISDS subtypes remain usable.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DataBoxType(String);

impl DataBoxType {
    /// Creates a non-empty databox type code.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(Error::invalid_input(
                "db_type",
                "databox type must not be empty",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the raw ISDS databox type code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DataBoxType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DataBoxType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Databox and owner details, also used as filters for `FindDataBox2`.
///
/// Start with [`DataBox::default`] or [`DataBox::by_id`] and set only the desired
/// search fields. Unset fields are encoded using the schema's required `xsi:nil`
/// representation. Response fields can be absent when the caller is not allowed
/// to see them or when they do not apply to the databox type.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataBox {
    /// Databox identifier.
    pub id: Option<DataBoxId>,
    /// Whether a personal databox is identified through the basic population register.
    pub aifo_isds: Option<bool>,
    /// Raw databox type code.
    pub box_type: Option<DataBoxType>,
    /// Subject identification number (IČO or a foreign equivalent).
    pub identification_number: Option<String>,
    /// All given names of a natural person.
    pub given_names: Option<String>,
    /// Surname of a natural person.
    pub last_name: Option<String>,
    /// Legal entity or business name.
    pub firm_name: Option<String>,
    /// Date of birth.
    pub birth_date: Option<Date>,
    /// City of birth.
    pub birth_city: Option<String>,
    /// County of birth.
    pub birth_county: Option<String>,
    /// Country of birth.
    pub birth_state: Option<String>,
    /// RÚIAN address-point code.
    pub address_code: Option<String>,
    /// Address city.
    pub address_city: Option<String>,
    /// Address district.
    pub address_district: Option<String>,
    /// Address street.
    pub address_street: Option<String>,
    /// Street orientation or descriptive number.
    pub address_number_in_street: Option<String>,
    /// Municipality house number.
    pub address_number_in_municipality: Option<String>,
    /// Postal code.
    pub address_zip_code: Option<String>,
    /// Address country.
    pub address_state: Option<String>,
    /// Nationality or legal-entity registration country.
    pub nationality: Option<String>,
    /// Identifier from the Register of Public Authorities.
    pub ovm_id: Option<String>,
    /// Raw databox state code; only `1` denotes an active box in owner information.
    pub state: Option<i64>,
    /// Whether a non-OVM databox has open addressing enabled.
    pub open_addressing: Option<bool>,
    /// Identifier of the superior OVM databox.
    pub upper_id: Option<DataBoxId>,
}

impl DataBox {
    /// Creates an exact-ID databox lookup.
    pub fn by_id(id: DataBoxId) -> Self {
        Self {
            id: Some(id),
            ..Self::default()
        }
    }
}

/// Results and continuation information from a databox lookup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DataBoxSearchResult {
    /// Returned databox records.
    pub boxes: Vec<DataBox>,
    /// Whether ISDS reported that additional results were truncated.
    pub truncated: bool,
    /// Whether a found non-OVM box cannot receive the caller's postal data message.
    ///
    /// This corresponds to informational status `0009` and does not describe
    /// general availability for ordinary data messages.
    pub postal_delivery_restricted: bool,
}

/// State code returned by `CheckDataBox`.
///
/// Unknown values are retained so newer ISDS states do not break decoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DataBoxState(i32);

impl DataBoxState {
    pub(crate) fn from_code(code: i32) -> Self {
        Self(code)
    }

    /// Returns the raw ISDS state code.
    pub fn code(self) -> i32 {
        self.0
    }

    /// Returns whether this is the active state (`1`).
    pub fn is_active(self) -> bool {
        self.0 == 1
    }
}

fn validate_id(value: &str) -> Result<()> {
    if value.len() != 7 || !value.is_ascii() {
        return Err(Error::invalid_input(
            "db_id",
            "a databox ID must contain exactly seven ASCII characters",
        ));
    }

    let bytes = value.as_bytes();
    if bytes.iter().any(|byte| !ID_ALPHABET.contains(byte)) {
        return Err(Error::invalid_input(
            "db_id",
            "databox ID contains a character outside the ISDS alphabet",
        ));
    }

    let expected = check_character(&bytes[..6]);
    if bytes[6] != expected {
        return Err(Error::invalid_input(
            "db_id",
            "databox ID has an invalid Luhn32 check character",
        ));
    }
    Ok(())
}

fn check_character(data: &[u8]) -> u8 {
    let sum = data.iter().enumerate().fold(0_u32, |sum, (index, byte)| {
        let mut value = ID_ALPHABET.iter().position(|item| item == byte).unwrap() as u32;
        if index % 2 == 1 {
            value *= 2;
            if value >= 32 {
                value = value - 32 + 1;
            }
        }
        sum + value
    });
    ID_ALPHABET[((32 - sum % 32) % 32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_ids() {
        for id in ["egtk37z", "n94k37v", "djwk5ns"] {
            assert_eq!(DataBoxId::new(id).unwrap().as_str(), id);
        }
    }

    #[test]
    fn rejects_invalid_checksum_and_characters() {
        assert!(DataBoxId::new("egtk37x").is_err());
        assert!(DataBoxId::new("EGTK37Z").is_err());
        assert!(DataBoxId::new("egtk37").is_err());
        assert!(DataBoxId::new("egtk37zz").is_err());
    }

    #[test]
    fn preserves_unknown_state_codes() {
        let state = DataBoxState::from_code(99);
        assert_eq!(state.code(), 99);
        assert!(!state.is_active());
    }

    #[test]
    fn databox_type_preserves_future_values() {
        let value = DataBoxType::new("FUTURE_TYPE").unwrap();
        assert_eq!(value.as_str(), "FUTURE_TYPE");
        assert!(DataBoxType::new("").is_err());
    }
}
