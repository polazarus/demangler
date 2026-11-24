//! Text for Kotlin/Native symbol bytes.
//!
//! Symbol names are byte strings, not UTF-8: a Kotlin identifier is encoded
//! with its UTF-8 bytes, but a linker symbol may also carry bytes that are not
//! part of a valid sequence. Every Kotlin/Native name component is an
//! identifier, so this backend asks the shared [`crate::parse::text`] helper for
//! `Escape::Identifier` text: valid UTF-8 borrows the symbol, and anything else
//! is owned text with each invalid byte spelled `\xNN`.

pub(crate) use crate::parse::text;
