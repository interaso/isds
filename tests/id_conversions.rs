//! Exercises the public string conversion traits for validated identifiers.

use isds::{DataBoxId, MessageId};

#[test]
fn message_id_supports_fallible_string_input_and_infallible_output() {
    let borrowed = MessageId::try_from("00042").unwrap();
    let owned = MessageId::try_from(String::from("00042")).unwrap();
    assert_eq!(owned, borrowed);

    let raw: String = owned.into();
    assert_eq!(raw, "00042");
    assert!(MessageId::try_from(String::new()).is_err());
    assert!(MessageId::try_from(String::from("123456789012345678901")).is_err());
}

#[test]
fn databox_id_supports_fallible_string_input_and_infallible_output() {
    let borrowed = DataBoxId::try_from("egtk37z").unwrap();
    let owned = DataBoxId::try_from(String::from("egtk37z")).unwrap();
    assert_eq!(owned, borrowed);

    let raw: String = owned.into();
    assert_eq!(raw, "egtk37z");
    assert!(DataBoxId::try_from(String::from("egtk37x")).is_err());
    assert!(DataBoxId::try_from(String::from("EGTK37Z")).is_err());
}
