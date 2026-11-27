//! Text for Go symbol bytes.
//!
//! Symbol names are byte strings, not UTF-8: a package path is
//! `objabi.PathToPrefix`-escaped ASCII, and a type string embeds whatever bytes
//! the compiler emitted. Every Go name component is an identifier, so this
//! backend asks the shared [`crate::parse::text`] helper for
//! [`Escape::Identifier`] text; opaque compiler payloads (`go:string.`,
//! `type:.namedata.`) keep their own quoting and backslashes through
//! [`Escape::Verbatim`].

use alloc::borrow::Cow;

pub use crate::parse::text;

use crate::parse::{Escape, owned};

/// Text for `bytes` with `%XX` escapes decoded.
///
/// The decoded bytes are escaped like an identifier, so a `%ff` escape prints
/// as `\xff` instead of falling back to the raw spelling.
pub fn decoded(bytes: &[u8]) -> Cow<'_, str> {
    match unescape(bytes) {
        Cow::Borrowed(raw) => text(raw, Escape::Identifier),
        Cow::Owned(decoded) => Cow::Owned(owned(&decoded, Escape::Identifier)),
    }
}

/// Decode `%XX` escapes, as produced by Go's `objabi.PathToPrefix`.
///
/// A `%` that is not followed by two hex digits is kept verbatim. The decoded
/// bytes are returned as they are: they are not required to be UTF-8.
pub fn unescape(bytes: &[u8]) -> Cow<'_, [u8]> {
    if !bytes.contains(&b'%') {
        return Cow::Borrowed(bytes);
    }

    let mut decoded = alloc::vec::Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).and_then(|byte| hex_value(*byte)),
                bytes.get(i + 2).and_then(|byte| hex_value(*byte)),
            )
        {
            decoded.push((high << 4) | low);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    Cow::Owned(decoded)
}

/// The value of one hexadecimal digit.
const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
