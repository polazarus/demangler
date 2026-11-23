//! Byte-to-text conversion for parsed C++ names.
//!
//! Symbol names are byte strings, not UTF-8. A name component whose bytes are
//! valid UTF-8 is printed exactly as it appears in the symbol. A component
//! that is not valid UTF-8 keeps its valid runs verbatim and renders each
//! invalid byte as `\xNN`, so no input byte is silently replaced or dropped.

use alloc::borrow::Cow;
use alloc::string::String;

use crate::parse::{Escape, write_escaped};

/// Interpret identifier bytes as node text.
///
/// Valid UTF-8 borrows the input; otherwise the escaped spelling is owned by
/// the node.
pub(crate) fn decode(bytes: &[u8]) -> Cow<'_, str> {
    core::str::from_utf8(bytes).map_or_else(|_| Cow::Owned(escaped(bytes)), Cow::Borrowed)
}

/// Interpret compiler payload bytes (a string literal) as owned node text.
#[cfg(feature = "cpp-msvc")]
pub(crate) fn owned_text(bytes: &[u8]) -> String {
    core::str::from_utf8(bytes).map_or_else(|_| escaped(bytes), String::from)
}

/// Escape the bytes that are not valid UTF-8, keeping valid runs verbatim.
fn escaped(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    write_escaped(&mut out, bytes, Escape::Verbatim).expect("writing to a String cannot fail");
    out
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
}
