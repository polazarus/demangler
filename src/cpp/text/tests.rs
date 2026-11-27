use super::*;

#[test]
fn valid_text_borrows_and_is_unchanged() {
    assert!(matches!(decode(b"Foo"), Cow::Borrowed("Foo")));
    assert!(matches!(decode("Grüße".as_bytes()), Cow::Borrowed("Grüße")));
}

#[test]
fn invalid_bytes_are_escaped() {
    assert_eq!(decode(b"F\xffoo"), "F\\xffoo");
    assert_eq!(decode(b"\x80\x81"), "\\x80\\x81");
}

#[test]
fn identifier_spelling_is_reversible() {
    assert_eq!(decode(b"a\\b"), "a\\\\b");
    assert_eq!(decode(b"a\tb"), "a\\x09b");
}

#[cfg(feature = "cpp-msvc")]
#[test]
fn payload_spelling_keeps_valid_bytes() {
    assert_eq!(owned_text(b"a\\b"), "a\\b");
    assert_eq!(owned_text(b"a\xffb"), "a\\xffb");
}
