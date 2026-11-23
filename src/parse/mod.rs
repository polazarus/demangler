//! Shared raw-byte cursor, budget, escape and combinator layer.
//!
//! Every byte-native backend parses through these primitives:
//!
//! - [`Cursor`] reads a bounded byte slice, tracks offsets and spans, and
//!   restores position and depth through [`Checkpoint`];
//! - [`Limits`] and [`State`] charge the input, depth, work and node budgets
//!   that keep untrusted symbol names bounded;
//! - [`Escape`] and [`write_escaped`] render identifier or payload bytes
//!   reversibly (`\\` for a literal backslash, `\xNN` for an invalid byte);
//! - [`LimitedWriter`] caps how many bytes a printer may emit;
//! - [`Parser`] and the combinators implement backtracking ([`alt`]) versus
//!   committed ([`cut`]) failure.
//!
//! Backends keep their own grammar, public error enum, error mapping and
//! printer; only the cursor, budgets, escaping, output cap and combinator
//! vocabulary are shared.
//!
//! `write_escaped` is compiled even when every backend feature is disabled
//! because [`crate::Demangled`] renders raw identifier bytes through it. The
//! core-only build therefore keeps this module and allows its unused items
//! instead of gating the module away.

#![cfg_attr(
    not(any(
        feature = "cpp-itanium",
        feature = "cpp-msvc",
        feature = "cpp-gnu-v2",
        feature = "rust",
        feature = "swift",
        feature = "go",
        feature = "objc",
        feature = "kotlin-native",
        feature = "d",
        feature = "jni",
        feature = "ada",
        feature = "fortran",
        feature = "delphi",
        feature = "ocaml"
    )),
    allow(dead_code)
)]

mod combinator;
mod cursor;
mod error;
mod escape;
mod output;
mod parser;
mod span;

// Backends are feature-gated, so any given feature subset leaves some of these
// re-exports unused; the core-only build is covered by the module attribute.
#[cfg(feature = "rust")]
#[allow(unused_imports)]
pub(crate) use combinator::many0;
#[allow(unused_imports)]
pub(crate) use combinator::{decimal_length, tag};
#[allow(unused_imports)]
pub(crate) use cursor::{Checkpoint, Cursor, Limits, State};
#[allow(unused_imports)]
pub(crate) use error::{Fail, FailKind, FailMode};
#[allow(unused_imports)]
pub(crate) use escape::{Escape, Ident, write_escaped};
#[allow(unused_imports)]
pub(crate) use output::LimitedWriter;
#[allow(unused_imports)]
pub(crate) use parser::{PResult, Parser};
#[allow(unused_imports)]
pub(crate) use span::Span;

#[cfg(test)]
mod tests;
