//! Shared raw-byte cursor, budget, search, escape and combinator layer.
//!
//! Every byte-native backend parses through these primitives:
//!
//! - [`Cursor`] reads a bounded byte slice, tracks offsets and spans, and
//!   restores position and depth through [`Checkpoint`];
//! - [`Limits`] and [`State`] charge the input, depth and work budgets that
//!   keep untrusted symbol names bounded (AST node budgets stay with the
//!   building backend);
//! - [`find`] and [`rfind`] locate a marker inside a byte slice;
//! - [`text`] and [`owned`] turn identifier or payload bytes into the node
//!   text a printer writes, escaping them under an explicit [`Escape`] policy;
//! - [`write_escaped`] renders those bytes reversibly (`\\` for a literal
//!   backslash, `\xNN` for an invalid byte);
//! - [`LimitedWriter`] is a test-only sink for asserting output bounds;
//! - [`Parser`] and the combinators carry the [`FailMode`] backtracking
//!   (`Backtrack`) versus committed (`Cut`) failure vocabulary.
//!
//! Backends keep their own grammar, public error enum, error mapping and
//! printer; only the cursor, budgets, search, text, escaping, output cap and
//! combinator vocabulary are shared.
//!
//! `write_escaped` is compiled even when every backend feature is disabled
//! because [`crate::Demangled`] renders raw identifier bytes through it. The
//! core-only and single-backend builds therefore keep this module and allow
//! its unused items instead of gating the module away; the all-features build
//! enforces the real inventory.

#![cfg_attr(
    not(all(
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
#[cfg(test)]
mod output;
mod parser;
mod search;
mod span;
// The text helpers need `alloc`, which the core-only build does not link; only
// the node-building backends below ask for them.
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native"
))]
mod text;

// Backends are feature-gated, so any given feature subset leaves some of these
// re-exports unused; the core-only build is covered by the module attribute.
#[cfg(feature = "rust")]
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use combinator::many0;
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use combinator::{decimal_length, tag};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use cursor::{Checkpoint, Cursor, Limits, State};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use error::{Fail, FailKind, FailMode};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use escape::{Escape, write_escaped};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
#[cfg(test)]
pub(crate) use output::LimitedWriter;
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use parser::{PResult, Parser};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use search::{find, rfind};
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use span::Span;
#[cfg(any(
    feature = "cpp-itanium",
    feature = "cpp-msvc",
    feature = "cpp-gnu-v2",
    feature = "swift",
    feature = "go",
    feature = "objc",
    feature = "kotlin-native"
))]
#[allow(
    unused_imports,
    reason = "each backend feature uses a different subset"
)]
pub(crate) use text::{owned, text};

#[cfg(test)]
mod tests;
