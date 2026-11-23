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
//! [`crate::swift::parse`] returns an [`crate::swift::Ast`] whose
//! [`crate::swift::Node`] views expose kind, text and
//! children without leaking the internal `RefCell` borrows. The tree is
//! `Rc`-owned and is therefore `!Send + !Sync`.

mod legacy;
mod node;
mod parser;
mod printer;
mod text;

use alloc::borrow::Cow;
use core::fmt;

pub use node::Kind;

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
    prefix_len(symbol).is_some()
}

/// Parser budgets shared by the stable and legacy grammars.
const fn limits() -> Limits {
    Limits {
        input: MAX_INPUT_LEN as u32,
        depth: MAX_DEPTH,
        work: WORK_BUDGET,
    }
}

/// A parsed Swift symbol with a read-only node graph.
///
/// The graph is retained and can be printed more than once without parsing the
/// mangled name again. It is `Rc`-owned, so `Ast` is `!Send + !Sync`.
#[derive(Clone, Debug)]
pub struct Ast<'a> {
    root: Node<'a>,
}

impl<'a> Ast<'a> {
    /// The root node.
    #[must_use]
    pub fn root(&self) -> Node<'a> {
        self.root.clone()
    }

    /// Render the retained graph into `out`.
    pub(crate) fn print<W: fmt::Write + ?Sized>(&self, out: &mut W) -> fmt::Result {
        Printer::new(out).print(self.root.0.clone())
    }
}

/// A read-only view of one Swift AST node.
///
/// The view holds a shared handle; cloning it only bumps a reference count.
/// Children are returned as owned views, so no `RefCell` borrow escapes the
/// API.
#[derive(Clone, Debug)]
pub struct Node<'a>(node::NodeRef<'a>);

impl<'a> Node<'a> {
    /// The node kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.0.kind()
    }

    /// The node's text, if it has any.
    ///
    /// Borrowed text stays borrowed; reconstructed text is cloned. Identifier
    /// bytes that were not valid UTF-8 are escaped as `\xNN` in the returned
    /// text.
    #[must_use]
    pub fn text(&self) -> Option<Cow<'a, str>> {
        self.0.text()
    }

    /// Run `f` with the node's text borrowed for the duration of the call.
    ///
    /// This avoids cloning reconstructed text and never exposes the underlying
    /// `RefCell` guard.
    pub fn with_text<R>(&self, f: impl FnOnce(Option<&str>) -> R) -> R {
        self.0.with_text(f)
    }

    /// The node's children in order.
    ///
    /// The returned view keeps the internal child borrow alive but never
    /// exposes the `RefCell` guard itself.
    #[must_use]
    pub fn children(&self) -> Children<'a, '_> {
        Children {
            inner: self.0.children(),
            index: 0,
        }
    }

    /// The child at `index`, if present.
    #[must_use]
    pub fn child(&self, index: usize) -> Option<Self> {
        self.0.child(index).map(Self)
    }

    /// The number of children.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.0.child_count()
    }

    /// True when the node has no children.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        self.0.child_count() == 0
    }
}

/// A read-only iterator over a node's children.
pub struct Children<'a, 'b> {
    inner: core::cell::Ref<'b, [node::NodeRef<'a>]>,
    index: usize,
}

impl<'a> Iterator for Children<'a, '_> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let child = self.inner.get(self.index)?.clone();
        self.index += 1;
        Some(Node(child))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.inner.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Children<'_, '_> {}

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
    let mut state = State::new(limits());
    let root = if symbol.starts_with(b"_T") && !symbol.starts_with(b"_T0") {
        legacy::parse(rest, payload_depth, &mut state).ok_or(fmt::Error)?
    } else {
        let input = Cursor::new(rest, 0, &mut state).map_err(|_| fmt::Error)?;
        let mut parser = Parser::new(input, symbol.starts_with(b"_T"));
        parser.parse().map_err(|_| fmt::Error)?
    };
    Ok(Ast {
        root: Node(root),
    })
}

/// Parse `symbol` into a reusable graph and check that it is renderable.
///
/// # Errors
///
/// Returns [`fmt::Error`] when the symbol is not Swift, uses an unsupported
/// prefix, contains symbolic references, or fails to parse. The retained tree
/// is validated by printing it once into a null sink, so [`Ast`] can be printed
/// repeatedly without a parser failure.
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

/// Validation sink used by [`parse`].
struct NullSink;

impl fmt::Write for NullSink {
    fn write_str(&mut self, _s: &str) -> fmt::Result {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
                texts.push(text.into_owned());
            }
            stack.extend(node.children());
            assert_eq!(node.is_leaf(), node.child_count() == 0);
        }

        assert!(texts.iter().any(|text| text == "main"));
        assert!(texts.iter().any(|text| text == "foo"));
    }

    #[test]
    fn ast_clone_shares_the_tree() {
        let ast = parse("$s4main3fooyySiF").unwrap();
        let cloned = ast.clone();
        assert_eq!(ast.root().kind(), cloned.root().kind());
        assert_eq!(
            ast.root().child_count(),
            cloned.root().child_count()
        );
    }

    #[test]
    fn ast_text_can_be_borrowed() {
        let ast = parse("$s4main3fooyySiF").unwrap();
        let mut stack = std::vec![ast.root()];
        let mut node = None;
        while let Some(candidate) = stack.pop() {
            if candidate.kind() == Kind::Identifier
                && candidate.with_text(|text| text == Some("foo"))
            {
                node = Some(candidate);
                break;
            }
            stack.extend(candidate.children());
        }
        let node = node.unwrap();
        node.with_text(|text| assert_eq!(text, Some("foo")));
        assert!(matches!(node.text(), Some(Cow::Borrowed(_))));
    }
}

#[cfg(test)]
mod bytes {
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
        // Valid UTF-8 is not escaped, so output for accepted symbols does not
        // move; a backslash is an ordinary valid ASCII byte.
        assert_eq!(
            render_bytes(b"$s4main3a\\b").as_deref(),
            Some("maina\\b")
        );
        let ast = parse("$s4main5f\u{f6}\u{f6}").expect("demangles");
        let mut stack = std::vec![ast.root()];
        let mut borrowed = 0_usize;
        while let Some(node) = stack.pop() {
            if let Some(text) = node.text() {
                assert!(matches!(text, Cow::Borrowed(_)));
                borrowed += 1;
            }
            stack.extend(node.children());
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
