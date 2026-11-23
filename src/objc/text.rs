//! Text for Objective-C symbol bytes.
//!
//! Symbol names are byte strings, not UTF-8: a class name in a Mach-O symbol
//! table is a C string and may hold any byte. Text that is valid UTF-8 is
//! returned verbatim; bytes that are not valid UTF-8 are escaped as `\xNN`, so
//! the rendered name never contains a broken byte sequence.

use alloc::borrow::Cow;
use alloc::string::String;

use crate::parse::{Escape, write_escaped};

/// Text for `bytes`, escaped according to `policy`.
pub(crate) fn text(bytes: &[u8], policy: Escape) -> Cow<'_, str> {
    if let Ok(text) = core::str::from_utf8(bytes)
        && (policy == Escape::Verbatim || !text.contains('\\'))
    {
        return Cow::Borrowed(text);
    }
    Cow::Owned(escape(bytes, policy))
}

/// Escape `bytes` into an owned string.
fn escape(bytes: &[u8], policy: Escape) -> String {
    let mut out = String::new();
    let written = write_escaped(&mut out, bytes, policy);
    // Writing into a `String` cannot fail; the result only reports `fmt` errors.
    debug_assert!(written.is_ok(), "writing into a String cannot fail");
    out
}
