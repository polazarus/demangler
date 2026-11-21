//! AST node model, spans and parsing budgets.
//!
//! One immutable node graph serves both rustc encodings: the v0 front-end
//! fills the path/type/const vocabulary and the legacy front-end contributes
//! [`Kind::LegacyPath`]/[`Kind::LegacyElement`]. Children are sealed at
//! construction and nodes are `Rc`-shared, so v0 backrefs make the graph a DAG
//! rather than a tree.
//!
//! [`Node`] is only built by the parsers, which are also the only place where
//! the node and work budgets are enforced.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;

use super::ParseError;

/// Maximum nesting depth of the parser.
///
/// Matches `rustc-demangle`'s `MAX_DEPTH`, so the in-tree parser accepts
/// exactly the nesting the reference decoder accepts. Deeper input is rejected
/// with [`ParseError::RecursedTooDeep`] instead of overflowing the stack.
pub const MAX_DEPTH: u32 = 500;

/// Maximum number of nodes a single symbol may produce.
pub const MAX_NODES: usize = 0x0001_0000;

/// Maximum number of parser work units (production entries, nodes created and
/// backrefs resolved).
pub const MAX_WORK: usize = 200_000;

/// Maximum depth the printer descends into the graph.
pub const MAX_PRINT_DEPTH: u32 = 512;

/// Maximum number of node visits while printing one symbol.
pub const MAX_PRINT_VISITS: usize = 1_000_000;

/// Maximum number of bytes the printer emits for one symbol.
pub const MAX_OUTPUT_BYTES: usize = 1_000_000;

/// Maximum accepted input length.
///
/// Every parser allocation is proportional to the input: the v0 offset cache,
/// the legacy element list and the node vectors. Capping the input at 64 KiB
/// keeps those allocations small even when the caller passes untrusted data;
/// real mangled names are far shorter.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// A byte range in the original symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// First byte of the range.
    pub start: u32,
    /// One past the last byte of the range.
    pub end: u32,
}

impl Span {
    /// The number of bytes in the range.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    /// True when the range covers no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// The namespace tag of a v0 nested path (`N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// `C`: a closure.
    Closure,
    /// `S`: a shim.
    Shim,
    /// Any other uppercase tag, printed verbatim inside `{...}`.
    Other(char),
    /// A lowercase tag: implementation-internal, hidden when the name is empty.
    Internal,
}

impl Namespace {
    /// The tag character as encoded.
    #[must_use]
    pub const fn tag(&self) -> char {
        match self {
            Self::Closure => 'C',
            Self::Shim => 'S',
            Self::Other(tag) => *tag,
            Self::Internal => 'a',
        }
    }
}

/// Which v0 impl production a [`Kind::Impl`] node came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImplKind {
    /// `M`: an inherent impl (`<Self>`).
    Inherent,
    /// `X`: a trait impl (`<Self as Trait>`).
    TraitImpl,
    /// `Y`: a trait definition (`<Self as Trait>`).
    TraitDef,
}

/// The value shape of a [`Kind::ConstValue`] node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstValueKind {
    /// An integer literal, `["n"] {hex-digit} "_"`; the node text is the
    /// nibble string and the const's type is its sibling type node.
    Integer {
        /// True when the encoding carried the `n` (negative) marker.
        negative: bool,
    },
    /// The placeholder const `p`, printed `_`.
    Placeholder,
    /// A `b` boolean literal.
    Bool(bool),
    /// A `c` char literal; the node text is the character.
    Char,
    /// An `e` `str` literal, printed `*"..."`; the node text holds the
    /// characters.
    Str,
    /// An `Re` `&str` literal, printed `"..."`; the node text holds the
    /// characters.
    StrRef,
    /// An `R`/`Q` const reference; children: `[value const]`.
    Ref {
        /// True for `Q` (`&mut`), false for `R` (`&`).
        mutability: bool,
    },
    /// An `A` const array; children: the element values.
    Array,
    /// A `T` const tuple; children: the element values.
    Tuple,
    /// A `V` ADT const; see [`AdtConstKind`] for its children.
    Adt(AdtConstKind),
}

/// The shape of a `V` ADT const, which decides the value's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdtConstKind {
    /// `U`: a unit const, printed as the path alone; children: `[path]`.
    Unit,
    /// `T`: a tuple const, printed with `(...)`; children:
    /// `[path, value...]`.
    Tuple,
    /// `S`: a struct const, printed with `{ name: value, ... }`; children:
    /// `[path, field...]`, where every field is a [`Kind::AdtField`].
    Struct,
}

/// The shape of a [`Kind::Pattern`] node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternKind {
    /// `R`: an inclusive range; children: `[low const, high const]`.
    Range,
    /// `O`: an or-pattern; children: the alternative patterns.
    Or,
    /// `N`: `!null`, with no children.
    NeverNull,
}

/// A typed node payload that is not a child node.
///
/// Every payload the implemented grammar produces lives here; accessors on
/// [`Node`] are convenience reads, not a parallel set of fields. The lifetime
/// is the symbol's: function ABI names borrow the input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NodeData<'a> {
    /// No payload.
    None,
    /// A disambiguator (`s` + base-62 + 1) on [`Kind::Ident`] or
    /// [`Kind::CrateRoot`].
    Disambiguator(u64),
    /// The namespace tag of a [`Kind::Nested`] path.
    Namespace(Namespace),
    /// Which of `M`, `X`, `Y` a [`Kind::Impl`] came from.
    Impl(ImplKind),
    /// `true` for `&mut`/`*mut`, `false` for shared references and `*const`.
    Mutability(bool),
    /// The base-62 index of a [`Kind::Lifetime`].
    Lifetime(u64),
    /// `true` when a [`Kind::LegacyElement`] is the trailing rustc hash.
    Hash(bool),
    /// The value shape of a [`Kind::ConstValue`].
    Const(ConstValueKind),
    /// The late-bound lifetime count of the `G` binder on a [`Kind::Dyn`]
    /// node; zero means the encoding carried no binder.
    Binder(u64),
    /// A [`Kind::Fn`] signature.
    Function {
        /// The late-bound lifetime count of the `G` binder; zero means none.
        binder: u64,
        /// True for `U`, printed `unsafe `.
        is_unsafe: bool,
        /// The ABI name of `K`, printed inside `extern "..."`; `None` when
        /// the encoding carried no `K`.
        abi: Option<Cow<'a, str>>,
        /// True when the return type was the bare `u` tag, which the
        /// reference prints as nothing instead of ` -> ()`.
        ret_elided: bool,
    },
    /// The shape of a [`Kind::Pattern`].
    Pattern(PatternKind),
}

/// The kind of a Rust AST node.
///
/// The vocabulary is shared by both encodings and grows with the grammar the
/// parser implements. Child order is part of the contract and documented per
/// variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A v0 crate root (`C`).
    ///
    /// No children; `text` is the crate name and [`NodeData::Disambiguator`]
    /// holds its disambiguator.
    CrateRoot,
    /// A v0 nested path (`N`).
    ///
    /// Children: `[parent path, ident]`; [`NodeData::Namespace`] holds the
    /// namespace tag.
    Nested,
    /// A v0 impl (`M`, `X`, `Y`); [`NodeData::Impl`] says which.
    ///
    /// Children: `[impl path, self type]` for `M`,
    /// `[impl path, self type, trait path]` for `X` and
    /// `[self type, trait path]` for `Y`.
    Impl,
    /// A v0 generic argument list (`I`).
    ///
    /// Children: `[base path, argument...]`.
    GenericArgs,
    /// The v0 instantiating crate: parsed for validation, never printed.
    ///
    /// Children: `[path]`.
    InstantiatingCrate,
    /// A v0 identifier.
    ///
    /// No children; `text` is the name and [`NodeData::Disambiguator`] holds
    /// the disambiguator that the enclosing path may print.
    Ident,
    /// A v0 basic type leaf; `text` is the printed type name.
    BasicType,
    /// A v0 array type (`A`).
    ///
    /// Children: `[element type, length const]`.
    Array,
    /// A v0 slice type (`S`).
    ///
    /// Children: `[element type]`.
    Slice,
    /// A v0 tuple type (`T`).
    ///
    /// Children: the element types, in order.
    Tuple,
    /// A v0 reference type (`R`, `Q`); [`NodeData::Mutability`] says which.
    ///
    /// Children: an optional [`Kind::Lifetime`] followed by the pointee type.
    Ref,
    /// A v0 raw pointer type (`P`, `O`); [`NodeData::Mutability`] says which.
    ///
    /// Children: `[pointee type]`.
    RawPtr,
    /// A v0 lifetime (`L`); [`NodeData::Lifetime`] holds the base-62 index.
    Lifetime,
    /// A v0 fn pointer type (`F`).
    ///
    /// Children: the parameter types followed by the return type;
    /// [`NodeData::Function`] holds the binder, `unsafe`, ABI and return
    /// elision.
    Fn,
    /// A v0 `dyn` trait object type (`D`).
    ///
    /// Children: the [`Kind::DynTrait`]s followed by the trailing lifetime;
    /// [`NodeData::Binder`] holds the binder's lifetime count.
    Dyn,
    /// One trait of a `dyn` trait object.
    ///
    /// Children: `[path, binding...]`. When the path is a
    /// [`Kind::GenericArgs`] node, the bindings extend its argument list, so
    /// they print inside the same `<...>`.
    DynTrait,
    /// One associated binding of a `dyn` trait (`p`).
    ///
    /// Children: `[name ident, value]`, where the value is a [`Kind::Const`]
    /// for a `K` binding and a type otherwise.
    DynAssocBinding,
    /// A v0 pattern type (`W`), printed `<type> is <pattern>`.
    ///
    /// Children: `[type, pattern]`.
    PatternType,
    /// The unstable splat marker (`w`) before a type.
    ///
    /// Children: `[type]`; printed `#[rustc_splat] ` followed by the type.
    /// The marker is not in RFC 2603 but is emitted by the unstable `splat`
    /// feature, and the pinned reference decoder accepts it.
    Splat,
    /// A v0 pattern; [`NodeData::Pattern`] says which shape it is.
    ///
    /// Children: `[low, high]` consts for a range, the alternative patterns
    /// for an or-pattern and none for `!null`.
    Pattern,
    /// One field of an `S` (struct) ADT const.
    ///
    /// Children: `[name ident, value const]`.
    AdtField,
    /// A v0 const: a `K` generic argument, an array length, a `dyn` binding
    /// value or a pattern bound.
    ///
    /// Children: `[value]` for every shape except an integer literal, which
    /// has `[type, value]`.
    Const,
    /// A v0 const value.
    ///
    /// [`NodeData::Const`] says which shape it is, `text` holds the decoded
    /// characters of a char or string literal and the hex nibbles of an
    /// integer literal, and compound shapes keep their operands as children.
    ConstValue,
    /// A legacy path (`_ZN...E`).
    ///
    /// Children: the [`Kind::LegacyElement`]s, in order.
    LegacyPath,
    /// One legacy path element.
    ///
    /// No children; `text` is the unescaped element and [`NodeData::Hash`]
    /// marks the trailing rustc hash.
    LegacyElement,
}

/// A shared handle to an AST node.
///
/// Cloning a `NodeRef` only bumps a reference count; v0 backrefs hand out the
/// same handle for every reference to one encoded position.
pub type NodeRef<'a> = Rc<Node<'a>>;

/// One immutable AST node.
///
/// Built only by the parsers (which also enforce the budgets), so the graph is
/// acyclic by construction and every child slice is sealed.
#[derive(Debug)]
pub struct Node<'a> {
    kind: Kind,
    text: Option<Cow<'a, str>>,
    span: Option<Span>,
    children: Box<[NodeRef<'a>]>,
    data: NodeData<'a>,
}

impl<'a> Node<'a> {
    /// The node kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's text, if it has any.
    ///
    /// Identifiers, basic types and legacy elements borrow the input; text
    /// that had to be reconstructed is owned.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// The byte range this node was parsed from, if it has one.
    ///
    /// Backref targets keep the span of the bytes they were first parsed from.
    #[must_use]
    pub const fn span(&self) -> Option<Span> {
        self.span
    }

    /// The node's children, in the order documented on its [`Kind`].
    #[must_use]
    pub fn children(&self) -> &[NodeRef<'a>] {
        &self.children
    }

    /// The typed payload.
    #[must_use]
    pub const fn data(&self) -> &NodeData<'a> {
        &self.data
    }

    /// The disambiguator of an [`Kind::Ident`] or [`Kind::CrateRoot`].
    #[must_use]
    pub const fn disambiguator(&self) -> Option<u64> {
        match self.data {
            NodeData::Disambiguator(value) => Some(value),
            _ => None,
        }
    }

    /// The namespace tag of a [`Kind::Nested`] path.
    #[must_use]
    pub const fn namespace(&self) -> Option<Namespace> {
        match self.data {
            NodeData::Namespace(namespace) => Some(namespace),
            _ => None,
        }
    }

    /// The base-62 index of a [`Kind::Lifetime`].
    #[must_use]
    pub const fn lifetime_index(&self) -> Option<u64> {
        match self.data {
            NodeData::Lifetime(index) => Some(index),
            _ => None,
        }
    }
}

/// Node factory that charges the node and work budgets.
pub(super) struct Builder {
    nodes: usize,
    work: usize,
}

impl<'a> Builder {
    /// A builder with no nodes charged yet.
    pub(super) const fn new() -> Self {
        Self { nodes: 0, work: 0 }
    }

    /// Charge `work` units, failing when the work budget is exhausted.
    pub(super) fn charge(&mut self, work: usize) -> Result<(), ParseError> {
        self.work = self.work.saturating_add(work);
        if self.work > MAX_WORK {
            return Err(ParseError::BudgetExceeded);
        }
        Ok(())
    }

    /// Create a node, charging one node and one work unit.
    pub(super) fn node(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, str>>,
        span: Option<Span>,
        data: NodeData<'a>,
        children: alloc::vec::Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err(ParseError::BudgetExceeded);
        }
        self.charge(1)?;
        Ok(Rc::new(Node {
            kind,
            text,
            span,
            children: children.into_boxed_slice(),
            data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::rust::parse;

    #[test]
    fn accessors_read_the_payload() {
        let ast = parse("_RNvC3foo3bar").unwrap();
        let root = ast.root();
        assert_eq!(root.kind(), Kind::Nested);
        assert_eq!(root.namespace(), Some(Namespace::Internal));
        assert_eq!(root.text(), None);
        assert_eq!(root.disambiguator(), None);

        let ident = root.children().get(1).unwrap();
        assert_eq!(ident.kind(), Kind::Ident);
        assert_eq!(ident.text(), Some("bar"));
        assert_eq!(ident.disambiguator(), Some(0));
        assert!(ident.children().is_empty());
    }

    #[test]
    fn span_helpers() {
        let span = Span { start: 3, end: 7 };
        assert_eq!(span.len(), 4);
        assert!(!span.is_empty());
        assert!(Span { start: 3, end: 3 }.is_empty());
    }

    #[test]
    fn namespace_tags_round_trip() {
        assert_eq!(Namespace::Closure.tag(), 'C');
        assert_eq!(Namespace::Shim.tag(), 'S');
        assert_eq!(Namespace::Other('X').tag(), 'X');
        assert_eq!(Namespace::Internal.tag(), 'a');
    }

    #[test]
    fn lifetime_index_is_exposed() {
        let ast = parse("_RIC0L_E").unwrap();
        let lifetime = ast.root().children().get(1).unwrap();
        assert_eq!(lifetime.kind(), Kind::Lifetime);
        assert_eq!(lifetime.lifetime_index(), Some(0));
    }

    /// Spans stay inside the input, and a depth-first walk terminates: a node
    /// is never its own ancestor. Repeated nodes are DAG sharing, not a cycle.
    #[test]
    fn graph_invariants() {
        for symbol in [
            "_ZN3foo3barE",
            "_RC3foo",
            "_RMC0TTTTTTpB8_EB7_EB6_EB5_EB4_EB3_E",
            "_RNCNCNgCs6DXkGYLi8lr_2cc5spawn00B5_",
            "_RIC0hKj4_E",
        ] {
            let ast = parse(symbol).unwrap();
            let mut on_path: Vec<*const Node<'_>> = Vec::new();
            let mut stack = vec![(ast.root().clone(), 0_usize)];
            let mut entered = 0_usize;
            while let Some(top) = stack.last_mut() {
                let node = top.0.clone();
                let index = top.1;
                if index == 0 {
                    let pointer = Rc::as_ptr(&node);
                    assert!(
                        !on_path.contains(&pointer),
                        "{symbol}: node is its own ancestor"
                    );
                    on_path.push(pointer);
                    entered += 1;
                    assert!(entered < 100_000, "{symbol}: walk did not terminate");
                    if let Some(span) = node.span() {
                        assert!(span.start <= span.end, "{symbol}: inverted span");
                        assert!(
                            span.end as usize <= symbol.len(),
                            "{symbol}: span past the input"
                        );
                        assert!(symbol.is_char_boundary(span.start as usize));
                        assert!(symbol.is_char_boundary(span.end as usize));
                    }
                }
                match node.children().get(index).cloned() {
                    Some(child) => {
                        top.1 += 1;
                        stack.push((child, 0));
                    }
                    None => {
                        let _ = on_path.pop();
                        let _ = stack.pop();
                    }
                }
            }
            assert!(entered > 0, "{symbol}: empty graph");
        }
    }

    // The node budget fires before the work budget for node-only growth, and
    // the print budgets sit above the parse depth limit.
    const _: () = assert!(MAX_NODES < MAX_WORK);
    const _: () = assert!(MAX_DEPTH <= MAX_PRINT_DEPTH);
}
