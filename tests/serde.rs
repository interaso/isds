//! Verifies optional public serde support without changing ID validation.

#![cfg(feature = "serde")]

use isds::{DataBoxId, MessageId};

#[test]
fn message_id_is_a_string_and_deserialization_validates() {
    let id = MessageId::new("00042").unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""00042""#);

    let decoded: MessageId = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, id);
    assert!(serde_json::from_str::<MessageId>(r#""""#).is_err());
    assert!(serde_json::from_str::<MessageId>("42").is_err());
}

#[test]
fn databox_id_is_a_string_and_deserialization_checks_luhn32() {
    let id = DataBoxId::new("egtk37z").unwrap();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, r#""egtk37z""#);

    let decoded: DataBoxId = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, id);
    assert!(serde_json::from_str::<DataBoxId>(r#""egtk37x""#).is_err());
    assert!(serde_json::from_str::<DataBoxId>(r#""EGTK37Z""#).is_err());
}
