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
use crate::error::BudgetKind;
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
        parse_bytes_with_budget(symbol, DEFAULT_BUDGET).is_ok() || legacy_claimed_bytes(symbol)
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
pub(crate) enum Encoding {
    /// The postfix grammar of the stable manglings (`$s`, `$S`, `_$s`, `_$S`)
    /// and of Swift 4.0 (`_T0`).
    Stable,
    /// The pre-stable `_T` grammar handled by `legacy.rs`.
    Legacy,
}

/// Why a symbol could not be parsed into an [`Ast`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Failure {
    /// The symbol has no Swift mangling prefix.
    NotSwift,
    /// The symbol body is empty or contains symbolic-reference control bytes,
    /// which are never interpreted for untrusted input.
    SymbolicReference,
    /// The symbol is malformed or uses an unsupported production.
    Invalid,
    /// A budget dimension rejected the symbol: a parser budget (input bytes,
    /// depth, work, nodes), an expansion gate, or the printer's depth ceiling.
    /// [`BudgetKind::Other`] marks work, depth and the printer ceiling, which
    /// the parser does not tell apart.
    Budget(BudgetKind),
}

impl Failure {
    /// The budget dimension, when this is [`Failure::Budget`].
    #[must_use]
    pub(crate) const fn budget_kind(self) -> Option<BudgetKind> {
        match self {
            Self::Budget(kind) => Some(kind),
            _ => None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotSwift => "not a Swift mangled name",
            Self::SymbolicReference => "symbolic references are not interpreted",
            Self::Invalid => "invalid Swift mangling",
            Self::Budget(kind) => return write!(f, "{kind} budget exceeded"),
        })
    }
}

impl core::error::Error for Failure {}

impl From<Failure> for crate::ParseError {
    fn from(failure: Failure) -> Self {
        Self::new(match failure {
            Failure::NotSwift => crate::ParseErrorKind::NotThisScheme,
            Failure::SymbolicReference => crate::ParseErrorKind::Unsupported,
            Failure::Invalid => crate::ParseErrorKind::Invalid,
            Failure::Budget(kind) => crate::ParseErrorKind::Budget(kind),
        })
    }
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
    /// The Swift scheme the symbol used: [`crate::Scheme::SwiftStable`] or
    /// [`crate::Scheme::SwiftLegacy`].
    #[must_use]
    pub const fn scheme(&self) -> crate::Scheme {
        crate::scheme::scheme_for_swift_encoding(self.encoding)
    }

    /// The mangling family behind [`Self::scheme`].
    #[must_use]
    pub(crate) const fn encoding(&self) -> Encoding {
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

impl fmt::Display for Ast<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.print(f)
    }
}

/// Parse `symbol` with one resolved budget, without the expansion gates.
fn parse_inner(symbol: &[u8], limits: ParseLimits) -> Result<Ast<'_>, Failure> {
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
) -> Result<Ast<'a>, Failure> {
    let Some(prefix) = prefix_len_bytes(symbol) else {
        return Err(Failure::NotSwift);
    };
    let rest = symbol.get(prefix..).ok_or(Failure::NotSwift)?;
    // Symbolic references start with control bytes; they must not be
    // interpreted (ABI requirement for untrusted input).
    if rest.iter().any(|&byte| byte < 0x20) || rest.is_empty() {
        return Err(Failure::SymbolicReference);
    }
    // Both grammars read `rest` through a cursor bounded by the input budget;
    // checking it here names the dimension for either one.
    if rest.len() > state.limits.input as usize {
        return Err(Failure::Budget(BudgetKind::Input));
    }
    // A failure is a budget failure when a budget, not the grammar, stopped it.
    let failure = |state: &State| {
        if state.exhausted() {
            Failure::Budget(BudgetKind::Other)
        } else {
            Failure::Invalid
        }
    };
    // `_T` without the `_T0` version marker is the pre-stable prefix grammar;
    // every other prefix uses the postfix grammar.
    let legacy = symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0");
    let root = if legacy {
        legacy::parse(rest, state, limits, nodes).ok_or_else(|| failure(state))?
    } else {
        let (parsed, failed) = {
            let input =
                Cursor::new(rest, 0, state).map_err(|_| Failure::Budget(BudgetKind::Input))?;
            let mut parser = Parser::new(input, symbol.starts_with(b"_T"), limits);
            let parsed = parser.parse();
            let (used, failed) = parser.node_usage();
            *nodes = used;
            if failed {
                parser.exhaust();
            }
            (parsed, failed)
        };
        if failed {
            return Err(Failure::Budget(BudgetKind::Nodes));
        }
        match parsed {
            Ok(root) => root,
            Err(_) => return Err(failure(state)),
        }
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
/// the retained graph again; cache the text if it is printed repeatedly.
///
/// `budget` overrides the compiled-in limits per dimension, exactly as on a
/// [`Demangler`](crate::Demangler): [`Budget::default`](crate::Budget::default)
/// keeps them, and every value is clamped to
/// [`Scheme::hard_budget`](crate::Scheme::hard_budget).
///
/// # Errors
///
/// Returns [`ParseError`](crate::ParseError) when the symbol is not Swift
/// ([`ParseErrorKind::NotThisScheme`](crate::ParseErrorKind::NotThisScheme)), contains symbolic references
/// ([`ParseErrorKind::Unsupported`](crate::ParseErrorKind::Unsupported)), is malformed or unsupported
/// ([`ParseErrorKind::Invalid`](crate::ParseErrorKind::Invalid)), or exceeds a parser budget, the expansion visit
/// or expansion byte gate, or the AST printability gate
/// ([`ParseErrorKind::Budget`](crate::ParseErrorKind::Budget)). The retained tree
/// can therefore be printed repeatedly without a parser failure.
pub fn parse(symbol: &[u8], budget: crate::Budget) -> Result<Ast<'_>, crate::ParseError> {
    let scheme = if symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0") {
        crate::Scheme::SwiftLegacy
    } else {
        crate::Scheme::SwiftStable
    };
    parse_bytes_with_budget(symbol, budget.effective(scheme)).map_err(crate::ParseError::from)
}

/// Parse a raw symbol byte string with one scheme's resolved budget.
///
/// [`parse`] wraps this with [`DEFAULT_BUDGET`]; a [`crate::Demangler`]
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
/// Returns [`Failure`] for the same reasons as [`parse`], including a
/// symbol outside the expansion gates.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, Failure> {
    let limits = ParseLimits::from_effective(budget);
    let parsed = parse_inner(symbol, limits)?;
    let root = parsed.root();
    // The printer has no caps, so the parser is the only place that can reject
    // a tree it could not render: the two expansion gates bound a DAG's
    // expanded output and the fixed ceiling bounds the printer's recursion.
    if root.print_visits() > limits.expansion_visits() {
        return Err(Failure::Budget(BudgetKind::ExpansionVisits));
    }
    if root.print_bytes() > limits.expansion_bytes() {
        return Err(Failure::Budget(BudgetKind::ExpansionBytes));
    }
    if root.print_depth() > MAX_PRINT_DEPTH {
        return Err(Failure::Budget(BudgetKind::Other));
    }
    Ok(parsed)
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
mod tests;

#[cfg(test)]
mod bytes_tests;

#[cfg(test)]
mod coverage_tests;

#[cfg(test)]
mod budget_tests;
