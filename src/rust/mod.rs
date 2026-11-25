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
//! Printing is not a budget: depth and visits are checked at parse time by the
//! fixed envelope ([`crate::rust::MAX_PRINT_DEPTH`],
//! [`crate::rust::MAX_PRINT_VISITS`]); [`crate::rust::MAX_OUTPUT_BYTES`] is a
//! truncating render outcome, not an error. Raising the parse depth can
//! therefore still be rejected by the fixed envelope.
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

use crate::budget::{EffectiveBudget, Limit};
use node::ParserLimits;

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
/// tree whose expanded print visits or depth exceed the printer's fixed
/// internal caps.
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
};

/// Parse `symbol` into a reusable tree with the compiled-in budget.
///
/// # Errors
///
/// Returns [`ParseError::Invalid`] when the symbol matches neither encoding,
/// [`ParseError::RecursedTooDeep`] when it nests deeper than [`MAX_DEPTH`] and
/// [`ParseError::BudgetExceeded`] when it exceeds the node, work or input
/// budget. A symbol that parses but cannot be printed is also rejected, so
/// every [`Ast`] is printable.
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
    hash.strip_prefix(b"17h").is_some_and(|digits| {
        digits.len() == 16 && digits.iter().all(u8::is_ascii_hexdigit)
    })
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
/// raising them never unbound an allocation or a recursive descent. Printing
/// is not part of the budget, so the print envelope below stays fixed.
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
    if !suffix_bytes.is_empty() && !(suffix_bytes.starts_with(b".") && is_symbol_like(suffix_bytes))
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
    if ast.root.print_visits() > MAX_PRINT_VISITS
        || ast.root.print_depth().saturating_mul(2) > MAX_PRINT_DEPTH
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
mod tests {
    use alloc::format;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use super::*;
    use crate::budget::Budget;
    use crate::scheme::Scheme;

    /// The scheme a symbol's mangling prefix announces.
    fn scheme_of(symbol: &str) -> Scheme {
        if v0::prefix(symbol.as_bytes()).is_some() {
            Scheme::RustV0
        } else {
            Scheme::RustLegacy
        }
    }

    #[test]
    fn the_compiled_in_budget_matches_the_policy_defaults() {
        let legacy = Budget::default().effective(Scheme::RustLegacy);
        assert_eq!(legacy.input_bytes, DEFAULT_BUDGET.input_bytes);
        assert_eq!(legacy.parse_depth, None);
        assert_eq!(legacy.work, DEFAULT_BUDGET.work);
        assert_eq!(legacy.nodes, DEFAULT_BUDGET.nodes);

        let v0 = Budget::default().effective(Scheme::RustV0);
        assert_eq!(v0, DEFAULT_BUDGET);
    }

    #[test]
    fn absent_dimensions_keep_the_compiled_in_limits() {
        // An unset dimension must resolve to the constants the parser used
        // before the budget was configurable.
        for (scheme, v0) in [(Scheme::RustLegacy, false), (Scheme::RustV0, true)] {
            let limits = ParserLimits::from_effective(Budget::new().effective(scheme), v0);
            assert_eq!(limits.input, MAX_INPUT_LEN, "{scheme}");
            assert_eq!(limits.depth, MAX_DEPTH, "{scheme}");
            assert_eq!(limits.work, MAX_WORK, "{scheme}");
            assert_eq!(limits.nodes, MAX_NODES, "{scheme}");
        }
    }

    /// Every parser dimension rejects one unit below the value a symbol needs
    /// and accepts at that value.
    #[test]
    fn lowering_each_dimension_rejects_at_the_boundary() {
        /// The hard cap of one dimension, which bounds the search.
        fn ceiling(dimension: Option<Limit>) -> usize {
            match dimension {
                Some(Limit::Bounded(value)) => value,
                _ => panic!("the scheme must bound this dimension"),
            }
        }

        fn assert_boundary(
            symbol: &str,
            name: &str,
            expected: ParseError,
            ceiling: usize,
            budget: &dyn Fn(usize) -> Budget,
        ) {
            let scheme = scheme_of(symbol);
            let needed = (0..=ceiling)
                .find(|&value| {
                    parse_bytes_with_budget(symbol.as_bytes(), budget(value).effective(scheme))
                        .is_ok()
                })
                .unwrap_or_else(|| panic!("{symbol}: no {name} within {ceiling} parses"));
            assert!(needed > 0, "{symbol}: {name} is not bounded");
            assert_eq!(
                parse_bytes_with_budget(symbol.as_bytes(), budget(needed - 1).effective(scheme))
                    .unwrap_err(),
                expected,
                "{symbol}: {name} accepted at {} (needs {needed})",
                needed - 1
            );
        }

        for symbol in [
            "_RC3foo",
            "_RNvCs1234_4test4main",
            "_ZN3fooE",
            "_ZN4test4main17h1234567890abcdefE",
        ] {
            let scheme = scheme_of(symbol);
            let hard = scheme.hard_budget();
            assert_boundary(
                symbol,
                "input",
                ParseError::BudgetExceeded,
                ceiling(hard.input_bytes),
                &|value| Budget::new().with_input_bytes(value),
            );
            assert_boundary(
                symbol,
                "work",
                ParseError::BudgetExceeded,
                ceiling(hard.work),
                &|value| Budget::new().with_work(to_u32(value)),
            );
            assert_boundary(
                symbol,
                "nodes",
                ParseError::BudgetExceeded,
                ceiling(hard.nodes),
                &|value| Budget::new().with_nodes(to_u32(value)),
            );
            if scheme == Scheme::RustV0 {
                assert_boundary(
                    symbol,
                    "depth",
                    ParseError::RecursedTooDeep,
                    ceiling(hard.parse_depth),
                    &|value| Budget::new().with_parse_depth(to_u32(value)),
                );
            }
        }
    }

    /// Widen a scanned boundary to the `u32` the budget builders take.
    fn to_u32(value: usize) -> u32 {
        u32::try_from(value).unwrap_or(u32::MAX)
    }

    /// A symbol the default budget rejects for its length is accepted once the
    /// caller raises the input budget within the hard cap.
    #[test]
    fn raising_the_input_budget_accepts_longer_symbols() {
        let mut symbol = String::from("_ZN");
        while symbol.len() < MAX_INPUT_LEN + 4 * 1024 {
            symbol.push_str("3foo");
        }
        symbol.push('E');
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);

        let budget = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::RustLegacy))
            .expect("raised input budget");
        assert_eq!(ast.encoding(), Encoding::Legacy);
        assert_eq!(ast.root().children().len(), (symbol.len() - 3) / 4);
    }

    /// The depth hard cap admits a tree deeper than the reference limit, as
    /// long as the fixed print envelope still covers it.
    #[test]
    fn raising_the_depth_budget_accepts_deeper_trees() {
        let symbol = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::RecursedTooDeep);

        let budget = Budget::new().with_parse_depth(501);
        let ast = parse_bytes_with_budget(symbol.as_bytes(), budget.effective(Scheme::RustV0))
            .expect("raised depth budget");
        assert!(!ast.to_string().is_empty());
    }

    /// Work and nodes are separate dimensions: raising the input budget alone
    /// still rejects on the node budget, and raising both accepts.
    #[test]
    fn raising_the_node_budget_accepts_wider_trees() {
        let symbol = format!("_RMC0T{}E", "p".repeat(70_000));
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);

        let input_only = Budget::new().with_input_bytes(128 * 1024);
        assert_eq!(
            parse_bytes_with_budget(symbol.as_bytes(), input_only.effective(Scheme::RustV0))
                .unwrap_err(),
            ParseError::BudgetExceeded
        );

        let raised = input_only.with_nodes(100_000);
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::RustV0)).is_ok()
        );
    }

    /// A long back-reference run costs work, not nodes, so it needs the input
    /// and work budgets raised together.
    #[test]
    fn raising_the_work_budget_accepts_longer_backref_runs() {
        let mut symbol = String::from("_RMC0TTpE");
        for _ in 0..MAX_WORK {
            symbol.push_str("B3_");
        }
        symbol.push('E');
        assert_eq!(parse(&symbol).unwrap_err(), ParseError::BudgetExceeded);

        let raised = Budget::new()
            .with_input_bytes(1024 * 1024)
            .with_work(1_000_000);
        assert!(
            parse_bytes_with_budget(symbol.as_bytes(), raised.effective(Scheme::RustV0)).is_ok()
        );
    }

    /// Raising a dimension past its hard cap is clamped, never unbounded.
    #[test]
    fn over_hard_budgets_are_clamped_to_the_hard_caps() {
        let budget = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(u32::MAX)
            .with_work(u32::MAX)
            .with_nodes(u32::MAX);
        assert_eq!(
            budget.effective(Scheme::RustV0).input_bytes,
            Some(Limit::Bounded(1024 * 1024))
        );
        assert_eq!(
            budget.effective(Scheme::RustLegacy).input_bytes,
            Some(Limit::Bounded(16 * 1024 * 1024))
        );
        assert_eq!(
            budget.effective(Scheme::RustV0).parse_depth,
            Some(Limit::Bounded(512))
        );
        assert_eq!(
            budget.effective(Scheme::RustV0).nodes,
            Some(Limit::Bounded(1024 * 1024))
        );
    }

    /// Zero at a migrated dimension rejects through the public demangler and
    /// through a Selection sharing the same budget.
    #[test]
    fn zero_input_budget_rejects_through_demangler_and_selection() {
        use crate::Demangler;

        let demangler = Demangler::with_budget(Budget::new().with_input_bytes(0));
        assert!(demangler.demangle("_RC3foo").is_err());
        assert!(
            demangler
                .selection()
                .allow(Scheme::RustV0)
                .demangle("_RC3foo")
                .is_err()
        );
    }

    /// Raising a dimension through the demangler, not only through the
    /// backend parse helper.
    #[test]
    fn demangler_raising_the_depth_budget_accepts_deeper_trees() {
        use crate::Demangler;

        let symbol = format!("_R{}C0{}", "I".repeat(500), "E".repeat(500));
        assert!(Demangler::new().demangle(&symbol).is_err());
        let raised = Demangler::with_budget(Budget::new().with_parse_depth(501));
        assert!(raised.demangle(&symbol).is_ok());
    }

    /// Rust legacy is iterative and has no depth dimension; a depth override
    /// is not reported as enforced for it.
    #[test]
    fn legacy_reports_no_depth_dimension() {
        assert_eq!(
            Budget::new()
                .with_parse_depth(0)
                .effective(Scheme::RustLegacy)
                .parse_depth,
            None
        );
        assert_eq!(
            Scheme::RustV0
                .hard_budget()
                .parse_depth,
            Some(Limit::Bounded(512))
        );
    }

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
    fn keeps_a_bare_thin_lto_marker() {
        // The shared decoration stripper only removes `.llvm.` when a non-empty
        // hex/`@` candidate follows, so a bare marker stays a vendor suffix
        // instead of vanishing from the printed name.
        let ast = parse("_ZN3fooE.llvm.").unwrap();
        assert_eq!(ast.suffix(), ".llvm.");
        assert_eq!(ast.to_string(), "foo.llvm.");
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
        // The byte entry point enforces the same input budget.
        assert_eq!(
            parse_bytes(symbol.as_bytes()).unwrap_err(),
            ParseError::BudgetExceeded
        );
    }

    /// A long but valid symbol stays inside the work and node budgets: the
    /// cursor charges one work unit per byte read, so a symbol that fits the
    /// input budget must not trip the work budget first.
    #[test]
    fn long_symbols_stay_within_the_work_budget() {
        let mut symbol = String::from("_ZN");
        while symbol.len() < MAX_INPUT_LEN - 8 {
            symbol.push_str("3foo");
        }
        symbol.push('E');
        let ast = parse_bytes(symbol.as_bytes()).unwrap();
        assert_eq!(ast.encoding(), Encoding::Legacy);
        assert_eq!(ast.root().children().len(), (symbol.len() - 3) / 4);
        assert!(ast.to_string().len() <= MAX_OUTPUT_BYTES);
    }

    /// Arbitrary bytes, including invalid UTF-8, never panic the parsers and
    /// never exceed the output budget.
    #[test]
    fn byte_fuzz_never_panics() {
        const PREFIXES: [&[u8]; 6] = [b"_R", b"_RN", b"_ZN", b"ZN", b"__R", b"R"];
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for iteration in 0..2_000 {
            let prefix = PREFIXES[usize::try_from(next() % 6).unwrap_or(0)];
            let mut symbol = Vec::from(prefix);
            let len = usize::try_from(next() % 24).unwrap_or(0);
            for _ in 0..len {
                symbol.push(u8::try_from(next() % 256).unwrap_or(0));
            }
            if let Ok(ast) = parse_bytes(&symbol) {
                let rendered = ast.to_string();
                assert!(
                    rendered.len() <= MAX_OUTPUT_BYTES,
                    "iteration {iteration}: {} bytes of output",
                    rendered.len()
                );
            }
        }
    }

    #[test]
    fn rejects_non_ascii_suffixes() {
        // A vendor suffix must stay symbol-like, which is an ASCII-only rule.
        assert_eq!(
            parse("_RNvNtC4rand4fork23FORK_HANDLER_REGISTERED.0é").unwrap_err(),
            ParseError::Invalid
        );
    }

    /// Names are byte strings: an identifier that is not UTF-8 parses and is
    /// escaped by the printer instead of being rejected.
    #[test]
    fn accepts_invalid_utf8_identifiers() {
        let ast = parse_bytes(b"_ZN2\xff\xfeE").unwrap();
        assert_eq!(ast.to_string(), "\\xff\\xfe");
        let ast = parse_bytes("_ZN5téstE".as_bytes()).unwrap();
        assert_eq!(ast.to_string(), "tést");
        let ast = parse_bytes(b"_RC2\xff\xfe").unwrap();
        assert_eq!(ast.to_string(), "\\xff\\xfe");
    }

    /// The byte entry point is crate-internal and mirrors `parse`.
    #[test]
    fn corpus_print_envelope_holds() {
        let base = format!("{}/refs/rust/corpus", env!("CARGO_MANIFEST_DIR"));
        let Ok(entries) = std::fs::read_dir(base) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("txt") {
                continue;
            }
            let Ok(data) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in data.lines() {
                let symbol = line.split(" ---> ").next().unwrap_or_default();
                if symbol.is_empty() || symbol.starts_with('#') {
                    continue;
                }
                let Ok(ast) = parse(symbol) else {
                    continue;
                };
                let mut out = String::new();
                let (visits, depth) = match printer::stats(&ast, &mut out) {
                    Ok(stats) => stats,
                    Err(error) => panic!("{symbol}: print failed after envelope: {error:?}"),
                };
                let root = ast.root();
                assert!(
                    visits <= root.print_visits(),
                    "{symbol}: {visits} visits > {}",
                    root.print_visits()
                );
                assert!(
                    depth <= root.print_depth().saturating_mul(2),
                    "{symbol}: depth {depth} > {}",
                    root.print_depth().saturating_mul(2)
                );
            }
        }
    }

    #[test]
    fn parse_bytes_matches_parse() {
        for symbol in [
            "_ZN3fooE",
            "_ZN4test4main17h1234567890abcdefE",
            "_RC3foo",
            "_RNvCs1234_4test4main",
            "_ZN3fooE.llvm.9D1C9369",
            "_ZN3fooE.llvm moocow",
            "_RC3foo bar",
            "",
            "main",
        ] {
            let text = parse(symbol);
            let bytes = parse_bytes(symbol.as_bytes());
            match (text, bytes) {
                (Ok(text), Ok(bytes)) => {
                    assert_eq!(text.to_string(), bytes.to_string(), "{symbol}");
                    assert_eq!(text.prefix(), bytes.prefix(), "{symbol}");
                    assert_eq!(text.suffix(), bytes.suffix(), "{symbol}");
                    assert_eq!(text.encoding(), bytes.encoding(), "{symbol}");
                }
                (Err(text), Err(bytes)) => assert_eq!(text, bytes, "{symbol}"),
                _ => panic!("{symbol}: byte and text paths disagree"),
            }
        }
    }

    #[test]
    fn ast_encoding_is_reported() {
        assert_eq!(parse("_ZN3fooE").unwrap().encoding(), Encoding::Legacy);
        assert_eq!(parse("_RC3foo").unwrap().encoding(), Encoding::V0);
    }
}
