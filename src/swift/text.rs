//! Byte-to-text conversion for Swift node text.
//!
//! A Swift mangling is a byte string: an identifier length is a byte count, so
//! a malformed symbol can slice a UTF-8 sequence in half. The parsers keep
//! bytes; this module is the only place that turns them into the `str` node
//! text the printer writes.
//!
//! Two policies are used, chosen per site:
//!
//! - [`Escape::Identifier`] for identifier text: a slice that is not valid
//!   UTF-8 becomes owned text with each invalid byte spelled `\xNN`, and a
//!   backslash inside that escaped text is doubled so the spelling stays
//!   reversible.
//! - [`Escape::Verbatim`] for compiler payloads (unmangled suffixes and nested
//!   legacy payload text): a slice that is not valid UTF-8 becomes owned text
//!   with each invalid byte spelled `\xNN`, but every valid byte — including a
//!   backslash — is kept exactly as the compiler wrote it.
//!
//! Valid UTF-8 always borrows and prints unchanged — even when it contains a
//! backslash — so every symbol the demangler accepted before this module
//! existed renders byte-identically; the two policies differ only inside text
//! that had to be escaped.

use alloc::{borrow::Cow, string::String};

use crate::parse::{Escape, write_escaped};

/// Borrow `bytes` when they are valid UTF-8, otherwise escape them.
pub(crate) fn cow<'a>(bytes: &'a [u8], policy: Escape) -> Cow<'a, str> {
    core::str::from_utf8(bytes).map_or_else(|_| Cow::Owned(owned(bytes, policy)), Cow::Borrowed)
}

/// Escape `bytes` into owned text.
///
/// Used where the bytes are reconstructed rather than borrowed (punycode and
/// word-substituted identifiers).
pub(crate) fn owned(bytes: &[u8], policy: Escape) -> String {
    let mut text = String::new();
    // Writing into a `String` is infallible.
    let _ = write_escaped(&mut text, bytes, policy);
    text
}
