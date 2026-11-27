//! Pure-Rust Swift symbol demangling.
//!
//! The demangler is a postfix operator machine that builds an immutable
//! `Rc`-owned node graph, followed by a printer that writes readable text into
//! any [`core::fmt::Write`] sink. A node is built once and never mutated, and
//! parsed graphs are retained so callers can render them repeatedly without
//! re-parsing. Shared nodes make the graph a DAG, so one node can be printed
//! once per reference; the parser bounds that expansion with the two
//! expansion gates described under [Budgets](self#budgets).
//!
//! Reference: `refs/swift/Mangling.rst` (vendored, pinned upstream commit).
//!
//! # Input bytes
//!
//! Both parsers read `&[u8]`: Swift identifier lengths are byte counts, so a
//! symbol is not required to be UTF-8. Parsing decisions are byte comparisons
//! only; UTF-8 is a printing concern handled by the `text` module per site:
//!
//! - identifiers (`4main`, word-substituted and punycode forms) that are not
//!   valid UTF-8 become owned text with each invalid byte spelled `\xNN`;
//! - compiler payloads — the unmangled suffix after `.` and nested legacy
//!   payload text — keep their bytes verbatim (invalid bytes `\xNN`, no other
//!   rewriting);
//! - valid UTF-8 and ASCII always borrow and print unchanged.
//!
//! # Supported prefixes
//!
//! - `$s` — stable Swift mangling
//! - `$S` — Swift 4.2
//! - `_$s`, `_$S` — the same manglings behind a Mach-O symbol underscore
//! - `_T0` — Swift 4.0 (postfix grammar, old function-type layout)
//! - `_T` — pre-stable mangling, handled by the legacy parser (`legacy.rs`)
//!
//! `$e` (Embedded Swift) and `@__swiftmacro_` (file names) are rejected, as
//! are symbolic references (control bytes), which the ABI requires tools to
//! refuse on untrusted input.
//!
//! # Budgets
//!
//! Untrusted input stays bounded by four parser dimensions — input bytes,
//! nesting depth, work units and allocated nodes — that a [`crate::Budget`]
//! may lower or raise within the scheme's hard ceiling. The public
//! [`parse`](crate::swift::parse) keeps the compiled-in defaults
//! (`MAX_INPUT_LEN`, `MAX_DEPTH`, `WORK_BUDGET` and `MAX_NODES`); a
//! [`crate::Demangler`] resolves its configured budget for
//! [`crate::Scheme::SwiftStable`] or [`crate::Scheme::SwiftLegacy`] and passes
//! the clamped values to both grammars. Exceeding a parse budget is an error,
//! never a partial tree.
//!
//! Two further dimensions are parse-time expansion gates. A shared node is
//! printed once per reference, so the parser charges every expanded visit and
//! a conservative upper bound on the rendered bytes while it builds each node,
//! and rejects a tree that exceeds the resolved `expansion_visits` or
//! `expansion_bytes` gate. The defaults are 131,072 visits and 1 MiB; a
//! [`crate::Budget`] may lower or raise them within the scheme's hard ceiling.
//!
//! Printing is not a budget. The printer has no depth, visit or output limit,
//! so a parser-built [`Ast`](crate::swift::Ast) always prints completely into a
//! working sink; the expansion gates and the caller's sink are the only output
//! bounds. The one fixed limit is the AST printability ceiling: `parse` rejects
//! a tree whose structural print depth exceeds `MAX_PRINT_DEPTH`, which is
//! deeper than the recursive printer can safely descend. That is a parser-side
//! stack bound, not a printer cap and not a [`crate::Budget`] dimension.
//!
//! [`crate::Scheme::detect`] keeps the compiled-in defaults. A
//! [`crate::Demangler`] passes the configured budget to the legacy `_T`
//! prefilter and re-parses a retained tree, so enforcement is exact.
//!
//! # Inspecting a tree
//!
//! [`parse`](crate::swift::parse) returns an [`Ast`](crate::swift::Ast) whose
//! [`Node`](crate::swift::Node)s expose kind, text, children and the typed
//! [`NodeData`](crate::swift::NodeData) payload. [`Ast::root`](crate::swift::Ast::root)
//! and [`Node::child`](crate::swift::Node::child) hand out
//! [`NodeRef`](crate::swift::NodeRef) handles, which are cheap `Rc` clones of
//! the retained graph; the graph is therefore `!Send + !Sync`.

mod legacy;
mod node;
mod parser;
mod printer;
mod text;

use core::fmt;

pub(crate) use node::MAX_NODES;
pub use node::{
    Accessor, AttributeKind, AutoDiffCode, DeinitializerKind, GenericSpecializationKind, Global,
    GlobalOperand, KeyPathKind, Kind, LayoutCode, MacroRole, Node, NodeData, NodeRef, ResumeStyle,
};

use crate::budget::{EffectiveBudget, Limit, clamp};
use crate::parse::{Cursor, Limits, State};
use parser::Parser;
use printer::Printer;

/// Default maximum symbol bytes one parse may read.
///
/// The cursor bounds the bytes after the mangling prefix. Swift was the only
/// backend without an input cap; the largest corpus symbol is a few hundred
/// bytes. A [`crate::Demangler`] budget may raise it up to `HARD_INPUT_LEN`.
pub(crate) const MAX_INPUT_LEN: usize = 64 * 1024;

/// Default maximum operator recursion depth.
///
/// A [`crate::Demangler`] budget may raise it up to `HARD_DEPTH`.
pub(crate) const MAX_DEPTH: u16 = 96;

/// Default maximum input-work units one parse may consume.
///
/// A [`crate::Demangler`] budget may raise it up to `HARD_WORK`.
pub(crate) const WORK_BUDGET: u32 = 200_000;

/// Hard input ceiling: bounds the text and node vectors one parse allocates.
const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Fixed AST printability limit: the deepest tree the recursive printer may
/// descend into.
///
/// This is a parser-side stack bound, not a printer cap and not a
/// [`crate::Budget`] dimension: [`parse`] rejects a deeper tree before it can
/// reach [`core::fmt::Display`], so every parser-built AST prints without
/// overflowing the stack. The printer itself has no depth, visit or output
/// limit; it descends one frame per level, so the ceiling stays below the parse
/// depth envelope's 1024.
pub(crate) const MAX_PRINT_DEPTH: u16 = 128;

/// Hard depth ceiling.
///
/// It equals `MAX_PRINT_DEPTH`, so accepting more parser depth would only
/// build a tree the printer could not safely render.
const HARD_DEPTH: u16 = MAX_PRINT_DEPTH;

/// Hard work ceiling.
const HARD_WORK: u32 = 64 * 1024 * 1024;

/// Hard node ceiling: bounds the nodes one parse may allocate.
const HARD_NODES: usize = 1024 * 1024;

/// Recognise a supported Swift mangling prefix, returning its length.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    prefix_len_bytes(symbol.as_bytes())
}

/// Recognise a supported Swift mangling prefix in a byte string.
pub(crate) fn prefix_len_bytes(symbol: &[u8]) -> Option<usize> {
    if symbol.starts_with(b"$s") || symbol.starts_with(b"$S") {
        Some(2)
    } else if symbol.starts_with(b"_$s") || symbol.starts_with(b"_$S") {
        // Mach-O prefixes symbols with an underscore.
        Some(3)
    } else if symbol.starts_with(b"_T0") {
        Some(3)
    } else if symbol.starts_with(b"_T") {
        // Pre-stable mangling. The `_T` stem is shared with `_T0`, so the
        // longer prefix is tested first.
        Some(2)
    } else {
        None
    }
}

/// True when `symbol` starts with a Swift mangling prefix we recognise.
#[must_use]
pub fn is_swift(symbol: &str) -> bool {
    is_swift_bytes(symbol.as_bytes())
}

/// Strict Swift predicate used by detection.
///
/// `_T` is shared with ordinary C identifiers, so the legacy grammar must be
/// parsed before claiming it; `$s`/`$S`/`_$s`/`_T0` are unambiguous prefixes.
pub(crate) fn is_swift_bytes(symbol: &[u8]) -> bool {
    if prefix_len_bytes(symbol).is_none() {
        return false;
    }
    if symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0") {
        parse_bytes(symbol).is_ok() || legacy_claimed_bytes(symbol)
    } else {
        true
    }
}

/// Whether a non-postfix `_T` symbol has a known legacy shape.
///
/// Some corpus rows are deliberately unsupported legacy forms; keep claiming
/// their known top-level shape so they reject as Swift rather than falling
/// through as C. `_T` followed by an unrelated lowercase word (for example
/// `_Time`) is not a legacy production.
pub(crate) fn legacy_claimed_bytes(symbol: &[u8]) -> bool {
    let tail = symbol.get(2..).unwrap_or_default();
    tail.is_empty()
        || matches!(
            tail.first(),
            Some(b't' | b'w' | b'W' | b'T' | b'M' | b'C' | b'F' | b'V' | b'D' | b'f')
        )
        || tail.starts_with(b"iC")
}

/// Parse a pre-stable `_T` symbol when it is valid, without claiming unrelated
/// `_T`-prefixed C names.
pub(crate) fn parse_legacy_if(symbol: &[u8]) -> Option<Ast<'_>> {
    parse_legacy_if_with(symbol, DEFAULT_BUDGET)
}

/// Parse a pre-stable `_T` symbol with one resolved budget.
///
/// Detection uses this so its legacy prefilter sees the configured limits; the
/// default-budget wrapper keeps [`parse_legacy_if`] unchanged.
pub(crate) fn parse_legacy_if_with(symbol: &[u8], budget: EffectiveBudget) -> Option<Ast<'_>> {
    if symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0") {
        parse_bytes_with_budget(symbol, budget).ok()
    } else {
        None
    }
}

/// The budget both Swift schemes start from.
///
/// The public [`parse`] entry point uses it, so it keeps exactly the
/// compiled-in limits; a [`crate::Demangler`] resolves its configured
/// [`crate::Budget`] per scheme instead.
pub(crate) const DEFAULT_BUDGET: EffectiveBudget = EffectiveBudget {
    input_bytes: Some(Limit::Bounded(MAX_INPUT_LEN)),
    parse_depth: Some(Limit::Bounded(MAX_DEPTH as usize)),
    work: Some(Limit::Bounded(WORK_BUDGET as usize)),
    nodes: Some(Limit::Bounded(MAX_NODES)),
    expansion_visits: Some(Limit::Bounded(node::DEFAULT_EXPANSION_VISITS)),
    expansion_bytes: Some(Limit::Bounded(node::DEFAULT_EXPANSION_BYTES)),
};

/// The parser limits one parse enforces, resolved from a [`crate::Budget`].
///
/// Every value is already clamped to the hard ceiling of its dimension, so an
/// allocation or a recursive descent stays bounded whatever the caller asks
/// for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ParseLimits {
    /// Maximum accepted input length in bytes.
    input: usize,
    /// Maximum parser nesting depth.
    depth: u16,
    /// Maximum parser work units.
    work: u32,
    /// Maximum nodes one parse may allocate.
    nodes: usize,
    /// Maximum expansion visits one accepted tree may reach.
    expansion_visits: usize,
    /// Maximum rendered-byte estimate one accepted tree may reach.
    expansion_bytes: usize,
}

impl ParseLimits {
    /// Resolve one scheme's budget for a parse of either grammar.
    ///
    /// An absent dimension keeps its compiled-in default, and every dimension
    /// is clamped to this module's hard ceiling.
    fn from_effective(budget: EffectiveBudget) -> Self {
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, HARD_INPUT_LEN),
            depth: u16::try_from(clamp(
                budget.parse_depth,
                usize::from(MAX_DEPTH),
                usize::from(HARD_DEPTH),
            ))
            .unwrap_or(HARD_DEPTH),
            work: u32::try_from(clamp(budget.work, WORK_BUDGET as usize, HARD_WORK as usize))
                .unwrap_or(HARD_WORK),
            nodes: clamp(budget.nodes, MAX_NODES, HARD_NODES),
            expansion_visits: clamp(
                budget.expansion_visits,
                node::DEFAULT_EXPANSION_VISITS,
                node::HARD_EXPANSION_VISITS,
            ),
            expansion_bytes: clamp(
                budget.expansion_bytes,
                node::DEFAULT_EXPANSION_BYTES,
                node::HARD_EXPANSION_BYTES,
            ),
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The cursor enforces the input length, the recursion depth of the
    /// operator machine and the work units; the node budget stays with
    /// [`node::Context`](node) and the iterative wrapper counters.
    fn cursor(&self) -> Limits {
        Limits {
            input: u32::try_from(self.input).unwrap_or(u32::MAX),
            depth: self.depth,
            work: self.work,
        }
    }

    /// Maximum operator recursion depth.
    const fn depth(&self) -> u16 {
        self.depth
    }

    /// Maximum parser work units.
    const fn work(&self) -> u32 {
        self.work
    }

    /// Maximum nodes one parse may allocate.
    const fn nodes(&self) -> usize {
        self.nodes
    }

    /// Maximum expansion visits one accepted tree may reach.
    const fn expansion_visits(&self) -> usize {
        self.expansion_visits
    }

    /// Maximum rendered-byte estimate one accepted tree may reach.
    const fn expansion_bytes(&self) -> usize {
        self.expansion_bytes
    }
}

/// Which Swift mangling family a symbol uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// The postfix grammar of the stable manglings (`$s`, `$S`, `_$s`, `_$S`)
    /// and of Swift 4.0 (`_T0`).
    Stable,
    /// The pre-stable `_T` grammar handled by `legacy.rs`.
    Legacy,
}

/// A parsed Swift symbol with a read-only node graph.
///
/// The graph is retained and can be printed more than once without parsing the
/// mangled name again. It is `Rc`-owned, so `Ast` is `!Send + !Sync`.
#[derive(Clone, Debug)]
pub struct Ast<'a> {
    encoding: Encoding,
    root: NodeRef<'a>,
}

impl<'a> Ast<'a> {
    /// The mangling family the symbol used.
    #[must_use]
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The root node.
    #[must_use]
    pub fn root(&self) -> NodeRef<'a> {
        self.root.clone()
    }

    /// Render the retained graph into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        Printer::new(out).print(self.root.clone())
    }
}

/// Parse `symbol` with one resolved budget, without the expansion gates.
fn parse_inner(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, fmt::Error> {
    let mut state = State::new(limits.cursor());
    let mut nodes = 0;
    parse_counted(symbol, limits, &mut state, &mut nodes)
}

/// [`parse_inner`] with the work and node accounting exposed.
///
/// `state` receives the work charged and whether a budget stopped the parse,
/// and `nodes` the nodes allocated, so a nested payload parse can charge what
/// it spent to the parse that contains it.
fn parse_counted<'a>(
    symbol: &'a [u8],
    limits: ParseLimits,
    state: &mut State,
    nodes: &mut usize,
) -> Result<Ast<'a>, fmt::Error> {
    let Some(prefix) = prefix_len_bytes(symbol) else {
        return Err(fmt::Error);
    };
    let rest = symbol.get(prefix..).ok_or(fmt::Error)?;
    // Symbolic references start with control bytes; they must not be
    // interpreted (ABI requirement for untrusted input).
    if rest.iter().any(|&byte| byte < 0x20) || rest.is_empty() {
        return Err(fmt::Error);
    }
    // `_T` without the `_T0` version marker is the pre-stable prefix grammar;
    // every other prefix uses the postfix grammar.
    let legacy = symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0");
    let root = if legacy {
        legacy::parse(rest, state, limits, nodes).ok_or(fmt::Error)?
    } else {
        let input = Cursor::new(rest, 0, state).map_err(|_| fmt::Error)?;
        let mut parser = Parser::new(input, symbol.starts_with(b"_T"), limits);
        let parsed = parser.parse();
        let (used, failed) = parser.node_usage();
        *nodes = used;
        if failed {
            parser.exhaust();
            return Err(fmt::Error);
        }
        parsed.map_err(|_| fmt::Error)?
    };
    Ok(Ast {
        encoding: if legacy {
            Encoding::Legacy
        } else {
            Encoding::Stable
        },
        root,
    })
}

/// Parse `symbol` into a reusable graph that is guaranteed to print.
///
/// The returned [`Ast`] can be printed more than once, but each print renders
/// the retained graph again; cache the text if it is printed repeatedly. The
/// compiled-in budget is used; configure a [`crate::Budget`] on a
/// [`crate::Demangler`] to change it.
///
/// # Errors
///
/// Returns [`fmt::Error`] when the symbol is not Swift, uses an unsupported
/// prefix, contains symbolic references, fails to parse, or exceeds the
/// expansion visit, expansion byte or AST printability gate. The retained tree
/// can therefore be printed repeatedly without a parser failure.
pub fn parse(symbol: &str) -> Result<Ast<'_>, fmt::Error> {
    parse_bytes(symbol.as_bytes())
}

/// Parse a raw symbol byte string into a reusable graph.
///
/// Byte entry point for the crate-wide raw-byte dispatch: it accepts the same
/// grammars as [`parse`] without requiring the symbol to be UTF-8. Identifier
/// bytes that are not valid UTF-8 are escaped in the retained text (see the
/// [module docs](self)); parsing itself never requires a text decoding. The
/// compiled-in budget is used; a [`crate::Demangler`] resolves its configured
/// budget instead.
///
/// # Errors
///
/// Returns [`fmt::Error`] for the same reasons as [`parse`], including a
/// symbol longer than the input budget.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, fmt::Error> {
    parse_bytes_with_budget(symbol, DEFAULT_BUDGET)
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// [`parse_bytes`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
/// passes [`crate::Demangler::effective_budget`] for the scheme it dispatches,
/// so a caller's overrides reach both grammars. The four parser dimensions —
/// input bytes, nesting depth, work units and allocated nodes — are clamped to
/// this module's hard ceilings, so raising them never unbound an allocation or
/// a recursive descent.
///
/// The two expansion gates are checked here, after the tree is built: an
/// expanded visit or rendered-byte estimate outside the resolved
/// `expansion_visits`/`expansion_bytes` rejects the symbol, and so does a tree
/// deeper than `MAX_PRINT_DEPTH`. Printing is not part of the budget, so the
/// printer itself has no limit; an accepted tree always prints completely.
///
/// # Errors
///
/// Returns [`fmt::Error`] for the same reasons as [`parse_bytes`], including a
/// symbol outside the expansion gates.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, fmt::Error> {
    let limits = ParseLimits::from_effective(budget);
    let parsed = parse_inner(symbol, limits)?;
    let root = parsed.root();
    // The printer has no caps, so the parser is the only place that can reject
    // a tree it could not render: the two expansion gates bound a DAG's
    // expanded output and the fixed ceiling bounds the printer's recursion.
    if root.print_visits() > limits.expansion_visits()
        || root.print_bytes() > limits.expansion_bytes()
        || root.print_depth() > MAX_PRINT_DEPTH
    {
        return Err(fmt::Error);
    }
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
    print_bytes(symbol.as_bytes(), out)
}

/// Demangle a raw symbol byte string into `out`.
///
/// This goes through the same expansion gates as [`parse_bytes`]: the printer
/// has no cap, so an ungated tree could expand without bound.
pub(crate) fn print_bytes<W: fmt::Write + ?Sized>(symbol: &[u8], out: &mut W) -> fmt::Result {
    parse_bytes(symbol)?.print(out)
}

/// What remains of the containing parse's budget for one payload symbol.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PayloadAllowance {
    /// Nesting levels left below the payload.
    pub(crate) depth: u16,
    /// Work units left.
    pub(crate) work: u32,
    /// Nodes left.
    pub(crate) nodes: usize,
    /// Expansion visits not yet spent on earlier payloads.
    pub(crate) expansion_visits: usize,
    /// Rendered bytes not yet spent on earlier payloads.
    pub(crate) expansion_bytes: usize,
}

/// The text a payload contributes to the containing parse.
pub(crate) enum PayloadText {
    /// The payload demangled within its allowance.
    Rendered(alloc::string::String),
    /// The payload is not a symbol this backend reads; print it verbatim.
    Verbatim,
    /// The payload ran out of budget; reject the containing symbol.
    Exhausted,
}

/// What one payload parse produced and spent.
pub(crate) struct PayloadOutcome {
    /// The payload's text.
    pub(crate) text: PayloadText,
    /// Work units the nested parse charged.
    pub(crate) work: usize,
    /// Nodes the nested parse allocated.
    pub(crate) nodes: usize,
    /// Expansion visits the render charged.
    pub(crate) expansion_visits: usize,
    /// Rendered bytes the render charged.
    pub(crate) expansion_bytes: usize,
}

/// Demangle a symbol nested inside a legacy specialization payload.
///
/// The reference printer re-demangles payload text. The nested parse runs
/// under `allowance`, the part of the containing parse's budget not yet
/// spent, and reports what it used so the caller can charge it: nesting,
/// work, nodes and the rendered expansion all count against the one budget
/// the caller configured. A tree over the remaining expansion gates or the
/// printer's depth ceiling is a budget failure like any other.
pub(crate) fn demangle_payload(
    symbol: &[u8],
    limits: ParseLimits,
    allowance: PayloadAllowance,
) -> PayloadOutcome {
    let nested = ParseLimits {
        depth: allowance.depth,
        work: allowance.work,
        nodes: allowance.nodes,
        expansion_visits: allowance.expansion_visits,
        expansion_bytes: allowance.expansion_bytes,
        ..limits
    };
    let mut state = State::new(nested.cursor());
    let mut nodes = 0;
    let parsed = parse_counted(symbol, nested, &mut state, &mut nodes);
    let mut outcome = PayloadOutcome {
        text: PayloadText::Verbatim,
        work: state.work() as usize,
        nodes,
        expansion_visits: 0,
        expansion_bytes: 0,
    };
    let ast = match parsed {
        Ok(ast) => ast,
        Err(_) if state.exhausted() => {
            outcome.text = PayloadText::Exhausted;
            return outcome;
        }
        Err(_) => return outcome,
    };
    let root = ast.root();
    outcome.expansion_visits = root.print_visits();
    outcome.expansion_bytes = root.print_bytes();
    if outcome.expansion_visits > nested.expansion_visits
        || outcome.expansion_bytes > nested.expansion_bytes
        || root.print_depth() > MAX_PRINT_DEPTH
    {
        outcome.text = PayloadText::Exhausted;
        return outcome;
    }
    let mut out = alloc::string::String::new();
    outcome.text = match ast.print(&mut out) {
        Ok(()) => PayloadText::Rendered(out),
        Err(_) => PayloadText::Verbatim,
    };
    outcome
}

#[cfg(test)]
mod tests {
    use alloc::borrow::{Cow, ToOwned};
    use alloc::format;
    use alloc::rc::Rc;
    use alloc::string::String;
    use std::vec::Vec;

    use super::*;

    #[test]
    fn ast_is_inspectable() {
        let ast = parse("$s4main3fooyySiF").unwrap();
        assert_eq!(ast.root().kind(), Kind::TopLevel);

        let mut stack = std::vec![ast.root()];
        let mut texts = Vec::new();
        while let Some(node) = stack.pop() {
            assert!(node.child_count() <= node.children().len());
            if let Some(text) = node.text() {
                texts.push(text.to_owned());
            }
            stack.extend(node.children().iter().cloned());
            assert_eq!(node.is_leaf(), node.child_count() == 0);
        }

        assert!(texts.iter().any(|text| text == "main"));
        assert!(texts.iter().any(|text| text == "foo"));
    }

    #[test]
    fn envelope_bounds_rendered_bytes() {
        // `IndexSubset` text is the one kind whose printer writes more than its
        // own text: each `S` byte becomes an index. A maximal subset keeps the
        // byte estimate an upper bound.
        let base = "$s13TangentVector16_Differentiation14DifferentiablePQzAaDQy_SdAFIegnnnr_TJSd";
        let long_subset = format!("{base}{}pSrSUSP", "S".repeat(64));
        for symbol in [
            "$s4main3fooyySiF",
            "_TtSi",
            "_TtGSqGSaC5sugar7MyClass__",
            long_subset.as_str(),
        ] {
            let ast = parse(symbol).unwrap();
            let mut out = String::new();
            ast.print(&mut out).unwrap();
            assert!(
                out.len() <= ast.root().print_bytes(),
                "{symbol}: {} bytes rendered, envelope {}",
                out.len(),
                ast.root().print_bytes()
            );
        }
    }

    #[test]
    fn corpus_print_envelope_holds() {
        let base = env!("CARGO_MANIFEST_DIR");
        if !std::path::Path::new(base).join("refs/swift").is_dir() {
            std::eprintln!("refs/swift is absent; skipping the Swift envelope corpus check");
            return;
        }
        let mut compared = 0_usize;
        for file in [
            "refs/swift/manglings.txt",
            "refs/swift/toolchain-6.4.2-manglings.txt",
        ] {
            let Ok(data) = std::fs::read_to_string(format!("{base}/{file}")) else {
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
                let (visits, depth, bytes) = match printer::stats(&ast, &mut out) {
                    Ok(stats) => stats,
                    Err(error) => panic!("{symbol}: print failed after the gates: {error:?}"),
                };
                let root = ast.root();
                assert!(
                    visits <= root.print_visits(),
                    "{symbol}: {visits} visits > {}",
                    root.print_visits()
                );
                assert!(
                    depth <= root.print_depth(),
                    "{symbol}: depth {depth} > {}",
                    root.print_depth()
                );
                assert!(
                    bytes <= root.print_bytes(),
                    "{symbol}: {bytes} bytes > {}",
                    root.print_bytes()
                );
                // The accepted tree must also fit the default expansion gates,
                // so the corpus never relies on a raised budget.
                assert!(
                    root.print_visits() <= node::DEFAULT_EXPANSION_VISITS,
                    "{symbol}: {} visits > the default gate",
                    root.print_visits()
                );
                assert!(
                    root.print_bytes() <= node::DEFAULT_EXPANSION_BYTES,
                    "{symbol}: {} bytes > the default gate",
                    root.print_bytes()
                );
                assert!(
                    root.print_depth() <= MAX_PRINT_DEPTH,
                    "{symbol}: depth {} > the printability ceiling",
                    root.print_depth()
                );
                compared += 1;
            }
        }
        assert!(compared > 0, "the corpus compared no symbols");
    }

    #[test]
    fn ast_reports_the_mangling_family() {
        assert_eq!(
            parse("$s4main3fooyySiF").unwrap().encoding(),
            Encoding::Stable
        );
        assert_eq!(
            parse("_T03foo3barC3basyAA3zimCAE_tFTo").unwrap().encoding(),
            Encoding::Stable
        );
        assert_eq!(parse("_TtSi").unwrap().encoding(), Encoding::Legacy);
        assert_eq!(
            parse("_TF3foog3barSi").unwrap().encoding(),
            Encoding::Legacy
        );
    }

    #[test]
    fn ast_clone_shares_the_tree() {
        let ast = parse("$s4main3fooyySiF").unwrap();
        let cloned = ast.clone();
        assert!(Rc::ptr_eq(&ast.root(), &cloned.root()));
        assert_eq!(ast.root().kind(), cloned.root().kind());
        assert_eq!(ast.root().child_count(), cloned.root().child_count());
    }

    #[test]
    fn deep_iterative_wrappers_are_rejected_without_stack_overflow() {
        let symbol = std::format!("$sSi{}", "h".repeat(0xFFFE));
        let parsed = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || parse(&symbol).is_err())
            .expect("spawn")
            .join()
            .expect("join");
        assert!(parsed);
    }

    #[test]
    fn ast_text_can_be_borrowed() {
        let ast = parse("$s4main3fooyySiF").unwrap();
        let mut stack = std::vec![ast.root()];
        let mut node = None;
        while let Some(candidate) = stack.pop() {
            if candidate.kind() == Kind::Identifier && candidate.text() == Some("foo") {
                node = Some(candidate);
                break;
            }
            stack.extend(candidate.children().iter().cloned());
        }
        let node = node.unwrap();
        assert_eq!(node.text(), Some("foo"));
        assert!(matches!(node.text_cow(), Some(Cow::Borrowed(_))));
    }

    #[test]
    fn node_handles_and_payloads_are_inspectable() {
        let ast = parse("$s4main3fooyySiFyyXEfU_TA.1").unwrap();
        let root = ast.root();
        assert_eq!(root.kind(), Kind::TopLevel);
        assert!(matches!(root.data(), NodeData::None));
        let first = root.child(0).expect("top-level child");
        assert!(Rc::ptr_eq(&first, &root.children()[0]));
        assert_eq!(first.child_count(), first.children().len());

        let mut stack = std::vec![root];
        let mut saw_global = false;
        while let Some(node) = stack.pop() {
            if let NodeData::Global { code, operand } = node.data() {
                assert_eq!(node.global_code(), Some(*code));
                assert_eq!(node.global_operand(), *operand);
                saw_global = true;
            }
            stack.extend(node.children().iter().cloned());
        }
        assert!(saw_global, "expected a global wrapper");
    }
}

#[cfg(test)]
mod bytes {
    use alloc::borrow::Cow;

    use super::*;

    /// Render a raw byte symbol through the byte entry point.
    fn render_bytes(symbol: &[u8]) -> Option<alloc::string::String> {
        let mut out = alloc::string::String::new();
        print_bytes(symbol, &mut out).ok()?;
        Some(out)
    }

    #[test]
    fn byte_entry_matches_text_entry() {
        // Stable, legacy and Unicode symbols must parse and print identically
        // through both entries.
        for symbol in [
            "$s4main3fooyySiF",
            "$s4main1AVSDyACAC_G_ACt",
            "$s4main3fooyySiF.ig",
            "$s4main5f\u{f6}\u{f6}",
            "$s1A1BA_",
            "_TtSi",
            "_TtGSaSS_",
            "_TtGSqGSaC5sugar7MyClass__",
            "_TtSiXYZ",
        ] {
            let mut from_text = alloc::string::String::new();
            if print(symbol, &mut from_text).is_err() {
                // Both entries must reject the same symbols.
                assert_eq!(render_bytes(symbol.as_bytes()), None, "{symbol}");
                continue;
            }
            assert_eq!(
                render_bytes(symbol.as_bytes()).as_deref(),
                Some(from_text.as_str()),
                "{symbol}"
            );
            assert_eq!(
                parse(symbol).is_ok(),
                parse_bytes(symbol.as_bytes()).is_ok(),
                "{symbol}"
            );
        }
    }

    #[test]
    fn identifier_backslashes_are_reversible() {
        assert_eq!(render_bytes(b"$s4main3a\\b").as_deref(), Some("maina\\\\b"));
    }

    #[test]
    fn invalid_identifier_bytes_are_escaped() {
        // Identifier lengths are byte counts, so a length can split a UTF-8
        // sequence; the bytes are kept and escaped instead of rejecting.
        assert_eq!(
            render_bytes(b"$s4main3f\xffo").as_deref(),
            Some("mainf\\xffo")
        );
        assert_eq!(
            render_bytes(b"_TtC5sugar3f\xffo").as_deref(),
            Some("sugar.f\\xffo")
        );
        assert_eq!(
            render_bytes(b"_TtC5sug\xffr3abc").as_deref(),
            Some("sug\\xffr.abc")
        );
    }

    #[test]
    fn word_substitution_reuses_escaped_identifier() {
        // `0b0` is a word-substituted identifier: it must render exactly like
        // the identifier the word was read from.
        assert_eq!(
            render_bytes(b"$s4main3f\xffo0b0").as_deref(),
            Some("mainf\\xffof\\xffo")
        );
    }

    #[test]
    fn valid_utf8_identifiers_are_borrowed() {
        // Valid UTF-8 without identifier-special bytes borrows from the input.
        assert_eq!(render_bytes(b"$s4main3abc").as_deref(), Some("mainabc"));
        let ast = parse("$s4main5f\u{f6}\u{f6}").expect("demangles");
        let mut stack = std::vec![ast.root()];
        let mut borrowed = 0_usize;
        while let Some(node) = stack.pop() {
            if let Some(text) = node.text_cow() {
                assert!(matches!(text, Cow::Borrowed(_)));
                borrowed += 1;
            }
            stack.extend(node.children().iter().cloned());
        }
        assert!(borrowed >= 2, "expected borrowed identifier text");
    }

    #[test]
    fn lists_have_no_length_cap_of_their_own() {
        use alloc::format;
        use alloc::string::String;

        // C fixed-size arrays import as wide tuples; the node and input
        // budgets bound them, not a fixed element count.
        let tuple = format!("$sSi_{}tD", "Si".repeat(255));
        let ast = parse(&tuple).expect("a 256-element tuple parses");
        let mut out = String::new();
        ast.print(&mut out).unwrap();
        assert_eq!(out.matches("Swift.Int").count(), 256);
    }

    #[test]
    fn generic_parameter_names_are_charged_not_elided() {
        use alloc::string::String;

        // 199 generic parameters at depth 1 print every name; the expansion
        // budget, not a printer cutoff, bounds a huge count.
        let mut out = String::new();
        print("_Ttu0_198_rW_2T_2TJ_", &mut out).expect("parses");
        assert!(!out.contains("..."), "{out}");
        assert!(out.contains(", RH1>"), "{out}");
        assert!(parse("_Ttu4222222222222222222222222_rW_2T_2TJ_").is_err());
    }

    #[test]
    fn a_payload_never_prints_undemangled_under_any_budget() {
        use alloc::string::ToString;

        const SYMBOL: &str = "_TTSf1cl35_TFF7specgen6callerFSiT_U_FTSiSi_T_Si___TF7specgen12take_closureFFTSiSi_T_T_";
        let full = crate::Demangler::new()
            .demangle(SYMBOL)
            .unwrap()
            .to_string();
        assert!(full.contains("closure #1 (Swift.Int, Swift.Int)"), "{full}");
        let mut rejected = 0;
        for limit in 0..400 {
            for budget in [
                crate::Budget::new().with_work(limit),
                crate::Budget::new().with_nodes(limit),
                crate::Budget::new().with_parse_depth(limit / 16),
            ] {
                match crate::Demangler::with_budget(budget).demangle(SYMBOL) {
                    Ok(demangled) => assert_eq!(demangled.to_string(), full, "{budget:?}"),
                    Err(_) => rejected += 1,
                }
            }
        }
        assert!(rejected > 0, "the sweep never reached the payload's budget");
    }

    #[test]
    fn payload_bytes_stay_verbatim() {
        // Compiler payloads keep their bytes: an invalid byte is escaped, a
        // backslash is not rewritten.
        assert_eq!(
            render_bytes(b"$s4main3fooyySiF.\xff\\x").as_deref(),
            Some("main.foo(Swift.Int) -> () with unmangled suffix \".\\xff\\x\"")
        );
        assert_eq!(
            render_bytes(b"_TtSi\xff").as_deref(),
            Some("Swift.Int with unmangled suffix \"\\xff\"")
        );
    }

    #[test]
    fn byte_truncations_never_panic() {
        for symbol in [
            &b"$s4main3fooyySiF"[..],
            b"$s4main3f\xffoyySiF",
            b"$s4main3fooyySiF.\xff",
            b"_TtC5sugar7MyClass",
            b"_TtC5sug\xffr3abc",
            b"_TtSi\xff",
        ] {
            for end in 0..=symbol.len() {
                let _ = parse_bytes(&symbol[..end]);
                let _ = render_bytes(&symbol[..end]);
            }
        }
    }

    #[test]
    fn punycoded_identifier_separator_is_consumed() {
        // A Punycode payload that starts with a digit or `_` carries an extra
        // `_` between its length and the payload, and the length counts only
        // the payload. Both raw identifiers in the official corpus need it.
        assert_eq!(
            render_bytes(b"$s15raw_identifiers0018_3times_pgaIGJCFbhayyF").as_deref(),
            Some("raw_identifiers.`3\u{a0}times`() -> ()")
        );
        assert_eq!(
            render_bytes(b"$s15raw_identifiers10FontWeightO009_100_FpEpdyyFZ").as_deref(),
            Some("static raw_identifiers.FontWeight.`100`() -> ()")
        );
        // The declared payload length still bounds the identifier, so a
        // truncated payload is rejected instead of running into the tail.
        assert_eq!(
            render_bytes(b"$s15raw_identifiers0018_3times_pgaIGJCFb"),
            None
        );
    }

    #[test]
    fn input_budget_is_enforced() {
        // The cap applies to the bytes after the prefix. A symbol that fills
        // the budget parses; one byte more is rejected.
        let base = "$s4main3fooyySiF.";
        let rest = base.len() - 2;
        let under = alloc::format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest));
        assert!(parse_bytes(under.as_bytes()).is_ok());
        let over = alloc::format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest + 1));
        assert!(parse_bytes(over.as_bytes()).is_err());
    }

    #[test]
    fn nested_type_budget_is_preserved() {
        // The parser-side printability ceiling keeps rejecting trees the
        // recursive printer cannot safely descend into.
        let symbol = alloc::format!("$s{}Si", "Sg".repeat(1_000));
        assert!(parse_bytes(symbol.as_bytes()).is_err());
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn grammar_examples_parse() {
        for symbol in [
            "$s5MyKit5PointVMa",
            "$s5MyKit5PointVMaTwa",
            "$s5MyKit5PointVMn",
            "$s5MyKit5PointVMnTwa",
        ] {
            assert!(parse(symbol).is_ok(), "{symbol}");
        }
    }

    #[test]
    fn malformed_names_are_rejected() {
        for symbol in [
            "$s",
            "$s4",
            "$s4main999foo",
            "$s4main3",
            "$s4main3f",
            "$s4main3foo\x00",
        ] {
            assert!(
                parse_bytes(symbol.as_bytes()).is_err(),
                "{symbol:?} should be rejected"
            );
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use alloc::format;
    use alloc::string::String;

    use super::*;
    use crate::{Budget, Scheme};

    /// Parse `symbol` with `budget` resolved for `scheme`.
    fn parse_with(symbol: &[u8], budget: Budget, scheme: Scheme) -> Result<Ast<'_>, fmt::Error> {
        parse_bytes_with_budget(symbol, budget.effective(scheme))
    }

    /// Whether `symbol` parses with one budget dimension lowered to `value`.
    fn parses_with(
        symbol: &[u8],
        scheme: Scheme,
        dimension: impl Fn(usize) -> Budget,
        value: usize,
    ) -> bool {
        parse_with(symbol, dimension(value), scheme).is_ok()
    }

    /// Assert that `need` is the exact boundary of one dimension for `symbol`.
    ///
    /// The dimension is the only override, so a failure one unit below the
    /// boundary comes from that dimension and not from another limit.
    fn assert_dimension_boundary(
        symbol: &str,
        scheme: Scheme,
        name: &str,
        dimension: impl Fn(usize) -> Budget,
    ) {
        let bytes = symbol.as_bytes();
        let ceiling = bytes.len() + 64;
        let need = (0..=ceiling)
            .find(|value| parses_with(bytes, scheme, &dimension, *value))
            .unwrap_or_else(|| panic!("{symbol}: no {name} budget parses"));
        assert!(need > 0, "{symbol}: {name} boundary is zero");
        assert!(
            parses_with(bytes, scheme, &dimension, need),
            "{symbol}: {name} budget {need} must accept"
        );
        assert!(
            !parses_with(bytes, scheme, &dimension, need - 1),
            "{symbol}: {name} budget {} must reject",
            need - 1
        );
    }

    #[test]
    fn defaults_match_the_compiled_in_limits() {
        let limits = ParseLimits::from_effective(DEFAULT_BUDGET);
        assert_eq!(limits.input, MAX_INPUT_LEN);
        assert_eq!(limits.depth, MAX_DEPTH);
        assert_eq!(limits.work, WORK_BUDGET);
        assert_eq!(limits.nodes, MAX_NODES);
        assert_eq!(limits.expansion_visits, node::DEFAULT_EXPANSION_VISITS);
        assert_eq!(limits.expansion_bytes, node::DEFAULT_EXPANSION_BYTES);
        for scheme in [Scheme::SwiftStable, Scheme::SwiftLegacy] {
            assert_eq!(
                scheme.default_budget().effective(scheme),
                DEFAULT_BUDGET,
                "{scheme}"
            );
        }
    }

    #[test]
    fn each_dimension_rejects_one_unit_below_what_the_symbol_needs() {
        // One stable and one legacy symbol, so both grammars are covered.
        for (symbol, scheme) in [
            ("$s4main3fooyySiF", Scheme::SwiftStable),
            ("_TtC5sugar7MyClass", Scheme::SwiftLegacy),
        ] {
            assert_dimension_boundary(symbol, scheme, "input", |value| {
                Budget::new().with_input_bytes(value)
            });
            assert_dimension_boundary(symbol, scheme, "depth", |value| {
                Budget::new().with_parse_depth(value)
            });
            assert_dimension_boundary(symbol, scheme, "work", |value| {
                Budget::new().with_work(value)
            });
            assert_dimension_boundary(symbol, scheme, "nodes", |value| {
                Budget::new().with_nodes(value)
            });
        }
    }

    /// Assert that `estimate` is the exact boundary of one expansion gate.
    ///
    /// The gate is the parser's own envelope, so the estimate is accepted and
    /// one unit less is rejected. The dimension is the only override.
    fn assert_expansion_boundary(
        symbol: &str,
        scheme: Scheme,
        name: &str,
        estimate: usize,
        dimension: impl Fn(usize) -> Budget,
    ) {
        assert!(
            parses_with(symbol.as_bytes(), scheme, &dimension, estimate),
            "{symbol}: {name} gate {estimate} must accept"
        );
        assert!(
            !parses_with(symbol.as_bytes(), scheme, &dimension, estimate - 1),
            "{symbol}: {name} gate {} must reject",
            estimate - 1
        );
    }

    #[test]
    fn expansion_gates_accept_the_estimate_and_reject_one_unit_below() {
        // Both grammars, one shared-node symbol and one maximal `IndexSubset`,
        // whose text expands when it prints.
        let base = "$s13TangentVector16_Differentiation14DifferentiablePQzAaDQy_SdAFIegnnnr_TJSd";
        let long_subset = format!("{base}{}pSrSUSP", "S".repeat(64));
        for (symbol, scheme) in [
            ("$s4main3fooyySiF", Scheme::SwiftStable),
            ("_TtC5sugar7MyClass", Scheme::SwiftLegacy),
            ("$s4main1AVSDyACAC_G_ACt", Scheme::SwiftStable),
            (long_subset.as_str(), Scheme::SwiftStable),
        ] {
            let ast = parse_with(symbol.as_bytes(), Budget::new(), scheme)
                .unwrap_or_else(|error| panic!("{symbol}: {error:?}"));
            let root = ast.root();
            assert_expansion_boundary(symbol, scheme, "visits", root.print_visits(), |value| {
                Budget::new().with_expansion_visits(value)
            });
            assert_expansion_boundary(symbol, scheme, "bytes", root.print_bytes(), |value| {
                Budget::new().with_expansion_bytes(value)
            });
        }
    }

    #[test]
    fn raising_the_input_budget_accepts_a_longer_symbol() {
        // One byte past the default cap, in the unmangled suffix.
        let base = "$s4main3fooyySiF.";
        let rest = base.len() - 2;
        let symbol = format!("{base}{}", "x".repeat(MAX_INPUT_LEN - rest + 1));
        assert!(parse_bytes(symbol.as_bytes()).is_err());
        let raised = Budget::new().with_input_bytes(2 * MAX_INPUT_LEN);
        assert!(parse_with(symbol.as_bytes(), raised, Scheme::SwiftStable).is_ok());

        // The legacy grammar reads the same dimension.
        let legacy = format!("_TtSi{}", "x".repeat(MAX_INPUT_LEN));
        assert!(parse_bytes(legacy.as_bytes()).is_err());
        assert!(parse_with(legacy.as_bytes(), raised, Scheme::SwiftLegacy).is_ok());
    }

    #[test]
    fn raising_the_parse_depth_accepts_a_deeper_tree() {
        // `h` is an iterative shared-type wrapper and `PA__T` nests the legacy
        // parser's recursion: each adds one level, so both chains exercise the
        // depth dimension only.
        for (symbol, scheme) in [
            (
                format!("$sSi{}", "h".repeat(usize::from(MAX_DEPTH) + 1)),
                Scheme::SwiftStable,
            ),
            (
                format!("_TPA__T{}F3foog3barSi", "PA__T".repeat(100)),
                Scheme::SwiftLegacy,
            ),
        ] {
            assert!(
                parse_bytes(symbol.as_bytes()).is_err(),
                "{symbol}: the default depth must reject"
            );
            let raised = Budget::new().with_parse_depth(usize::MAX);
            assert!(
                parse_with(symbol.as_bytes(), raised, scheme).is_ok(),
                "{symbol}: the hard depth cap must accept"
            );
        }
    }

    #[test]
    fn the_hard_depth_cap_fits_a_modest_stack() {
        // 512 KiB is the stack budget the crate's other recursive parser (D) is
        // tested with. The deepest legacy chain the print envelope accepts is
        // found under that stack, so raising the depth cap cannot recurse past
        // what the parser can safely build.
        let budget = Budget::new().with_parse_depth(usize::MAX);
        let deepest = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                (0..=usize::from(HARD_DEPTH)).rfind(|levels| {
                    let symbol = format!("_TPA__T{}F3foog3barSi", "PA__T".repeat(*levels));
                    parse_with(symbol.as_bytes(), budget, Scheme::SwiftLegacy).is_ok()
                })
            })
            .expect("spawn")
            .join()
            .expect("the depth cap must not overflow a 512 KiB stack");
        assert!(
            deepest.is_some_and(|levels| levels > usize::from(MAX_DEPTH)),
            "the raised cap must reach deeper than the default: {deepest:?}"
        );
    }

    #[test]
    fn clamping_keeps_the_hard_ceilings() {
        let limits = ParseLimits::from_effective(
            Budget::new()
                .with_input_bytes(usize::MAX)
                .with_parse_depth(usize::MAX)
                .with_work(usize::MAX)
                .with_nodes(usize::MAX)
                .with_expansion_visits(usize::MAX)
                .with_expansion_bytes(usize::MAX)
                .effective(Scheme::SwiftStable),
        );
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(limits.nodes, HARD_NODES);
        assert_eq!(limits.expansion_visits, node::HARD_EXPANSION_VISITS);
        assert_eq!(limits.expansion_bytes, node::HARD_EXPANSION_BYTES);

        // A chain past the hard depth stays rejected even when the caller asks
        // for more.
        let deeper = format!("$sSi{}", "h".repeat(usize::from(HARD_DEPTH) + 1));
        let unbounded = Budget::new().with_parse_depth(usize::MAX);
        assert!(parse_with(deeper.as_bytes(), unbounded, Scheme::SwiftStable).is_err());
    }

    #[test]
    fn the_ast_printability_ceiling_is_not_a_budget() {
        // Every parser and expansion dimension raised to its hard ceiling. A
        // chain of `n` shared wrappers nests `n + 3` levels in the tree, so the
        // fixed ceiling admits one wrapper less than the depth cap allows and
        // rejects the next one at parse time.
        let all_raised = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(usize::MAX)
            .with_work(usize::MAX)
            .with_nodes(usize::MAX)
            .with_expansion_visits(usize::MAX)
            .with_expansion_bytes(usize::MAX);
        let scheme = Scheme::SwiftStable;
        let ceiling = usize::from(MAX_PRINT_DEPTH) - 3;
        let accepted = format!("$sSi{}", "h".repeat(ceiling));
        assert!(parse_with(accepted.as_bytes(), all_raised, scheme).is_ok());
        let rejected = format!("$sSi{}", "h".repeat(ceiling + 1));
        assert!(parse_with(rejected.as_bytes(), all_raised, scheme).is_err());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let base = env!("CARGO_MANIFEST_DIR");
        if !std::path::Path::new(base).join("refs/swift").is_dir() {
            std::eprintln!("refs/swift is absent; skipping the Swift budget corpus parity check");
            return;
        }
        let mut compared = 0_usize;
        for file in [
            "refs/swift/manglings.txt",
            "refs/swift/toolchain-6.4.2-manglings.txt",
        ] {
            let Ok(data) = std::fs::read_to_string(format!("{base}/{file}")) else {
                continue;
            };
            for line in data.lines() {
                let symbol = line.split(" ---> ").next().unwrap_or_default().trim();
                if symbol.is_empty() || symbol.starts_with('#') {
                    continue;
                }
                let expected = parse(symbol);
                let explicit = parse_bytes_with_budget(symbol.as_bytes(), DEFAULT_BUDGET);
                assert_eq!(
                    expected.is_ok(),
                    explicit.is_ok(),
                    "{file}: {symbol}: the default budget changes acceptance"
                );
                let Ok(expected) = expected else {
                    continue;
                };
                let mut want = String::new();
                let mut got = String::new();
                expected.print(&mut want).unwrap();
                explicit.unwrap().print(&mut got).unwrap();
                assert_eq!(want, got, "{file}: {symbol}");
                compared += 1;
            }
        }
        assert!(compared > 0, "the default budget compared no symbols");
    }
}
