//! Byte-to-text conversion for Swift node text.
//!
//! A Swift mangling is a byte string: an identifier length is a byte count, so
//! a malformed symbol can slice a UTF-8 sequence in half. The parsers keep
//! bytes; the shared [`crate::parse::text`] helper is the only place that turns
//! them into the `str` node text the printer writes.
//!
//! Two policies are used, chosen per site:
//!
//! - `Escape::Identifier` for identifier text: text that a printer would have
//!   to rewrite (a backslash or a control character) becomes owned text with
//!   each invalid byte spelled `\xNN`, so the spelling stays reversible.
//! - `Escape::Verbatim` for compiler payloads (unmangled suffixes and nested
//!   legacy payload text): every valid byte — including a backslash — is kept
//!   exactly as the compiler wrote it, and only invalid bytes are escaped.

pub(crate) use crate::parse::{owned, text as cow};
