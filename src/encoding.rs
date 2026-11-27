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
//! - [`Encoding::Windows1252`]: Windows-1252. Bytes `0x80..=0x9F` use the Windows
//!   table (`0x80` -> `€`, `0x92` -> `’`); `0xA0..=0xFF` match Latin-1. The
//!   five bytes Windows leaves undefined (`0x81`, `0x8D`, `0x8F`, `0x90`,
//!   `0x9D`) are rejected.
//! - [`Encoding::MacRoman`]: the classic Mac OS Roman table. Bytes
//!   `0x00..=0x7F` are ASCII and `0x80..=0xFF` use Apple's Roman table
//!   (`0x8E` -> `é`, `0xA5` -> `•`, `0xF0` -> the Apple logo `U+F8FF`). Every
//!   byte is defined.
//! - [`Encoding::Ebcdic037`]: EBCDIC code page 037, the US/Canada EBCDIC page
//!   (`0x81` -> `a`, `0xC1` -> `A`, `0xF0` -> `0`, `0x40` -> space). Every byte
//!   is defined; the `0x00..=0x3F` range decodes to C0 control characters.
//!
//! # Rejected encodings
//!
//! Shift-JIS, EUC-JP, other Windows code pages and EBCDIC code pages other
//! than 037 are **not** supported. A Shift-JIS table is a multi-thousand-entry
//! double-byte mapping; this crate carries no such table and no runtime codec
//! dependency, so a Shift-JIS symbol name must be converted to UTF-8 before
//! demangling or decoded with an external codec. EBCDIC pages differ per
//! country and encoding a name in the wrong page silently changes it, so only
//! the default US/Canada page is claimed.
//!
//! UTF-16 and UTF-32 are rejected as well, with a reason that is not about
//! table size: a symbol table stores NUL-terminated byte strings, so a UTF-16
//! name has no byte-order mark to read and no defined code-unit alignment
//! after the escapes were rendered. Guessing an endianness would be a silent
//! corruption, so no UTF-16 mode exists.
//!
//! No byte-order mark is recognised and no replacement character is emitted:
//! an unrepresentable byte is an error.
//!
//! # Tables and provenance
//!
//! `MAC_ROMAN_HIGH` and `EBCDIC_037` are `const` data, so the module stays
//! dependency-free and `no_std`. They were generated with CPython 3.13.5's
//! `mac_roman` and `cp037` codecs, which implement the Unicode Consortium
//! mapping files:
//!
//! - MacRoman: `VENDORS/APPLE/ROMAN.TXT` from the Unicode `MAPPINGS` tree
//!   (<https://www.unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT>);
//! - EBCDIC 037: `VENDORS/MICSFT/EBCDIC/CP037.TXT` from the same tree
//!   (<https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/EBCDIC/CP037.TXT>),
//!   the same mapping as IBM's `ibm-037` code page.
//!
//! Both tables are injective and contain no surrogate code points; the unit
//! tests pin that property together with the distinctive mappings. Nothing is
//! fetched at build time and no codec crate is involved.
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
//! use demangler::Encoding;
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
    Windows1252,
    /// Classic Mac OS Roman. `0x00..=0x7F` is ASCII and `0x80..=0xFF` is
    /// Apple's Roman table.
    MacRoman,
    /// EBCDIC code page 037 (US/Canada). Every byte maps to one code point.
    Ebcdic037,
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
        let mut escaped = Vec::new();
        while index < bytes.len() {
            if bytes[index] != b'\\' {
                // Copy the whole non-escape run in one push.
                let next = text[index..]
                    .find('\\')
                    .map_or(bytes.len(), |offset| index + offset);
                decoded.push_str(&text[index..next]);
                index = next;
                continue;
            }
            if bytes.get(index + 1) == Some(&b'\\') {
                decoded.push('\\');
                index += 2;
                continue;
            }
            escaped.clear();
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
        Encoding::Windows1252 => {
            for byte in bytes {
                out.push(cp1252(*byte).ok_or(EncodingError::InvalidEncoding)?);
            }
        }
        Encoding::MacRoman => {
            for byte in bytes {
                out.push(mac_roman(*byte));
            }
        }
        Encoding::Ebcdic037 => {
            for byte in bytes {
                out.push(EBCDIC_037[usize::from(*byte)]);
            }
        }
    }
    Ok(())
}

/// The MacRoman mapping for one byte.
fn mac_roman(byte: u8) -> char {
    if byte < 0x80 {
        char::from(byte)
    } else {
        MAC_ROMAN_HIGH[usize::from(byte - 0x80)]
    }
}

/// MacRoman code points for the bytes `0x80..=0xFF`; `0x00..=0x7F` is ASCII.
const MAC_ROMAN_HIGH: [char; 128] = [
    '\u{c4}', '\u{c5}', '\u{c7}', '\u{c9}', '\u{d1}', '\u{d6}', '\u{dc}', '\u{e1}', '\u{e0}',
    '\u{e2}', '\u{e4}', '\u{e3}', '\u{e5}', '\u{e7}', '\u{e9}', '\u{e8}', '\u{ea}', '\u{eb}',
    '\u{ed}', '\u{ec}', '\u{ee}', '\u{ef}', '\u{f1}', '\u{f3}', '\u{f2}', '\u{f4}', '\u{f6}',
    '\u{f5}', '\u{fa}', '\u{f9}', '\u{fb}', '\u{fc}', '\u{2020}', '\u{b0}', '\u{a2}', '\u{a3}',
    '\u{a7}', '\u{2022}', '\u{b6}', '\u{df}', '\u{ae}', '\u{a9}', '\u{2122}', '\u{b4}', '\u{a8}',
    '\u{2260}', '\u{c6}', '\u{d8}', '\u{221e}', '\u{b1}', '\u{2264}', '\u{2265}', '\u{a5}',
    '\u{b5}', '\u{2202}', '\u{2211}', '\u{220f}', '\u{3c0}', '\u{222b}', '\u{aa}', '\u{ba}',
    '\u{3a9}', '\u{e6}', '\u{f8}', '\u{bf}', '\u{a1}', '\u{ac}', '\u{221a}', '\u{192}', '\u{2248}',
    '\u{2206}', '\u{ab}', '\u{bb}', '\u{2026}', '\u{a0}', '\u{c0}', '\u{c3}', '\u{d5}', '\u{152}',
    '\u{153}', '\u{2013}', '\u{2014}', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{f7}',
    '\u{25ca}', '\u{ff}', '\u{178}', '\u{2044}', '\u{20ac}', '\u{2039}', '\u{203a}', '\u{fb01}',
    '\u{fb02}', '\u{2021}', '\u{b7}', '\u{201a}', '\u{201e}', '\u{2030}', '\u{c2}', '\u{ca}',
    '\u{c1}', '\u{cb}', '\u{c8}', '\u{cd}', '\u{ce}', '\u{cf}', '\u{cc}', '\u{d3}', '\u{d4}',
    '\u{f8ff}', '\u{d2}', '\u{da}', '\u{db}', '\u{d9}', '\u{131}', '\u{2c6}', '\u{2dc}', '\u{af}',
    '\u{2d8}', '\u{2d9}', '\u{2da}', '\u{b8}', '\u{2dd}', '\u{2db}', '\u{2c7}',
];

/// EBCDIC code page 037 code points for all 256 bytes.
const EBCDIC_037: [char; 256] = [
    '\u{0}', '\u{1}', '\u{2}', '\u{3}', '\u{9c}', '\u{9}', '\u{86}', '\u{7f}', '\u{97}', '\u{8d}',
    '\u{8e}', '\u{b}', '\u{c}', '\u{d}', '\u{e}', '\u{f}', '\u{10}', '\u{11}', '\u{12}', '\u{13}',
    '\u{9d}', '\u{85}', '\u{8}', '\u{87}', '\u{18}', '\u{19}', '\u{92}', '\u{8f}', '\u{1c}',
    '\u{1d}', '\u{1e}', '\u{1f}', '\u{80}', '\u{81}', '\u{82}', '\u{83}', '\u{84}', '\u{a}',
    '\u{17}', '\u{1b}', '\u{88}', '\u{89}', '\u{8a}', '\u{8b}', '\u{8c}', '\u{5}', '\u{6}',
    '\u{7}', '\u{90}', '\u{91}', '\u{16}', '\u{93}', '\u{94}', '\u{95}', '\u{96}', '\u{4}',
    '\u{98}', '\u{99}', '\u{9a}', '\u{9b}', '\u{14}', '\u{15}', '\u{9e}', '\u{1a}', ' ', '\u{a0}',
    '\u{e2}', '\u{e4}', '\u{e0}', '\u{e1}', '\u{e3}', '\u{e5}', '\u{e7}', '\u{f1}', '\u{a2}', '.',
    '<', '(', '+', '|', '&', '\u{e9}', '\u{ea}', '\u{eb}', '\u{e8}', '\u{ed}', '\u{ee}', '\u{ef}',
    '\u{ec}', '\u{df}', '!', '$', '*', ')', ';', '\u{ac}', '-', '/', '\u{c2}', '\u{c4}', '\u{c0}',
    '\u{c1}', '\u{c3}', '\u{c5}', '\u{c7}', '\u{d1}', '\u{a6}', ',', '%', '_', '>', '?', '\u{f8}',
    '\u{c9}', '\u{ca}', '\u{cb}', '\u{c8}', '\u{cd}', '\u{ce}', '\u{cf}', '\u{cc}', '`', ':', '#',
    '@', '\'', '=', '"', '\u{d8}', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', '\u{ab}', '\u{bb}',
    '\u{f0}', '\u{fd}', '\u{fe}', '\u{b1}', '\u{b0}', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
    '\u{aa}', '\u{ba}', '\u{e6}', '\u{b8}', '\u{c6}', '\u{a4}', '\u{b5}', '~', 's', 't', 'u', 'v',
    'w', 'x', 'y', 'z', '\u{a1}', '\u{bf}', '\u{d0}', '\u{dd}', '\u{de}', '\u{ae}', '^', '\u{a3}',
    '\u{a5}', '\u{b7}', '\u{a9}', '\u{a7}', '\u{b6}', '\u{bc}', '\u{bd}', '\u{be}', '[', ']',
    '\u{af}', '\u{a8}', '\u{b4}', '\u{d7}', '{', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I',
    '\u{ad}', '\u{f4}', '\u{f6}', '\u{f2}', '\u{f3}', '\u{f5}', '}', 'J', 'K', 'L', 'M', 'N', 'O',
    'P', 'Q', 'R', '\u{b9}', '\u{fb}', '\u{fc}', '\u{f9}', '\u{fa}', '\u{ff}', '\\', '\u{f7}', 'S',
    'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '\u{b2}', '\u{d4}', '\u{d6}', '\u{d2}', '\u{d3}', '\u{d5}',
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '\u{b3}', '\u{db}', '\u{dc}', '\u{d9}',
    '\u{da}', '\u{9f}',
];

/// One hexadecimal digit.
const fn hex_digit(byte: u8) -> Option<u8> {
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
mod tests;
