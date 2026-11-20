//! Pure-Rust Swift symbol demangling.
//!
//! The demangler is a postfix operator machine that builds an `Rc`-owned node
//! graph, followed by a printer that writes readable text into any
//! [`core::fmt::Write`] sink. Parsed graphs are retained so callers can render
//! them repeatedly without re-parsing. Shared nodes make the graph a DAG, so
//! printing is bounded by depth, node-visit and output-byte budgets.
//!
//! Reference: `refs/swift/Mangling.rst` (vendored, pinned upstream commit).
//!
//! # Supported prefixes
//!
//! - `$s` — stable Swift mangling
//! - `$S` — Swift 4.2
//! - `_T0` — Swift 4.0
//!
//! `$e` (Embedded Swift), `@__swiftmacro_` (file names) and the legacy `_T`
//! prefixes are rejected for now, as are symbolic references (control bytes),
//! which the ABI requires tools to refuse on untrusted input.

pub(crate) mod node;
mod parser;
mod printer;

use core::fmt;

use parser::Parser;
use printer::Printer;

/// Recognise a supported Swift mangling prefix, returning its length.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    if symbol.starts_with("$s") || symbol.starts_with("$S") {
        Some(2)
    } else if symbol.starts_with("_T0") {
        Some(3)
    } else {
        None
    }
}

/// True when `symbol` starts with a Swift mangling prefix we recognise.
#[must_use]
pub fn is_swift(symbol: &str) -> bool {
    prefix_len(symbol).is_some()
}

/// A parsed Swift symbol.
///
/// The node graph is retained and can be printed more than once without parsing
/// the mangled name again. It is `Rc`-owned, so `Parsed` is `!Send + !Sync`.
#[derive(Debug)]
pub(crate) struct Parsed<'a> {
    root: node::NodeRef<'a>,
}

impl Parsed<'_> {
    /// Render the retained graph into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        Printer::new(out).print(self.root.clone())
    }
}

fn parse_inner(symbol: &str) -> Result<Parsed<'_>, fmt::Error> {
    let Some(prefix) = prefix_len(symbol) else {
        return Err(fmt::Error);
    };
    let rest = symbol.get(prefix..).ok_or(fmt::Error)?;
    // Symbolic references start with control bytes; they must not be
    // interpreted (ABI requirement for untrusted input).
    if rest.bytes().any(|byte| byte < 0x20) || rest.is_empty() {
        return Err(fmt::Error);
    }
    let mut parser = Parser::new(rest.as_bytes(), symbol.starts_with("_T"));
    let root = parser.parse().map_err(|_| fmt::Error)?;
    Ok(Parsed { root })
}

/// Parse `symbol` into a reusable graph and check that it is renderable.
pub(crate) fn parse(symbol: &str) -> Result<Parsed<'_>, fmt::Error> {
    let parsed = parse_inner(symbol)?;
    parsed.print(&mut NullSink)?;
    Ok(parsed)
}

/// Demangle `symbol` into `out`.
///
/// # Errors
///
/// Returns [`fmt::Error`] when the symbol is not Swift, uses an unsupported
/// prefix, contains symbolic references, or fails to parse. The sink may also
/// fail.
pub fn print<W: fmt::Write + ?Sized>(symbol: &str, out: &mut W) -> fmt::Result {
    parse_inner(symbol)?.print(out)
}

/// Validation sink used by [`parse`].
struct NullSink;

impl fmt::Write for NullSink {
    fn write_str(&mut self, _s: &str) -> fmt::Result {
        Ok(())
    }
}
