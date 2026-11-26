//! Pure-Rust Swift symbol demangling.
//!
//! The demangler is a postfix operator machine that builds an immutable
//! `Rc`-owned node graph, followed by a printer that writes readable text into
//! any [`core::fmt::Write`] sink. A node is built once and never mutated, and
//! parsed graphs are retained so callers can render them repeatedly without
//! re-parsing. Shared nodes make the graph a DAG, so printing is bounded by
//! depth, node-visit and output-byte budgets.
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
//! Printing is not a budget: the fixed envelope (printer visits, depth and
//! output bytes) is checked at parse time and no [`crate::Budget`] field
//! configures it. Raising the parse depth is therefore clamped to the depth the
//! envelope can render.
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

use crate::budget::{EffectiveBudget, Limit};
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

/// Hard depth ceiling.
///
/// The print envelope rejects a tree nested deeper than
/// [`printer::DEPTH_CAP`], so accepting more parser depth would only build a
/// tree the printer cannot render.
const HARD_DEPTH: u16 = printer::DEPTH_CAP as u16;

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
pub(crate) fn parse_legacy_if_with(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Option<Ast<'_>> {
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
    // The policy-table expansion gates; no parser reads them yet.
    expansion_visits: Some(Limit::Bounded(1 << 17)),
    expansion_bytes: Some(Limit::Bounded(1 << 20)),
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
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The cursor enforces the input length, the recursion depth of the
    /// operator machine and the work units; the node budget stays with
    /// [`node::Context`](node) and the iterative wrapper counters.
    const fn cursor(&self) -> Limits {
        // The input dimension is clamped to `HARD_INPUT_LEN`, so it fits a
        // `u32` whatever the caller asked for.
        Limits {
            input: self.input as u32,
            depth: self.depth,
            work: self.work,
        }
    }

    /// Maximum operator recursion depth.
    const fn depth(&self) -> u16 {
        self.depth
    }

    /// Maximum nodes one parse may allocate.
    const fn nodes(&self) -> usize {
        self.nodes
    }
}

/// One budget dimension after clamping to the compiled-in default and ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
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

/// Parse `symbol` with one resolved budget, without the print envelope.
///
/// `payload_depth` counts how often a legacy payload symbol may nest.
fn parse_inner(
    symbol: &[u8],
    payload_depth: u8,
    limits: ParseLimits,
) -> Result<Ast<'_>, fmt::Error> {
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
    let mut state = State::new(limits.cursor());
    let root = if legacy {
        legacy::parse(rest, payload_depth, &mut state, limits).ok_or(fmt::Error)?
    } else {
        let input = Cursor::new(rest, 0, &mut state).map_err(|_| fmt::Error)?;
        let mut parser = Parser::new(input, symbol.starts_with(b"_T"), limits);
        parser.parse().map_err(|_| fmt::Error)?
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

/// Parse `symbol` into a reusable graph and check that it is renderable.
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
/// printer envelope (expanded visits, depth or output bytes). The retained
/// tree can therefore be printed repeatedly without a parser failure.
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
/// Printing is not part of the budget: the fixed envelope below still rejects a
/// tree the printer cannot render, however far the parser budgets are raised.
pub(crate) fn parse_bytes_with_budget(
    symbol: &[u8],
    budget: EffectiveBudget,
) -> Result<Ast<'_>, fmt::Error> {
    let parsed = parse_inner(symbol, 0, ParseLimits::from_effective(budget))?;
    let root = parsed.root();
    if root.print_visits() > printer::NODE_CAP
        || root.print_depth() > u16::from(printer::DEPTH_CAP)
        || root.print_bytes() > printer::OUTPUT_CAP
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
pub(crate) fn print_bytes<W: fmt::Write + ?Sized>(symbol: &[u8], out: &mut W) -> fmt::Result {
    parse_inner(symbol, 0, ParseLimits::from_effective(DEFAULT_BUDGET))?.print(out)
}

/// Demangle a symbol nested inside a legacy specialization payload.
///
/// The reference printer re-demangles payload text. `depth` bounds how often
/// that may nest; callers also bound the payload length. The payload parse
/// inherits the outer parse's budget.
pub(crate) fn demangle_payload(
    symbol: &str,
    depth: u8,
    limits: ParseLimits,
) -> Option<alloc::string::String> {
    let ast = parse_inner(symbol.as_bytes(), depth.checked_add(1)?, limits).ok()?;
    let mut out = alloc::string::String::new();
    ast.print(&mut out).ok()?;
    Some(out)
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
        for symbol in ["$s4main3fooyySiF", "_TtSi", "_TtGSqGSaC5sugar7MyClass__"] {
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
                    Err(error) => panic!("{symbol}: print failed after envelope: {error:?}"),
                };
                let root = ast.root();
                assert!(
                    visits <= root.print_visits(),
                    "{symbol}: {visits} visits > {}",
                    root.print_visits()
                );
                assert!(
                    u16::from(depth) <= root.print_depth(),
                    "{symbol}: depth {depth} > {}",
                    root.print_depth()
                );
                assert!(
                    bytes <= root.print_bytes(),
                    "{symbol}: {bytes} bytes > {}",
                    root.print_bytes()
                );
            }
        }
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
        // The parse-time render check keeps rejecting trees the printer's
        // depth budget cannot render.
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
                .effective(Scheme::SwiftStable),
        );
        assert_eq!(limits.input, HARD_INPUT_LEN);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(limits.nodes, HARD_NODES);

        // A chain past the hard depth stays rejected even when the caller asks
        // for more.
        let deeper = format!("$sSi{}", "h".repeat(usize::from(HARD_DEPTH) + 1));
        let unbounded = Budget::new().with_parse_depth(usize::MAX);
        assert!(parse_with(deeper.as_bytes(), unbounded, Scheme::SwiftStable).is_err());
    }

    #[test]
    fn the_print_envelope_is_not_a_budget() {
        // Every parser dimension raised to its hard ceiling. A chain of `n`
        // shared wrappers nests `n + 3` levels in the tree, so the fixed
        // envelope admits one wrapper less than the depth cap allows and
        // rejects the next one at parse time.
        let all_raised = Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(usize::MAX)
            .with_work(usize::MAX)
            .with_nodes(usize::MAX);
        let scheme = Scheme::SwiftStable;
        let envelope = usize::from(printer::DEPTH_CAP) - 3;
        let accepted = format!("$sSi{}", "h".repeat(envelope));
        assert!(parse_with(accepted.as_bytes(), all_raised, scheme).is_ok());
        let rejected = format!("$sSi{}", "h".repeat(envelope + 1));
        assert!(parse_with(rejected.as_bytes(), all_raised, scheme).is_err());
    }

    #[test]
    fn the_default_budget_parses_the_corpus_like_parse() {
        let base = env!("CARGO_MANIFEST_DIR");
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
