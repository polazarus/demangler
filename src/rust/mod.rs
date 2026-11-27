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
//! parser cannot represent, and checks the tree against the printer envelope
//! (expanded visits and depth).
//! Symbols are read as bytes through the crate's shared cursor, so lengths and
//! backrefs are byte offsets and an identifier may hold bytes that are not
//! UTF-8. Text that is not UTF-8 is escaped as `\xNN` by the printer instead of
//! rejecting the symbol; valid UTF-8 prints verbatim. Every input the reference
//! decoder `rustc_demangle` accepts still demangles to the same text.
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
//! Untrusted input stays bounded by four parser dimensions — input bytes,
//! nesting depth, work units and allocated nodes — that a
//! [`crate::Budget`] may lower or raise within the scheme's hard ceiling. The
//! public [`crate::rust::parse`] keeps the compiled-in defaults
//! ([`crate::rust::MAX_INPUT_LEN`], [`crate::rust::MAX_DEPTH`], [`crate::rust::MAX_WORK`] and
//! [`crate::rust::MAX_NODES`]); a [`crate::Demangler`] resolves its configured budget
//! for [`crate::Scheme::RustLegacy`] or [`crate::Scheme::RustV0`] and passes the clamped
//! values to the parser. Exceeding a parse budget is an error, never a partial
//! tree.
//!
//! Two further dimensions are parse-time expansion gates. A v0 backref makes
//! the graph a DAG, so one node can be printed once per reference: the parser
//! charges every expanded visit and a conservative upper bound on the rendered
//! bytes while it builds each node, and rejects a tree that exceeds the
//! resolved `expansion_visits` or `expansion_bytes` gate. The defaults are
//! 4,000,000 visits and 1 MiB; a [`crate::Budget`] may lower or raise them
//! within the scheme's hard ceiling.
//!
//! Printing is not a budget. The printer has no depth, visit or output limit,
//! so a parser-built [`Ast`](crate::rust::Ast) always prints completely into a working sink; the
//! expansion gates and the caller's sink are the only output bounds. The one
//! fixed limit is the AST printability ceiling: `parse` rejects a tree whose
//! structural print depth exceeds 512, which is deeper than the recursive
//! printer can safely descend. That is a parser-side stack bound, not a
//! printer cap and not a [`crate::Budget`] dimension.
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

use crate::error::BudgetKind;
use core::fmt;

use crate::budget::{EffectiveBudget, Limit};
use node::ParserLimits;

pub use node::{
    AdtConstKind, ConstValueKind, ImplKind, Kind, MAX_DEPTH, MAX_INPUT_LEN, MAX_NODES, MAX_WORK,
    Namespace, Node, NodeData, NodeRef, Span,
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
/// [`ParseError::BudgetExceeded`] added for the input, node and work budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseError {
    /// The symbol does not match either grammar.
    Invalid,
    /// The symbol nests deeper than the parser depth budget (by default
    /// [`MAX_DEPTH`]).
    RecursedTooDeep,
    /// The symbol needs more nodes or work than the budgets allow.
    BudgetExceeded,
}

impl ParseError {
    /// The parser budget dimension this failure belongs to, when it is a
    /// capacity failure rather than a malformed symbol.
    #[must_use]
    pub const fn budget_kind(&self) -> Option<BudgetKind> {
        match self {
            Self::RecursedTooDeep => Some(BudgetKind::ParseDepth),
            Self::BudgetExceeded => Some(BudgetKind::Other),
            _ => None,
        }
    }
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
/// The tree is retained and can be printed more than once, but every
/// `Display` call re-renders it; cache the rendered text if it is printed
/// repeatedly. It is `Rc`-owned, so `Ast` is `!Send + !Sync`. Parsing rejects a
/// tree the recursive printer cannot safely descend into, so every `Ast`
/// prints completely; the printer itself has no depth, visit or output limit.
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
        printer::print(self, f, f.alternate())
    }
}

/// The budget both rustc schemes start from.
///
/// The public [`parse`] entry points use it, so they keep exactly the
/// compiled-in limits; a [`crate::Demangler`] resolves its configured
/// [`crate::Budget`] per scheme instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH as usize)),
    work: Some(Limit::Bounded(MAX_WORK)),
    nodes: Some(Limit::Bounded(MAX_NODES)),
    expansion_visits: Some(Limit::Bounded(node::DEFAULT_EXPANSION_VISITS)),
    expansion_bytes: Some(Limit::Bounded(node::DEFAULT_EXPANSION_BYTES)),
};

/// Parse `symbol` into a reusable tree with the compiled-in budget.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol matches neither encoding,
/// [`ParseError::RecursedTooDeep`] when it nests deeper than [`MAX_DEPTH`] and
/// [`ParseError::BudgetExceeded`] when it exceeds the node, work, input,
/// expansion visit or expansion byte budget, or the fixed AST printability
/// limit. A tree the recursive printer cannot safely descend into is rejected
/// here, so every [`Ast`] prints completely.
pub fn parse(symbol: &str) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET)
}

/// Parse a raw symbol byte string with the compiled-in budget.
///
/// Test-only twin of [`parse`] for symbols that are not UTF-8; the crate's
/// byte entry points resolve a budget instead.
#[cfg(test)]
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, ParseError> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// True for a rustc v0 byte prefix with at least one tag byte.
#[must_use]
pub(crate) fn is_v0_prefix_bytes(symbol: &[u8]) -> bool {
    (symbol.len() > 2 && symbol.starts_with(b"_R"))
        || (symbol.len() > 3 && symbol.starts_with(b"__R"))
}

/// True for a legacy rustc byte symbol with a trailing crate hash.
#[must_use]
pub(crate) fn is_legacy_hash_bytes(symbol: &[u8]) -> bool {
    let inner = symbol
        .strip_prefix(b"__ZN")
        .or_else(|| symbol.strip_prefix(b"_ZN"))
        .or_else(|| symbol.strip_prefix(b"ZN"));
    let Some(inner) = inner else {
        return false;
    };
    let Some(path) = inner.strip_suffix(b"E") else {
        return false;
    };
    let Some(hash) = path.get(path.len().saturating_sub(19)..) else {
        return false;
    };
    hash.strip_prefix(b"17h")
        .is_some_and(|digits| digits.len() == 16 && digits.iter().all(u8::is_ascii_hexdigit))
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// The public [`parse`] wraps this with [`DEFAULT_BUDGET`]; a
/// [`crate::Demangler`] passes [`crate::Demangler::effective_budget`] for the
/// scheme it dispatches, so a caller's overrides reach both encodings.
/// Lengths and backrefs are byte offsets, and text that is not UTF-8 is
/// escaped by the printer, never rejected or replaced.
///
/// The budget's parser dimensions are clamped to this module's hard ceilings:
/// raising them never unbound an allocation or a recursive descent. Printing is
/// not part of the budget, so the printer has no limit; the two expansion gates
/// and the fixed AST printability ceiling are checked here, after the tree is
/// built, so an accepted tree always prints completely.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, ParseError> {
    let limits = ParserLimits::from_effective(budget, v0::prefix(symbol).is_some());
    if symbol.len() > limits.input {
        return Err(ParseError::BudgetExceeded);
    }

    // ThinLTO renames internal symbols; the shared decoration stripper removes
    // those endings before parsing and never prints them back.
    let stripped = crate::scheme::strip_thin_lto_bytes(symbol);
    let parsed = parse_symbol(stripped, limits)?;
    let suffix_bytes = stripped.get(parsed.end()..).unwrap_or(&[]);
    if !suffix_bytes.is_empty()
        && (!suffix_bytes.starts_with(b".") || !is_symbol_like(suffix_bytes))
    {
        return Err(ParseError::Invalid);
    }
    // `is_symbol_like` only accepts ASCII, so the suffix is valid UTF-8.
    let suffix = core::str::from_utf8(suffix_bytes).map_err(|_| ParseError::Invalid)?;

    let (encoding, prefix, root, instantiating_crate) = match parsed {
        Parsed::Legacy { prefix, root, .. } => (Encoding::Legacy, prefix, root, None),
        Parsed::V0 {
            prefix,
            root,
            instantiating_crate,
            ..
        } => (Encoding::V0, prefix, root, instantiating_crate),
    };
    let ast = Ast {
        encoding,
        prefix,
        suffix,
        root,
        instantiating_crate,
    };
    // The printer has no caps, so the parser is the only place that can reject
    // a tree it could not render: the two expansion gates bound a DAG's
    // expanded output and the fixed ceiling bounds the printer's recursion.
    if ast.root.print_visits() > limits.expansion_visits
        || ast.root.print_bytes() > limits.expansion_bytes
        || ast.root.print_depth() > node::MAX_PRINT_DEPTH
    {
        return Err(ParseError::BudgetExceeded);
    }
    Ok(ast)
}

/// A successful parse of one encoding, before the suffix rule is applied.
enum Parsed<'a> {
    Legacy {
        prefix: &'static str,
        root: NodeRef<'a>,
        end: usize,
    },
    V0 {
        prefix: &'static str,
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
}

/// Try the legacy front-end, then the v0 one, like `rustc-demangle` does.
fn parse_symbol(symbol: &[u8], limits: ParserLimits) -> Result<Parsed<'_>, ParseError> {
    let mut budget_error = None;
    if let Some((prefix_len, prefix)) = legacy::prefix(symbol) {
        match legacy::parse(symbol, prefix_len, limits) {
            Ok((root, end)) => {
                return Ok(Parsed::Legacy { prefix, root, end });
            }
            // Legacy is iterative, so the only recoverable failure is a budget
            // one: report it instead of retrying the v0 grammar.
            Err(ParseError::Invalid) => {}
            Err(error) => budget_error = Some(error),
        }
    }

    let Some((prefix_len, prefix)) = v0::prefix(symbol) else {
        return Err(budget_error.unwrap_or(ParseError::Invalid));
    };
    match v0::parse(symbol, prefix_len, limits) {
        Ok(parsed) => Ok(Parsed::V0 {
            prefix,
            root: parsed.root,
            instantiating_crate: parsed.instantiating_crate,
            end: parsed.end,
        }),
        Err(error) => Err(budget_error.unwrap_or(error)),
    }
}

/// True when every byte is ASCII alphanumeric or ASCII punctuation.
fn is_symbol_like(text: &[u8]) -> bool {
    text.iter()
        .all(|&byte| byte.is_ascii_alphanumeric() || byte.is_ascii_punctuation())
}

#[cfg(test)]
mod tests;
