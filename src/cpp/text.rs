//! Byte-to-text conversion for parsed C++ names.
//!
//! Symbol names are byte strings, not UTF-8. A name component whose bytes are
//! valid UTF-8 is printed exactly as it appears in the symbol. A component
//! that is not valid UTF-8 keeps its valid runs verbatim and renders each
//! invalid byte as `\xNN`, so no input byte is silently replaced or dropped.
//!
//! Both spellings come from the shared [`crate::parse::text`] helper with an
//! explicit policy: identifiers stay reversible (a backslash or control
//! character is escaped), while a compiler payload keeps every valid byte.

use crate::parse::{Escape, text};
use alloc::borrow::Cow;
#[cfg(feature = "cpp-msvc")]
use {crate::parse::owned, alloc::string::String};

/// Interpret identifier bytes as node text.
///
/// Valid UTF-8 that needs no escaping borrows the input; otherwise the escaped
/// spelling is owned by the node.
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    text(bytes, Escape::Identifier)
}

/// Interpret compiler payload bytes (a string literal) as owned node text.
#[cfg(feature = "cpp-msvc")]
pub fn owned_text(bytes: &[u8]) -> String {
    owned(bytes, Escape::Verbatim)
}

#[cfg(test)]
mod tests {
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
}
