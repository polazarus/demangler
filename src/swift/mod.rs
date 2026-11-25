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

pub use node::{
    Accessor, AttributeKind, AutoDiffCode, DeinitializerKind, GenericSpecializationKind, Global,
    GlobalOperand, KeyPathKind, Kind, LayoutCode, MacroRole, Node, NodeData, NodeRef, ResumeStyle,
};

use crate::parse::{Cursor, Limits, State};
use parser::Parser;
use printer::Printer;

/// Maximum symbol bytes one parse may read.
///
/// Swift was the only backend without an input cap; the largest corpus symbol
/// is a few hundred bytes.
pub(crate) const MAX_INPUT_LEN: usize = 64 * 1024;

/// Maximum operator recursion depth.
pub(crate) const MAX_DEPTH: u16 = 96;

/// Maximum input-work units one parse may consume.
pub(crate) const WORK_BUDGET: u32 = 200_000;

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
    if symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0") {
        parse_bytes(symbol).ok()
    } else {
        None
    }
}

/// Parser budgets shared by the stable and legacy grammars.
const fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH,
        work: WORK_BUDGET,
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

fn parse_inner(symbol: &[u8], payload_depth: u8) -> Result<Ast<'_>, fmt::Error> {
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
    let mut state = State::new(limits());
    let root = if legacy {
        legacy::parse(rest, payload_depth, &mut state).ok_or(fmt::Error)?
    } else {
        let input = Cursor::new(rest, 0, &mut state).map_err(|_| fmt::Error)?;
        let mut parser = Parser::new(input, symbol.starts_with(b"_T"));
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
/// the retained graph again; cache the text if it is printed repeatedly.
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
/// [module docs](self)); parsing itself never requires a text decoding.
///
/// # Errors
///
/// Returns [`fmt::Error`] for the same reasons as [`parse`], including a
/// symbol longer than the input budget.
pub(crate) fn parse_bytes(symbol: &[u8]) -> Result<Ast<'_>, fmt::Error> {
    let parsed = parse_inner(symbol, 0)?;
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
    parse_inner(symbol, 0)?.print(out)
}

/// Demangle a symbol nested inside a legacy specialization payload.
///
/// The reference printer re-demangles payload text. `depth` bounds how often
/// that may nest; callers also bound the payload length.
pub(crate) fn demangle_payload(symbol: &str, depth: u8) -> Option<alloc::string::String> {
    let ast = parse_inner(symbol.as_bytes(), depth.checked_add(1)?).ok()?;
    let mut out = alloc::string::String::new();
    ast.print(&mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use alloc::borrow::{Cow, ToOwned};
    use alloc::rc::Rc;
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
            let mut out = alloc::string::String::new();
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
