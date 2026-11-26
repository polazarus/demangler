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
//!
//! Node text is raw bytes, not `str`: a symbol name is a byte string, so an
//! identifier may carry bytes that are not UTF-8. The printer decides how those
//! bytes are rendered (see [`crate::rust::printer`]).

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;

use crate::budget::{EffectiveBudget, Limit};
use crate::parse::Limits;

use super::ParseError;

/// Default maximum nesting depth of the parser.
///
/// Matches `rustc-demangle`'s `MAX_DEPTH`, so at the default budget the
/// in-tree parser accepts exactly the nesting the reference decoder accepts.
/// Deeper input is rejected with [`ParseError::RecursedTooDeep`] instead of
/// overflowing the stack. A [`crate::Demangler`] budget may raise it up to
/// `HARD_DEPTH`.
pub const MAX_DEPTH: u32 = 500;

/// Default maximum number of nodes a single symbol may produce.
pub const MAX_NODES: usize = 0x0001_0000;

/// Default maximum number of parser work units (production entries, nodes
/// created and backrefs resolved).
pub const MAX_WORK: usize = 200_000;

/// Maximum depth the printer descends into the graph.
pub const MAX_PRINT_DEPTH: u32 = 1024;

/// Maximum number of node visits while printing one symbol.
pub const MAX_PRINT_VISITS: usize = 4_000_000;

/// Maximum number of bytes the printer emits for one symbol.
pub const MAX_OUTPUT_BYTES: usize = 1_000_000;

/// Default maximum accepted input length.
///
/// Every parser allocation is proportional to the input: the v0 offset cache,
/// the legacy element list and the node vectors. Capping the input at 64 KiB
/// keeps those allocations small even when the caller passes untrusted data;
/// real mangled names are far shorter. A [`crate::Demangler`] budget may raise
/// it up to `HARD_INPUT_LEN`, or `HARD_INPUT_LEN_V0` for v0.
pub const MAX_INPUT_LEN: usize = 64 * 1024;

/// Hard input ceiling of the legacy encoding.
///
/// Bounds the offset, element and node vectors a parse of untrusted bytes may
/// allocate, whatever the caller asks for.
const HARD_INPUT_LEN: usize = 16 * 1024 * 1024;

/// Hard input ceiling of the v0 encoding.
///
/// v0 keeps a checkpoint and a cache entry per input byte, so its ceiling is
/// memory-derived and much tighter than `HARD_INPUT_LEN`.
const HARD_INPUT_LEN_V0: usize = 1024 * 1024;

/// Hard parser depth ceiling: bounds the recursion stack of the v0 parser.
const HARD_DEPTH: u32 = 512;

/// Hard parser work ceiling.
const HARD_WORK: usize = 64 * 1024 * 1024;

/// Hard node ceiling: bounds the nodes one parse may allocate.
const HARD_NODES: usize = 1024 * 1024;

/// The limits one parse enforces, resolved from a [`crate::Budget`].
///
/// Every value is already clamped to the hard ceiling of its dimension, so an
/// allocation or a recursive descent stays bounded whatever the caller asks
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParserLimits {
    /// Maximum accepted input length in bytes.
    pub(super) input: usize,
    /// Maximum parser nesting depth.
    pub(super) depth: u32,
    /// Maximum parser work units.
    pub(super) work: usize,
    /// Maximum nodes one parse may allocate.
    pub(super) nodes: usize,
}

impl ParserLimits {
    /// Resolve a scheme budget for a parse of the `v0` encoding.
    ///
    /// An absent dimension keeps its compiled-in default, and every dimension
    /// is clamped to this module's hard ceiling.
    pub(super) fn from_effective(budget: EffectiveBudget, v0: bool) -> Self {
        let input_ceiling = if v0 {
            HARD_INPUT_LEN_V0
        } else {
            HARD_INPUT_LEN
        };
        Self {
            input: clamp(budget.input_bytes, MAX_INPUT_LEN, input_ceiling),
            depth: u32::try_from(clamp(
                budget.parse_depth,
                MAX_DEPTH as usize,
                HARD_DEPTH as usize,
            ))
            .unwrap_or(HARD_DEPTH),
            work: clamp(budget.work, MAX_WORK, HARD_WORK),
            nodes: clamp(budget.nodes, MAX_NODES, HARD_NODES),
        }
    }

    /// Limits handed to the shared byte cursor.
    ///
    /// The cursor enforces the input length and is the only reader of the
    /// symbol bytes. The work, node and depth budgets stay with [`Builder`]
    /// and the parsers' own depth counters, which report them as
    /// [`ParseError`] variants, so the cursor's work and depth counters never
    /// bind.
    pub(super) fn cursor(&self) -> Limits {
        Limits {
            input: u32::try_from(self.input).unwrap_or(u32::MAX),
            depth: u16::try_from(self.depth).unwrap_or(u16::MAX),
            work: u32::MAX,
        }
    }
}

/// One budget dimension after clamping to the compiled-in default and ceiling.
fn clamp(dimension: Option<Limit>, default: usize, ceiling: usize) -> usize {
    match dimension {
        Some(Limit::Bounded(value)) => value.min(ceiling),
        Some(Limit::Unbounded) | None => default.min(ceiling),
    }
}

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
        /// the encoding carried no `K`. Raw bytes, like every other name.
        abi: Option<Cow<'a, [u8]>>,
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
    text: Option<Cow<'a, [u8]>>,
    span: Option<Span>,
    children: Box<[NodeRef<'a>]>,
    data: NodeData<'a>,
    /// Expanded printer visits for this node, counting a shared child once per
    /// reference (saturating).
    print_visits: usize,
    /// Structural printer depth for this node (saturating).
    print_depth: u32,
}

impl<'a> Node<'a> {
    /// The node kind.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The node's text bytes, if it has any.
    ///
    /// Identifiers, basic types and legacy elements borrow the input; text
    /// that had to be reconstructed is owned. The bytes are not guaranteed to
    /// be UTF-8: an identifier copied from a symbol may hold arbitrary bytes,
    /// and the printer escapes them.
    #[must_use]
    pub fn text(&self) -> Option<&[u8]> {
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

    /// Expanded printer visits, counting a shared child once per reference.
    #[must_use]
    pub(crate) const fn print_visits(&self) -> usize {
        self.print_visits
    }

    /// Structural printer depth.
    #[must_use]
    pub(crate) const fn print_depth(&self) -> u32 {
        self.print_depth
    }
}

/// Node factory that charges the node and work budgets.
pub(super) struct Builder {
    nodes: usize,
    work: usize,
    max_nodes: usize,
    max_work: usize,
}

impl<'a> Builder {
    /// A builder with no nodes charged yet and the parse's work and node
    /// ceilings.
    pub(super) const fn new(limits: ParserLimits) -> Self {
        Self {
            nodes: 0,
            work: 0,
            max_nodes: limits.nodes,
            max_work: limits.work,
        }
    }

    /// Charge `work` units, failing when the work budget is exhausted.
    pub(super) fn charge(&mut self, work: usize) -> Result<(), ParseError> {
        self.work = self.work.saturating_add(work);
        if self.work > self.max_work {
            return Err(ParseError::BudgetExceeded);
        }
        Ok(())
    }

    /// Create a node, charging one node and one work unit.
    pub(super) fn node(
        &mut self,
        kind: Kind,
        text: Option<Cow<'a, [u8]>>,
        span: Option<Span>,
        data: NodeData<'a>,
        children: alloc::vec::Vec<NodeRef<'a>>,
    ) -> Result<NodeRef<'a>, ParseError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.max_nodes {
            return Err(ParseError::BudgetExceeded);
        }
        self.charge(1)?;
        let mut print_visits = 1_usize;
        let mut print_depth = 1_u32;
        for child in &children {
            print_visits = print_visits.saturating_add(child.print_visits());
            print_depth = print_depth.max(child.print_depth().saturating_add(1));
        }
        Ok(Rc::new(Node {
            kind,
            text,
            span,
            children: children.into_boxed_slice(),
            data,
            print_visits,
            print_depth,
        }))
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;
    use crate::rust::{DEFAULT_BUDGET, parse};

    #[test]
    fn print_envelope_counts_shared_references() {
        let mut builder = Builder::new(ParserLimits::from_effective(DEFAULT_BUDGET, true));
        let leaf = builder
            .node(Kind::Ident, None, None, NodeData::None, vec![])
            .unwrap();
        let root = builder
            .node(
                Kind::Tuple,
                None,
                None,
                NodeData::None,
                vec![leaf.clone(), leaf],
            )
            .unwrap();
        assert_eq!(root.print_visits(), 3);
        assert_eq!(root.print_depth(), 2);
    }

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
        assert_eq!(ident.text(), Some(&b"bar"[..]));
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

    // At the defaults the node budget fires before the work budget for
    // node-only growth, and the print budgets sit above the parse depth limit.
    // A raised budget can exceed the print envelope, which then rejects the
    // tree: printing is not configurable.
    const _: () = assert!(MAX_NODES < MAX_WORK);
    const _: () = assert!(MAX_DEPTH <= MAX_PRINT_DEPTH);

    #[test]
    fn builder_charges_the_resolved_limits() {
        let limits = ParserLimits {
            input: MAX_INPUT_LEN,
            depth: MAX_DEPTH,
            work: 2,
            nodes: 1,
        };
        let mut builder = Builder::new(limits);
        let _ = builder
            .node(Kind::Ident, None, None, NodeData::None, vec![])
            .expect("first node");
        assert_eq!(
            builder
                .node(Kind::Ident, None, None, NodeData::None, vec![])
                .unwrap_err(),
            ParseError::BudgetExceeded
        );
    }

    #[test]
    fn hard_ceilings_mirror_the_budget_table() {
        use crate::budget::Limit;
        use crate::scheme::Scheme;

        let legacy = Scheme::RustLegacy.hard_budget();
        assert_eq!(legacy.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN)));
        assert_eq!(legacy.parse_depth, None);
        assert_eq!(legacy.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(legacy.nodes, Some(Limit::Bounded(HARD_NODES)));

        let v0 = Scheme::RustV0.hard_budget();
        assert_eq!(v0.input_bytes, Some(Limit::Bounded(HARD_INPUT_LEN_V0)));
        assert_eq!(v0.parse_depth, Some(Limit::Bounded(HARD_DEPTH as usize)));
        assert_eq!(v0.work, Some(Limit::Bounded(HARD_WORK)));
        assert_eq!(v0.nodes, Some(Limit::Bounded(HARD_NODES)));
    }

    #[test]
    fn over_hard_values_are_clamped_to_the_ceilings() {
        let budget = crate::Budget::new()
            .with_input_bytes(usize::MAX)
            .with_parse_depth(u32::MAX)
            .with_work(u32::MAX)
            .with_nodes(u32::MAX);
        let limits = ParserLimits::from_effective(budget.effective(crate::Scheme::RustV0), true);
        assert_eq!(limits.input, HARD_INPUT_LEN_V0);
        assert_eq!(limits.depth, HARD_DEPTH);
        assert_eq!(limits.work, HARD_WORK);
        assert_eq!(limits.nodes, HARD_NODES);
    }

    #[test]
    fn absent_dimensions_keep_the_compiled_in_defaults() {
        for (scheme, v0) in [
            (crate::Scheme::RustLegacy, false),
            (crate::Scheme::RustV0, true),
        ] {
            let limits = ParserLimits::from_effective(crate::Budget::new().effective(scheme), v0);
            assert_eq!(
                limits,
                ParserLimits {
                    input: MAX_INPUT_LEN,
                    depth: MAX_DEPTH,
                    work: MAX_WORK,
                    nodes: MAX_NODES,
                },
                "{scheme}"
            );
        }
    }
}
