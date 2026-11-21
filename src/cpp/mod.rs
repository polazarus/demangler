//! Itanium C++ ABI symbol parsing.
//!
//! This module exposes an immutable, inspectable syntax tree for Itanium
//! mangled names. It is independent of the text backends: [`crate::Demangler`]
//! still uses `itanium-demangle` for display text, and this AST is not wired
//! into [`crate::Demangled`] yet.
//!
//! The tree is `Rc`-owned and therefore `!Send + !Sync`. Nodes are immutable
//! after construction, children are sealed, and substitution references reuse
//! the same node handle, so the graph may be a DAG.
//!
//! # Supported grammar
//!
//! The parser covers the core Itanium productions:
//!
//! - prefixes `_Z`, `__Z`, `___Z`, `____Z`;
//! - encodings, function parameter lists and data names;
//! - unscoped, nested, template-id and well-known substituted names;
//! - source names, operator names, constructors/destructors and ABI tags;
//! - local names, lambdas and unnamed types;
//! - builtin, pointer, reference, rvalue-reference, cv-qualified, array,
//!   function, member-pointer, `decltype` and vendor-qualified types;
//! - template arguments, template parameters and expression literals;
//! - special names: vtables, VTTs, typeinfo, thunks, guard variables,
//!   reference temporaries and TLS wrappers/initializers;
//! - substitutions and back-references.
//!
//! Builtins include the single-letter codes plus parametrized `_BitInt`,
//! `_Float`, `bfloat16` and fixed-point types.
//!
//! The long tail of the grammar is still rejected rather than silently
//! accepted or truncated. Rejections are usually `ParseErrorKind::Unsupported`,
//! but an unrecognised byte in a name or type position is
//! `ParseErrorKind::UnexpectedByte`.
//!
//! # Example
//!
//! ```rust
//! use demangle::cpp::{self, Kind};
//!
//! let ast = cpp::parse("_ZN3Foo3barEv").unwrap();
//! let encoding = ast.root();
//! assert_eq!(encoding.kind(), Kind::Encoding);
//! ```

mod node;
mod parser;

use alloc::rc::Rc;
use core::fmt;

pub use node::{Kind, Node, NodeRef};

use parser::Parser;

/// Length of a recognised Itanium mangling prefix.
///
/// The longest matching prefix wins: `____Z`, `___Z`, `__Z`, then `_Z`.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    if symbol.starts_with("____Z") {
        Some(5)
    } else if symbol.starts_with("___Z") {
        Some(4)
    } else if symbol.starts_with("__Z") {
        Some(3)
    } else if symbol.starts_with("_Z") {
        Some(2)
    } else {
        None
    }
}

/// True when `symbol` starts with an Itanium mangling prefix.
#[must_use]
pub fn is_itanium(symbol: &str) -> bool {
    prefix_len(symbol).is_some()
}

/// A parsed Itanium symbol with an immutable node graph.
///
/// The graph is `Rc`-owned, so `Ast` is `!Send + !Sync`. Clone the `Ast` to
/// share the graph without re-parsing.
#[derive(Clone)]
pub struct Ast<'a> {
    root: NodeRef<'a>,
}

impl<'a> Ast<'a> {
    /// Wrap a parsed root node.
    pub(crate) fn from_root(root: NodeRef<'a>) -> Self {
        Self { root }
    }

    /// The root [`Kind::Encoding`] node.
    #[must_use]
    pub fn root(&self) -> NodeRef<'a> {
        Rc::clone(&self.root)
    }
}

impl fmt::Debug for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ast").field("root", &self.root).finish()
    }
}

/// Why an Itanium symbol could not be parsed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The symbol does not start with a supported Itanium prefix.
    NotItanium,
    /// The input ended in the middle of a production.
    UnexpectedEnd,
    /// A byte was not valid at this position.
    UnexpectedByte,
    /// A source-name length or sequence id was malformed or out of range.
    InvalidLength,
    /// A substitution referenced an entry that does not exist yet.
    UnknownSubstitution,
    /// The production is valid Itanium but not implemented yet.
    Unsupported,
    /// The nesting budget was exceeded.
    TooDeep,
    /// The node budget was exceeded.
    TooManyNodes,
    /// The work budget was exceeded.
    TooMuchWork,
    /// The symbol is too long for `u32` spans.
    TooLong,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotItanium => "not an Itanium mangled name",
            Self::UnexpectedEnd => "unexpected end of input",
            Self::UnexpectedByte => "unexpected byte",
            Self::InvalidLength => "invalid length",
            Self::UnknownSubstitution => "unknown substitution",
            Self::Unsupported => "unsupported production",
            Self::TooDeep => "nesting budget exceeded",
            Self::TooManyNodes => "node budget exceeded",
            Self::TooMuchWork => "work budget exceeded",
            Self::TooLong => "symbol too long",
        };
        f.write_str(message)
    }
}

/// A parse failure with the byte offset where it was detected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParseError {
    position: usize,
    kind: ParseErrorKind,
}

impl ParseError {
    /// Create an error at `position`.
    pub(crate) const fn new(position: usize, kind: ParseErrorKind) -> Self {
        Self { position, kind }
    }

    /// Byte offset in the input where parsing failed.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// What went wrong.
    #[must_use]
    pub const fn kind(&self) -> ParseErrorKind {
        self.kind
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.kind, self.position)
    }
}

impl core::error::Error for ParseError {}

/// Parse an Itanium C++ mangled name into an inspectable AST.
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not Itanium, uses an unsupported
/// production, is malformed, or exceeds the parser's depth, node or work
/// budgets.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    let prefix =
        prefix_len(symbol).ok_or_else(|| ParseError::new(0, ParseErrorKind::NotItanium))?;
    Parser::new(symbol, prefix)?.parse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_prefixes() {
        assert_eq!(prefix_len("_Z1fv"), Some(2));
        assert_eq!(prefix_len("__Z1fv"), Some(3));
        assert_eq!(prefix_len("___Z1fv"), Some(4));
        assert_eq!(prefix_len("____Z1fv"), Some(5));
        assert_eq!(prefix_len("_Z"), Some(2));
        assert_eq!(prefix_len("main"), None);
        assert!(is_itanium("_ZN3Foo3barEv"));
        assert!(!is_itanium("$s4main3foo"));
    }

    #[test]
    fn rejects_non_itanium() {
        let error = parse("main").unwrap_err();
        assert_eq!(error.kind(), ParseErrorKind::NotItanium);
        assert_eq!(error.position(), 0);
    }

    #[test]
    fn root_is_an_encoding() {
        let ast = parse("_Z1A").unwrap();
        assert_eq!(ast.root().kind(), Kind::Encoding);
        assert_eq!(ast.root().child_count(), 1);
        assert_eq!(ast.root().child(0).unwrap().kind(), Kind::Data);
    }

    #[test]
    fn ast_clone_shares_the_root() {
        let ast = parse("_Z1fv").unwrap();
        let cloned = ast.clone();
        assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
    }

    #[test]
    fn ast_debug_is_bounded() {
        let ast = parse("_Z1fPPPPi").unwrap();
        let rendered = alloc::format!("{ast:?}");
        assert!(rendered.contains("Ast"));
        assert!(rendered.contains("Encoding"));
        assert!(!rendered.contains("BuiltinType"));
    }
}
