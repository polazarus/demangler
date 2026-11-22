//! C++ ABI symbol parsing.
//!
//! This module exposes an immutable, inspectable syntax tree shared by the
//! Itanium and Microsoft C++ mangling schemes. It is independent of the text
//! backends: [`crate::Demangler`] parses C++ into this AST and prints it
//! in-tree.
//!
//! `parse` dispatches on the mangling prefix: `_Z` (and its underscore
//! variants) selects the Itanium parser, `?` selects the Microsoft parser.
//! `parse_msvc` parses a Microsoft name directly, including the `.?` RTTI
//! type descriptor name form.
//!
//! The Itanium and Microsoft printers render this AST as demangled text.
//! [`crate::Demangler`] uses them directly; `print_msvc` is also exposed for
//! callers that want Microsoft text from a parsed tree.
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
//!
//! let ast = cpp::parse_msvc("?x@@3HA").unwrap();
//! let mut text = String::new();
//! cpp::print_msvc(&ast, &mut text).unwrap();
//! assert_eq!(text, "int x");
//! ```

mod msvc;
mod msvc_printer;
mod node;
mod parser;
mod printer;

use alloc::rc::Rc;
use core::fmt;

pub use node::{Kind, Node, NodeRef};

pub use msvc::parse_msvc;
pub use msvc_printer::{PrintError, PrintErrorKind, print_msvc};

use parser::Parser;

/// Maximum input length accepted by the C++ text and AST paths.
pub(crate) const MAX_INPUT_LEN: usize = 64 * 1024;

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

/// True when `symbol` starts with a Microsoft C++ mangling prefix.
///
/// A Microsoft symbol starts with `?`. The RTTI type descriptor name form
/// starts with `.?` and is accepted by [`parse_msvc`] directly.
#[must_use]
pub fn is_msvc(symbol: &str) -> bool {
    symbol.starts_with('?') || symbol.starts_with(".?")
}

/// The mangling scheme an [`Ast`] was parsed from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// Itanium C++ ABI (`_Z...`).
    Itanium,
    /// Microsoft C++ (`?...`).
    Msvc,
}

/// A parsed C++ symbol with an immutable node graph.
///
/// The graph is `Rc`-owned, so `Ast` is `!Send + !Sync`. Clone the `Ast` to
/// share the graph without re-parsing.
#[derive(Clone)]
pub struct Ast<'a> {
    dialect: Dialect,
    root: NodeRef<'a>,
}

impl<'a> Ast<'a> {
    /// Wrap a parsed Itanium root node.
    pub(crate) fn from_root(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Itanium,
            root,
        }
    }

    /// Wrap a parsed Microsoft root node.
    pub(crate) fn from_root_msvc(root: NodeRef<'a>) -> Self {
        Self {
            dialect: Dialect::Msvc,
            root,
        }
    }

    /// The mangling scheme this tree was parsed from.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The root [`Kind::Encoding`] node.
    #[must_use]
    pub fn root(&self) -> NodeRef<'a> {
        Rc::clone(&self.root)
    }
}

impl fmt::Debug for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ast")
            .field("dialect", &self.dialect)
            .field("root", &self.root)
            .finish()
    }
}

impl fmt::Display for Ast<'_> {
    /// Render the parsed symbol as demangled C++ text.
    ///
    /// The text follows the LLVM demangler's conventions. Printing is bounded
    /// by depth, node-visit and output-byte budgets, so a symbol that parses
    /// but cannot be rendered is reported as an error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.dialect {
            Dialect::Itanium => printer::print(self, f),
            Dialect::Msvc => print_msvc(self, f).map_err(|_| fmt::Error),
        }
    }
}

/// Why a C++ symbol could not be parsed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// The symbol does not start with a supported Itanium prefix.
    NotItanium,
    /// The symbol does not start with a supported Microsoft prefix.
    NotMsvc,
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
            Self::NotMsvc => "not a Microsoft mangled name",
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

/// Parse a C++ mangled name into an inspectable AST.
///
/// Itanium names (a `_Z` prefix, including the underscored variants) use the
/// Itanium parser; names starting with `?` use the Microsoft parser.
///
/// # Errors
///
/// Returns [`ParseError`] when the symbol is not a recognised C++ mangling,
/// uses an unsupported production, is malformed, or exceeds the parser's
/// depth, node or work budgets.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    if let Some(prefix) = prefix_len(symbol) {
        return Parser::new(symbol, prefix)?.parse();
    }
    if symbol.starts_with('?') {
        return parse_msvc(symbol);
    }
    Err(ParseError::new(0, ParseErrorKind::NotItanium))
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
