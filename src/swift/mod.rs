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

use alloc::borrow::Cow;
use core::fmt;

pub use node::Kind;

use parser::Parser;
use printer::Printer;

/// Recognise a supported Swift mangling prefix, returning its length.
#[must_use]
pub fn prefix_len(symbol: &str) -> Option<usize> {
    if symbol.starts_with("$s") || symbol.starts_with("$S") {
        Some(2)
    } else if symbol.starts_with("_$s") || symbol.starts_with("_$S") {
        // Mach-O prefixes symbols with an underscore.
        Some(3)
    } else if symbol.starts_with("_T0") {
        Some(3)
    } else if symbol.starts_with("_T") {
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
    /// Borrowed text stays borrowed; reconstructed text is cloned.
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

fn parse_inner(symbol: &str, payload_depth: u8) -> Result<Ast<'_>, fmt::Error> {
    let Some(prefix) = prefix_len(symbol) else {
        return Err(fmt::Error);
    };
    let rest = symbol.get(prefix..).ok_or(fmt::Error)?;
    // Symbolic references start with control bytes; they must not be
    // interpreted (ABI requirement for untrusted input).
    if rest.bytes().any(|byte| byte < 0x20) || rest.is_empty() {
        return Err(fmt::Error);
    }
    // `_T` without the `_T0` version marker is the pre-stable prefix grammar;
    // every other prefix uses the postfix grammar.
    let root = if symbol.starts_with("_T") && !symbol.starts_with("_T0") {
        legacy::parse(rest, payload_depth).ok_or(fmt::Error)?
    } else {
        let mut parser = Parser::new(rest.as_bytes(), symbol.starts_with("_T"));
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
    parse_inner(symbol, 0)?.print(out)
}

/// Demangle a symbol nested inside a legacy specialization payload.
///
/// The reference printer re-demangles payload text. `depth` bounds how often
/// that may nest; callers also bound the payload length.
pub(crate) fn demangle_payload(symbol: &str, depth: u8) -> Option<alloc::string::String> {
    let ast = parse_inner(symbol, depth.checked_add(1)?).ok()?;
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
