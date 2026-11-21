//! Pure-Rust rustc symbol demangling.
//!
//! Both rustc encodings are parsed into one inspectable tree:
//!
//! - **legacy** `_ZN...E` (also `ZN`, `__ZN`): length-prefixed path elements
//!   with `$`-escapes and a trailing hash;
//! - **v0** `_R...` (also `R`, `__R`): the RFC 2603 grammar, including
//!   backrefs.
//!
//! [`crate::rust::parse`] is strict: it returns [`crate::rust::ParseError`] for anything the in-tree
//! parser cannot represent, and validates that the resulting tree is printable.
//! It accepts a symbol only when `rustc_demangle::try_demangle` would also
//! accept it, plus the documented budgets below.
//!
//! Reference: `refs/rust/` (vendored, pinned upstream commits), documented in
//! `refs/rust.md`. The printer follows `rustc-demangle` byte for byte, because
//! that crate — not the RFC — defines the demangled text.
//!
//! # Inspecting a tree
//!
//! [`crate::rust::Ast`] exposes the encoding, the mangling prefix, the vendor suffix and an
//! `Rc`-owned [`crate::rust::Node`] graph. Children are sealed and shared, so the graph is a
//! DAG and `Ast` is `!Send + !Sync`; demangle again on the target thread
//! instead of moving a tree across threads.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by [`crate::rust::MAX_INPUT_LEN`], [`crate::rust::MAX_DEPTH`],
//! [`crate::rust::MAX_NODES`], [`crate::rust::MAX_WORK`], [`crate::rust::MAX_PRINT_DEPTH`], [`crate::rust::MAX_PRINT_VISITS`] and
//! [`crate::rust::MAX_OUTPUT_BYTES`]. Exceeding a parse budget is an error, never a partial
//! tree.
//!
//! # Splat marker
//!
//! The unstable `w` splat marker is accepted and printed as
//! `#[rustc_splat] `, the spelling of the pinned reference decoder. The
//! locked `rustc-demangle` 0.1.28 still prints `#[splat] `; the corpus gate
//! normalizes that one documented divergence rather than rewriting the
//! expectations.

mod legacy;
mod node;
mod printer;
mod v0;

use core::fmt;

pub use node::{
    AdtConstKind, ConstValueKind, ImplKind, Kind, MAX_DEPTH, MAX_INPUT_LEN, MAX_NODES,
    MAX_OUTPUT_BYTES, MAX_PRINT_DEPTH, MAX_PRINT_VISITS, MAX_WORK, Namespace, Node, NodeData,
    NodeRef, Span,
};

/// Which rustc mangling encoding a symbol uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// Legacy `_ZN...E`: length-prefixed path elements and a trailing hash.
    Legacy,
    /// v0 `_R...`: the RFC 2603 grammar.
    V0,
}

/// Why a symbol could not be parsed into an [`Ast`].
///
/// The variants mirror `rustc-demangle`'s `ParseError`, with
/// [`ParseError::BudgetExceeded`] added for the node and work budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match either grammar.
    Invalid,
    /// The symbol nests deeper than [`MAX_DEPTH`].
    RecursedTooDeep,
    /// The symbol needs more nodes or work than the budgets allow.
    BudgetExceeded,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Invalid => "invalid syntax",
            Self::RecursedTooDeep => "recursion limit reached",
            Self::BudgetExceeded => "size limit reached",
        };
        f.write_str(message)
    }
}

impl core::error::Error for ParseError {}

/// A parsed rustc symbol with a read-only node graph.
///
/// The tree is retained and can be printed more than once. It is `Rc`-owned,
/// so `Ast` is `!Send + !Sync`.
#[derive(Debug, Clone)]
pub struct Ast<'a> {
    encoding: Encoding,
    prefix: &'a str,
    suffix: &'a str,
    root: NodeRef<'a>,
    instantiating_crate: Option<NodeRef<'a>>,
}

impl<'a> Ast<'a> {
    /// The encoding the symbol used.
    #[must_use]
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The mangling prefix, one of `_ZN`, `ZN`, `__ZN`, `_R`, `R`, `__R`.
    #[must_use]
    pub const fn prefix(&self) -> &'a str {
        self.prefix
    }

    /// The vendor suffix printed verbatim after the demangled text.
    ///
    /// Empty for most symbols. A ThinLTO `.llvm.<hex>` suffix is stripped
    /// before parsing and is therefore not reported here, matching
    /// `rustc-demangle`, which drops it from the output.
    #[must_use]
    pub const fn suffix(&self) -> &'a str {
        self.suffix
    }

    /// The root of the tree: the demangled path, or the legacy path.
    #[must_use]
    pub const fn root(&self) -> &NodeRef<'a> {
        &self.root
    }

    /// The v0 instantiating crate, when the symbol carries one.
    ///
    /// It is parsed for validation and never printed, so it is exposed
    /// separately from [`Ast::root`].
    #[must_use]
    pub const fn instantiating_crate(&self) -> Option<&NodeRef<'a>> {
        self.instantiating_crate.as_ref()
    }
}

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let alternate = f.alternate();
        match printer::print(self, f, alternate) {
            Ok(printer::Outcome::Printed) => Ok(()),
            Ok(printer::Outcome::SizeLimit) => f.write_str(printer::SIZE_LIMIT),
            Err(error) => Err(error),
        }
    }
}

/// Parse `symbol` into a reusable tree.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol matches neither encoding,
/// [`ParseError::RecursedTooDeep`] when it nests deeper than [`MAX_DEPTH`] and
/// [`ParseError::BudgetExceeded`] when it exceeds the node, work or input
/// budget. A symbol that parses but cannot be printed is also rejected, so
/// every [`Ast`] is printable.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    if symbol.len() > MAX_INPUT_LEN {
        return Err(ParseError::BudgetExceeded);
    }

    // ThinLTO renames internal symbols; the reference strips those endings
    // before parsing and never prints them back.
    let stripped = strip_thin_lto(symbol);
    let parsed = parse_symbol(stripped)?;
    let suffix = stripped.get(parsed.end()..).unwrap_or("");
    if !suffix.is_empty() && !(suffix.starts_with('.') && is_symbol_like(suffix)) {
        return Err(ParseError::Invalid);
    }

    let prefix = stripped.get(..parsed.prefix_len()).unwrap_or("");
    let (encoding, root, instantiating_crate) = match parsed {
        Parsed::Legacy { root, .. } => (Encoding::Legacy, root, None),
        Parsed::V0 {
            root,
            instantiating_crate,
            ..
        } => (Encoding::V0, root, instantiating_crate),
    };
    let ast = Ast {
        encoding,
        prefix,
        suffix,
        root,
        instantiating_crate,
    };
    printer::validate(&ast)?;
    Ok(ast)
}

/// A successful parse of one encoding, before the suffix rule is applied.
enum Parsed<'a> {
    Legacy {
        prefix_len: usize,
        root: NodeRef<'a>,
        end: usize,
    },
    V0 {
        prefix_len: usize,
        root: NodeRef<'a>,
        instantiating_crate: Option<NodeRef<'a>>,
        end: usize,
    },
}

impl Parsed<'_> {
    /// Byte offset just past the mangled path.
    const fn end(&self) -> usize {
        match self {
            Self::Legacy { end, .. } | Self::V0 { end, .. } => *end,
        }
    }

    /// Length of the encoding's mangling prefix.
    const fn prefix_len(&self) -> usize {
        match self {
            Self::Legacy { prefix_len, .. } | Self::V0 { prefix_len, .. } => *prefix_len,
        }
    }
}

/// Try the legacy front-end, then the v0 one, like `rustc-demangle` does.
fn parse_symbol(symbol: &str) -> Result<Parsed<'_>, ParseError> {
    let mut budget_error = None;
    if let Some(prefix_len) = legacy::prefix_len(symbol) {
        match legacy::parse(symbol) {
            Ok((root, end)) => {
                return Ok(Parsed::Legacy {
                    prefix_len,
                    root,
                    end,
                });
            }
            // Legacy is iterative, so the only recoverable failure is a budget
            // one: report it instead of retrying the v0 grammar.
            Err(ParseError::Invalid) => {}
            Err(error) => budget_error = Some(error),
        }
    }

    let Some(prefix_len) = v0::prefix_len(symbol) else {
        return Err(budget_error.unwrap_or(ParseError::Invalid));
    };
    match v0::parse(symbol, prefix_len) {
        Ok(parsed) => Ok(Parsed::V0 {
            prefix_len,
            root: parsed.root,
            instantiating_crate: parsed.instantiating_crate,
            end: parsed.end,
        }),
        Err(error) => Err(budget_error.unwrap_or(error)),
    }
}

/// Strip a trailing `.llvm.<hex>` ThinLTO suffix, as the reference does.
fn strip_thin_lto(symbol: &str) -> &str {
    const LLVM: &str = ".llvm.";
    let Some(index) = symbol.find(LLVM) else {
        return symbol;
    };
    let Some(candidate) = symbol.get(index + LLVM.len()..) else {
        return symbol;
    };
    let all_hex = candidate
        .chars()
        .all(|c| matches!(c, '0'..='9' | 'A'..='F' | '@'));
    if all_hex {
        symbol.get(..index).unwrap_or(symbol)
    } else {
        symbol
    }
}

/// True when every byte is ASCII alphanumeric or ASCII punctuation.
fn is_symbol_like(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn detects_legacy_prefixes() {
        for (symbol, prefix) in [
            ("_ZN3fooE", "_ZN"),
            ("ZN3fooE", "ZN"),
            ("__ZN3fooE", "__ZN"),
        ] {
            let ast = parse(symbol).unwrap();
            assert_eq!(ast.encoding(), Encoding::Legacy);
            assert_eq!(ast.prefix(), prefix);
            assert_eq!(ast.to_string(), "foo");
        }
    }

    #[test]
    fn detects_v0_prefixes() {
        for (symbol, prefix) in [("_RC3foo", "_R"), ("RC3foo", "R"), ("__RC3foo", "__R")] {
            let ast = parse(symbol).unwrap();
            assert_eq!(ast.encoding(), Encoding::V0);
            assert_eq!(ast.prefix(), prefix);
            assert_eq!(ast.to_string(), "foo");
        }
    }

    #[test]
    fn rejects_short_prefixes() {
        for symbol in ["_R", "R", "__R", "_ZN", "ZN", "__ZN", ""] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
    }

    #[test]
    fn strips_thin_lto_suffix() {
        for symbol in [
            "_ZN3fooE.llvm.9D1C9369",
            "_ZN3fooE.llvm.9D1C9369@@16",
            "_RC3foo.llvm.9D1C9369",
        ] {
            let ast = parse(symbol).unwrap();
            assert_eq!(ast.suffix(), "");
            assert_eq!(ast.to_string(), "foo");
        }
    }

    #[test]
    fn keeps_vendor_suffix() {
        let ast = parse("_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0.0").unwrap();
        assert_eq!(ast.suffix(), ".0.0");
        assert_eq!(ast.to_string(), "rand::fork::FORK_HANDLER_REGISTERED.0.0");
    }

    #[test]
    fn rejects_unsymbol_like_suffix() {
        assert_eq!(
            parse("_ZN3fooE.llvm moocow").unwrap_err(),
            ParseError::Invalid
        );
        assert_eq!(parse("_ZN3fooEv").unwrap_err(), ParseError::Invalid);
        assert_eq!(parse("_RC3foo bar").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn rejects_cpp_names() {
        // `_ZN3Foo3barEv` leaves a `v` tail that is not a vendor suffix, and
        // `_ZN4test` has no `E` terminator.
        for symbol in ["_ZN3Foo3barEv", "_ZN4test"] {
            assert_eq!(parse(symbol).unwrap_err(), ParseError::Invalid, "{symbol}");
        }
        // `_ZN3fooE` is valid in both schemes; only a full parse decides.
        assert_eq!(parse("_ZN3fooE").unwrap().to_string(), "foo");
    }

    #[test]
    fn input_budget_is_enforced() {
        let mut symbol = String::from("_R");
        symbol.push_str(&"p".repeat(MAX_INPUT_LEN));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);
    }

    #[test]
    fn rejects_non_ascii_input() {
        assert_eq!(parse("_ZN4t\u{e9}stE").unwrap_err(), ParseError::Invalid);
        assert_eq!(parse("_RC4t\u{e9}st").unwrap_err(), ParseError::Invalid);
    }

    #[test]
    fn ast_encoding_is_reported() {
        assert_eq!(parse("_ZN3fooE").unwrap().encoding(), Encoding::Legacy);
        assert_eq!(parse("_RC3foo").unwrap().encoding(), Encoding::V0);
    }
}
