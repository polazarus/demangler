//! Explicit decoding of reversible identifier escapes.
//!
//! Symbol names are byte strings. Backends parse those bytes directly and
//! render identifier text with the crate's reversible escape policy: a
//! literal backslash becomes `\\`, and a byte that is not valid UTF-8 in
//! context becomes `\xNN`. This module turns that rendered text back into
//! bytes and decodes them with a caller-selected legacy encoding.
//!
//! The layer is deliberately opt-in. [`crate::Demangler::demangle`] and
//! [`crate::Demangler::demangle_bytes`] never transcode: they preserve the
//! escape spelling so the result is reversible and encoding-agnostic.
//!
//! # Supported encodings
//!
//! - [`Encoding::Utf8`]: an escaped byte run is decoded as UTF-8. A run that
//!   is not valid UTF-8 is rejected.
//! - [`Encoding::Latin1`]: ISO-8859-1. Every byte maps to the code point of
//!   the same value (`0xE9` -> `é`).
//! - [`Encoding::Cp1252`]: Windows-1252. Bytes `0x80..=0x9F` use the Windows
//!   table (`0x80` -> `€`, `0x92` -> `’`); `0xA0..=0xFF` match Latin-1. The
//!   five bytes Windows leaves undefined (`0x81`, `0x8D`, `0x8F`, `0x90`,
//!   `0x9D`) are rejected.
//!
//! # Rejected encodings
//!
//! Shift-JIS, EUC-JP, MacRoman, EBCDIC, UTF-16, UTF-32 and arbitrary Windows
//! code pages are **not** supported. They need a mapping table this crate does
//! not carry; convert the symbol bytes to UTF-8 before demangling, or decode
//! the escaped text with an external codec. No byte-order mark is recognised
//! and no replacement character is emitted: an unrepresentable byte is an
//! error.
//!
//! # Scope
//!
//! Decoding is a text-level post-pass over the rendered output. It does not
//! change a backend AST, and it cannot know which fragments a backend emitted
//! with `Escape::Verbatim` (compiler payload text) rather than the reversible
//! identifier policy. A verbatim payload that literally contains `\xNN` is
//! therefore indistinguishable from an escape. Identifier text is always
//! escaped reversibly, which is the intended input.
//!
//! Plain C and unknown names are already Unicode and are not escaped;
//! [`crate::Demangled::to_string_with_encoding`] returns them unchanged.
//!
//! # Example
//!
//! ```
//! use demangle::Encoding;
//!
//! let text = Encoding::Latin1
//!     .decode_identifier_escapes("F\\xe9oo")
//!     .unwrap();
//! assert_eq!(text, "Féoo");
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::error::Error;

/// A character encoding for the bytes carried by reversible `\xNN` escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-8. Escaped byte runs must be valid UTF-8.
    Utf8,
    /// ISO-8859-1 (Latin-1). Every byte maps to the same code point.
    Latin1,
    /// Windows-1252. `0x80..=0x9F` use the Windows table; the rest match
    /// Latin-1.
    Cp1252,
}

impl Encoding {
    /// Decode the crate's reversible identifier escapes in `text`.
    ///
    /// `\\` becomes a literal backslash and each `\xNN` escape becomes the
    /// byte `NN`. Consecutive escapes are decoded as one byte run with
    /// `self`, so a UTF-8 sequence can be reconstructed from escaped bytes.
    /// Text outside an escape is already Unicode and is copied unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`EncodingError::InvalidEscape`] when a backslash is followed
    /// by anything other than `\` or `x` plus two hexadecimal digits, and
    /// [`EncodingError::InvalidEncoding`] when the escaped bytes are not valid
    /// in `self`.
    pub fn decode_identifier_escapes(self, text: &str) -> Result<String, EncodingError> {
        let bytes = text.as_bytes();
        let mut decoded = String::with_capacity(text.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'\\' {
                let Some(character) = text[index..].chars().next() else {
                    return Err(EncodingError::InvalidEscape);
                };
                decoded.push(character);
                index += character.len_utf8();
                continue;
            }
            if bytes.get(index + 1) == Some(&b'\\') {
                decoded.push('\\');
                index += 2;
                continue;
            }
            let mut escaped = Vec::new();
            while bytes.get(index) == Some(&b'\\') && bytes.get(index + 1) == Some(&b'x') {
                let Some(high) = bytes.get(index + 2).copied().and_then(hex_digit) else {
                    return Err(EncodingError::InvalidEscape);
                };
                let Some(low) = bytes.get(index + 3).copied().and_then(hex_digit) else {
                    return Err(EncodingError::InvalidEscape);
                };
                escaped.push((high << 4) | low);
                index += 4;
            }
            if escaped.is_empty() {
                // A backslash that is neither `\\` nor `\xNN`.
                return Err(EncodingError::InvalidEscape);
            }
            decode_run(&escaped, self, &mut decoded)?;
        }
        Ok(decoded)
    }
}

/// Why identifier escapes could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodingError {
    /// The symbol could not be demangled before decoding.
    Demangle(Error),
    /// A backslash sequence is neither `\\` nor `\xNN` with two hex digits.
    InvalidEscape,
    /// The escaped bytes are not valid in the selected encoding.
    InvalidEncoding,
}

impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Demangle(error) => write!(f, "{error}"),
            Self::InvalidEscape => f.write_str("invalid \\xNN escape"),
            Self::InvalidEncoding => {
                f.write_str("escaped bytes are not valid in the selected encoding")
            }
        }
    }
}

impl core::error::Error for EncodingError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Demangle(error) => Some(error),
            Self::InvalidEscape | Self::InvalidEncoding => None,
        }
    }
}

impl From<Error> for EncodingError {
    fn from(error: Error) -> Self {
        Self::Demangle(error)
    }
}

/// Decode one run of escaped bytes with `encoding`.
fn decode_run(bytes: &[u8], encoding: Encoding, out: &mut String) -> Result<(), EncodingError> {
    match encoding {
        Encoding::Utf8 => {
            let text = core::str::from_utf8(bytes).map_err(|_| EncodingError::InvalidEncoding)?;
            out.push_str(text);
        }
        Encoding::Latin1 => {
            for byte in bytes {
                out.push(char::from(*byte));
            }
        }
        Encoding::Cp1252 => {
            for byte in bytes {
                out.push(cp1252(*byte).ok_or(EncodingError::InvalidEncoding)?);
            }
        }
    }
    Ok(())
}

/// One hexadecimal digit.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// The Windows-1252 mapping for one byte.
fn cp1252(byte: u8) -> Option<char> {
    match byte {
        0x80 => Some('\u{20ac}'),
        0x81 | 0x8d | 0x8f | 0x90 | 0x9d => None,
        0x82 => Some('\u{201a}'),
        0x83 => Some('\u{0192}'),
        0x84 => Some('\u{201e}'),
        0x85 => Some('\u{2026}'),
        0x86 => Some('\u{2020}'),
        0x87 => Some('\u{2021}'),
        0x88 => Some('\u{02c6}'),
        0x89 => Some('\u{2030}'),
        0x8a => Some('\u{0160}'),
        0x8b => Some('\u{2039}'),
        0x8c => Some('\u{0152}'),
        0x8e => Some('\u{017d}'),
        0x91 => Some('\u{2018}'),
        0x92 => Some('\u{2019}'),
        0x93 => Some('\u{201c}'),
        0x94 => Some('\u{201d}'),
        0x95 => Some('\u{2022}'),
        0x96 => Some('\u{2013}'),
        0x97 => Some('\u{2014}'),
        0x98 => Some('\u{02dc}'),
        0x99 => Some('\u{2122}'),
        0x9a => Some('\u{0161}'),
        0x9b => Some('\u{203a}'),
        0x9c => Some('\u{0153}'),
        0x9e => Some('\u{017e}'),
        0x9f => Some('\u{0178}'),
        _ => Some(char::from(byte)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_latin1_and_cp1252() {
        assert_eq!(
            Encoding::Latin1
                .decode_identifier_escapes("F\\xe9\\xffoo")
                .unwrap(),
            "Féÿoo"
        );
        assert_eq!(
            Encoding::Cp1252
                .decode_identifier_escapes("\\x80\\x92\\xe9")
                .unwrap(),
            "€’é"
        );
        assert_eq!(
            Encoding::Latin1
                .decode_identifier_escapes("\\x80\\x92")
                .unwrap(),
            "\u{80}\u{92}"
        );
    }

    #[test]
    fn decodes_utf8_runs() {
        assert_eq!(
            Encoding::Utf8
                .decode_identifier_escapes("F\\xc3\\xa9o")
                .unwrap(),
            "Féo"
        );
        assert_eq!(
            Encoding::Utf8
                .decode_identifier_escapes("\\xe9")
                .unwrap_err(),
            EncodingError::InvalidEncoding
        );
    }

    #[test]
    fn preserves_literal_backslashes() {
        assert_eq!(
            Encoding::Latin1
                .decode_identifier_escapes("a\\\\x41b")
                .unwrap(),
            "a\\x41b"
        );
    }

    #[test]
    fn rejects_malformed_escapes() {
        for text in ["\\", "\\q", "\\x", "\\x4", "\\xGG", "\\X41"] {
            assert_eq!(
                Encoding::Latin1
                    .decode_identifier_escapes(text)
                    .unwrap_err(),
                EncodingError::InvalidEscape,
                "{text:?}"
            );
        }
    }

    #[test]
    fn rejects_undefined_cp1252_bytes() {
        for text in ["\\x81", "\\x8d", "\\x8f", "\\x90", "\\x9d"] {
            assert_eq!(
                Encoding::Cp1252
                    .decode_identifier_escapes(text)
                    .unwrap_err(),
                EncodingError::InvalidEncoding,
                "{text}"
            );
        }
    }
}
