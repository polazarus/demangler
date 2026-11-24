//! Text for Objective-C symbol bytes.
//!
//! Symbol names are byte strings, not UTF-8: a class name in a Mach-O symbol
//! table is a C string and may hold any byte. Every Objective-C name component
//! is an identifier, so this backend asks the shared [`crate::parse::text`]
//! helper for `Escape::Identifier` text: the rendered name never contains a
//! broken byte sequence, and a backslash or control character stays reversible.

pub(crate) use crate::parse::text;
