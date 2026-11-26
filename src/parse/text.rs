//! Byte-to-text conversion shared by the node-building backends.
//!
//! A symbol name is a byte string: a length prefix counts bytes, so a
//! malformed symbol can slice a UTF-8 sequence in half. The parsers keep bytes
//! and turn them into the `str` node text a printer writes only here.
//!
//! The [`Escape`] policy is chosen per site, not per backend:
//!
//! - [`Escape::Identifier`] for identifier text. Valid UTF-8 that a printer
//!   would have to rewrite (a backslash or a control character) becomes owned
//!   text, so the spelling stays reversible and no control byte reaches a
//!   terminal or a line-oriented consumer.
//! - [`Escape::Verbatim`] for compiler payloads (string literals, unmangled
//!   suffixes, nested legacy payloads). Valid UTF-8 is kept exactly as the
//!   compiler wrote it, backslashes included; only bytes that are not part of
//!   a valid sequence become `\xNN`.
//!
//! Text that renders unchanged under its policy borrows the input, so an
//! ordinary name allocates nothing.

use alloc::{borrow::Cow, string::String};

use super::{Escape, write_escaped};

/// Text for `bytes` under `policy`, borrowing when nothing has to be escaped.
pub fn text(bytes: &[u8], policy: Escape) -> Cow<'_, str> {
    if let Ok(text) = core::str::from_utf8(bytes)
        && (policy == Escape::Verbatim || !needs_escaping(text))
    {
        return Cow::Borrowed(text);
    }
    Cow::Owned(owned(bytes, policy))
}

/// Owned text for `bytes` under `policy`.
///
/// Used where the bytes are reconstructed rather than borrowed (punycode and
/// word-substituted identifiers, decoded payloads).
pub fn owned(bytes: &[u8], policy: Escape) -> String {
    let mut text = String::with_capacity(bytes.len());
    // Writing into a `String` cannot fail.
    let _ = write_escaped(&mut text, bytes, policy);
    text
}

/// True when [`Escape::Identifier`] has to rewrite this valid UTF-8 text.
fn needs_escaping(text: &str) -> bool {
    text.chars()
        .any(|character| character == '\\' || character.is_control())
}
