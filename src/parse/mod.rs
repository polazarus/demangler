//! Prototype raw-byte cursor and combinator layer.
//!
//! Private while backends migrate. Tests demonstrate byte-length parsing,
//! invalid UTF-8 identifiers, backtracking and budget behavior.

#![allow(dead_code)]

mod combinator;
mod cursor;
mod error;
mod escape;
mod parser;
mod span;

#[allow(unused_imports)]
pub(crate) use combinator::{
    alt, cut, decimal_length, delimited, eof, ident, opt, peek, preceded, take, take_while, tag,
};
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "rust",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native"
))]
#[allow(unused_imports)]
pub(crate) use combinator::many0;
#[allow(unused_imports)]
pub(crate) use cursor::{Checkpoint, Cursor, Limits, State};
pub(crate) use error::{Fail, FailMode};
#[allow(unused_imports)]
pub(crate) use escape::{Escape, Ident, write_escaped};
pub(crate) use parser::{PResult, Parser};
pub(crate) use span::Span;

#[cfg(test)]
mod tests;
