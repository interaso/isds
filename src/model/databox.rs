use std::fmt;
use std::str::FromStr;

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
}
